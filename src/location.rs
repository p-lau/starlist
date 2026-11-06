//! Where in the app's own code an event came from: the top app frame of a
//! stack trace as `file:line in function`, for Python, Java/Kotlin,
//! Node/TypeScript, Go and Rust panics.
//!
//! "Top" means closest to the failure: the last app frame of a Python
//! traceback (most recent call last), the first app frame everywhere else.
//! Frames from the standard library and third-party packages are skipped.

use regex::Regex;
use std::sync::LazyLock;

static PY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*File "([^"]+)", line (\d+)(?:, in (\S+))?"#).unwrap());
static JAVA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*at\s+([\w$.<>/]+)\(([\w$-]+\.(?:java|kt|kts|scala|groovy|clj)):(\d+)\)")
        .unwrap()
});
static NODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*at\s+(?:async\s+)?(?:(.+?)\s+\()?(?:file://)?([^\s()]+?):(\d+):\d+\)?\s*$")
        .unwrap()
});
static GO_FILE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s+(\S+\.go):(\d+)(?:\s|$)").unwrap());
static GO_FUNC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([\w./*()$\[\]-]+)\(").unwrap());
static RUST_PANIC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"panicked at (?:'.*?', |".*?", )?([^\s:'"]+\.rs):(\d+):\d+"#).unwrap()
});
static RUST_AT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s+at\s+(\S+\.rs):(\d+)(?::\d+)?\s*$").unwrap());
static RUST_FN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\d+:\s+(\S+)").unwrap());
static RUST_HASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"::h[0-9a-f]{16}$").unwrap());

/// Paths and function prefixes that belong to runtimes and third-party code.
const LIB_PATH: &[&str] = &[
    "site-packages",
    "dist-packages",
    "/lib/python",
    "\\lib\\python",
    "<frozen",
    "node_modules",
    "node:",
    "/usr/local/go/src/",
    "/go/pkg/mod/",
    "/usr/lib/go",
    "/rustc/",
    ".cargo/registry",
    ".cargo/git",
    "/library/std/",
    "/library/core/",
    "/library/alloc/",
];
const LIB_FUNC: &[&str] = &[
    "java.",
    "javax.",
    "jdk.",
    "sun.",
    "kotlin.",
    "kotlinx.",
    "scala.",
    "org.springframework.",
    "org.apache.",
    "io.netty.",
    "runtime.",
    "std::",
    "core::",
    "alloc::",
    "tokio::",
    "rust_begin_unwind",
    "__rust",
];

/// Is this a library frame (by file path or function name)?
pub fn is_library(file: &str, function: Option<&str>) -> bool {
    LIB_PATH.iter().any(|m| file.contains(m))
        || function.is_some_and(|f| LIB_FUNC.iter().any(|m| f.starts_with(m)))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub file: String,
    pub line: u32,
    pub function: Option<String>,
}

impl Frame {
    fn new(file: &str, line: &str, function: Option<&str>) -> Option<Self> {
        Some(Self {
            file: file.trim_start_matches("file://").to_string(),
            line: line.parse().ok()?,
            function: function
                .map(|f| RUST_HASH.replace(f.trim(), "").into_owned())
                .filter(|f| !f.is_empty() && f != "<module>" && f != "<anonymous>"),
        })
    }

    fn is_app(&self) -> bool {
        !is_library(&self.file, self.function.as_deref())
    }

    /// `file:line in function`, with the first matching root stripped from the path.
    pub fn display(&self, roots: &[String]) -> String {
        let file = strip_root(&self.file, roots);
        match &self.function {
            Some(f) => format!("{file}:{} in {f}", self.line),
            None => format!("{file}:{}", self.line),
        }
    }
}

/// Make a path relative to the repository: drop the first matching root and a leading `./`.
pub fn strip_root<'a>(path: &'a str, roots: &[String]) -> &'a str {
    let mut p = path;
    for r in roots {
        let r = r.trim_end_matches(['/', '\\']);
        if !r.is_empty()
            && let Some(rest) = p.strip_prefix(r)
            && (rest.starts_with('/') || rest.starts_with('\\'))
        {
            p = &rest[1..];
            break;
        }
    }
    p.strip_prefix("./").unwrap_or(p)
}

/// The top app frame in an event's text, if it has a stack trace.
pub fn app_frame(text: &str) -> Option<Frame> {
    if !text.contains('\n') && !text.contains("panicked at") {
        return None;
    }
    let lines: Vec<&str> = text.lines().collect();
    let mut python: Vec<Frame> = vec![];
    let mut others: Vec<Frame> = vec![];

    // A Rust panic names its own location first.
    if let Some(c) = lines.iter().find_map(|l| RUST_PANIC.captures(l)) {
        others.extend(Frame::new(&c[1], &c[2], None));
    }
    for (i, l) in lines.iter().enumerate() {
        if let Some(c) = PY.captures(l) {
            python.extend(Frame::new(&c[1], &c[2], c.get(3).map(|m| m.as_str())));
        } else if let Some(c) = JAVA.captures(l) {
            others.extend(Frame::new(&c[2], &c[3], Some(&c[1])));
        } else if let Some(c) = RUST_AT.captures(l) {
            let func = i
                .checked_sub(1)
                .and_then(|j| RUST_FN.captures(lines[j]))
                .map(|f| f[1].to_string());
            others.extend(Frame::new(&c[1], &c[2], func.as_deref()));
        } else if let Some(c) = NODE.captures(l) {
            others.extend(Frame::new(&c[2], &c[3], c.get(1).map(|m| m.as_str())));
        } else if let Some(c) = GO_FILE.captures(l) {
            let func = i
                .checked_sub(1)
                .and_then(|j| GO_FUNC.captures(lines[j]))
                .map(|f| f[1].to_string());
            others.extend(Frame::new(&c[1], &c[2], func.as_deref()));
        }
    }
    if !python.is_empty() {
        return python.into_iter().rev().find(Frame::is_app);
    }
    others.into_iter().find(Frame::is_app)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(text: &str) -> Option<String> {
        app_frame(text).map(|f| f.display(&["/app".to_string()]))
    }

    #[test]
    fn python_takes_the_last_app_frame() {
        let t = "ERROR checkout failed\nTraceback (most recent call last):\n  File \"/app/views.py\", line 12, in checkout\n    total = price_cart(cart)\n  File \"/app/pricing.py\", line 41, in price_cart\n    rate = RATES[cart.currency]\n  File \"/usr/lib/python3.12/site-packages/x/y.py\", line 3, in get\nKeyError: 'currency'";
        assert_eq!(loc(t).unwrap(), "pricing.py:41 in price_cart");
    }

    #[test]
    fn java_and_kotlin_take_the_first_app_frame() {
        let t = "ERROR boom\njava.lang.IllegalStateException: no rate\n\tat java.base/java.util.HashMap.get(HashMap.java:556)\n\tat com.shop.Pricing.price(Pricing.java:41)\n\tat com.shop.Api.checkout(Api.kt:12)";
        assert_eq!(loc(t).unwrap(), "Pricing.java:41 in com.shop.Pricing.price");
    }

    #[test]
    fn node_skips_node_modules_and_internals() {
        let t = "Error: no rate\n    at Object.get (/app/node_modules/lib/index.js:5:9)\n    at priceCart (/app/src/pricing.ts:41:13)\n    at async checkout (file:///app/src/api.ts:12:3)\n    at process.processTicksAndRejections (node:internal/process/task_queues:95:5)";
        assert_eq!(loc(t).unwrap(), "src/pricing.ts:41 in priceCart");
        let anon = "TypeError: x\n    at /app/src/api.ts:7:1";
        assert_eq!(loc(anon).unwrap(), "src/api.ts:7");
    }

    #[test]
    fn go_panics() {
        let t = "panic: runtime error: index out of range\n\ngoroutine 1 [running]:\nruntime.panicmem()\n\t/usr/local/go/src/runtime/panic.go:261 +0x1d\nmain.priceCart(0x0)\n\t/app/pricing.go:41 +0x1d\nmain.main()\n\t/app/main.go:9 +0x25";
        assert_eq!(loc(t).unwrap(), "pricing.go:41 in main.priceCart");
        // Go apps often keep code under internal/.
        let t = "panic: boom\n\ngoroutine 1 [running]:\nshop/internal/pricing.Cart(...)\n\t/app/internal/pricing/cart.go:41 +0x1d";
        assert_eq!(
            loc(t).unwrap(),
            "internal/pricing/cart.go:41 in shop/internal/pricing.Cart"
        );
    }

    #[test]
    fn rust_panics() {
        let t = "thread 'main' panicked at src/pricing.rs:41:5:\nno rate for currency";
        assert_eq!(loc(t).unwrap(), "src/pricing.rs:41");
        let old = "thread 'main' panicked at 'no rate', src/pricing.rs:41:5";
        assert_eq!(loc(old).unwrap(), "src/pricing.rs:41");
        let bt = "thread 'main' panicked at /rustc/abc/library/core/src/option.rs:2:3:\nboom\nstack backtrace:\n   0: rust_begin_unwind\n             at /rustc/abc/library/std/src/panicking.rs:665:5\n   5: shop::pricing::price_cart::h0123456789abcdef\n             at ./src/pricing.rs:41:5";
        assert_eq!(
            loc(bt).unwrap(),
            "src/pricing.rs:41 in shop::pricing::price_cart"
        );
    }

    #[test]
    fn plain_lines_have_no_location() {
        assert!(loc("INFO GET /api/items 200 in 12ms").is_none());
        assert!(loc("ERROR db timeout\nretrying in 5s").is_none());
    }

    #[test]
    fn roots_only_strip_whole_path_segments() {
        let roots = vec!["/app".to_string(), "C:\\srv\\shop\\".to_string()];
        assert_eq!(strip_root("/app/x.py", &roots), "x.py");
        assert_eq!(strip_root("/apple/x.py", &roots), "/apple/x.py");
        assert_eq!(strip_root("C:\\srv\\shop\\x.py", &roots), "x.py");
        assert_eq!(strip_root("./src/a.rs", &[]), "src/a.rs");
    }
}
