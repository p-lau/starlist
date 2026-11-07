//! Redaction: strips secrets and personal data from log text before anything is
//! stored or shown to an agent. Runs on the collector, so raw secrets never leave
//! the host.

use regex::{Captures, Regex};
use std::sync::LazyLock;

static PRIVATE_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----")
        .unwrap()
});
static JWT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+").unwrap());
static BEARER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(bearer|basic)\s+[A-Za-z0-9\-._~+/]+=*").unwrap());
static API_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\b(?:(?:sk|pk|rk)_(?:live|test)_[A-Za-z0-9]{10,}|gh[pousr]_[A-Za-z0-9]{20,}|xox[abprs]-[A-Za-z0-9-]{10,}|sk-[A-Za-z0-9_-]{20,}|(?:AKIA|ASIA)[0-9A-Z]{16})\b",
    )
    .unwrap()
});
static KEY_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b(password|passwd|pwd|secret|client[_-]?secret|token|access[_-]?token|refresh[_-]?token|api[_-]?key|apikey|access[_-]?key|private[_-]?key|authorization|auth|session[_-]?id|sessionid|cookie|set-cookie)(["']?\s*[:=]\s*["']?)([^\s"',;&}]+)"#,
    )
    .unwrap()
});
static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b").unwrap());
static IPV4: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)\b").unwrap()
});
static CARD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(?:\d[ -]?){12,18}\d\b").unwrap());

fn luhn_valid(digits: &[u32]) -> bool {
    let mut sum = 0;
    for (i, d) in digits.iter().rev().enumerate() {
        let mut v = *d;
        if i % 2 == 1 {
            v *= 2;
            if v > 9 {
                v -= 9;
            }
        }
        sum += v;
    }
    sum % 10 == 0
}

#[derive(Clone)]
pub struct Redactor {
    redact_ips: bool,
    extra: Vec<Regex>,
}

impl Default for Redactor {
    fn default() -> Self {
        Self {
            redact_ips: true,
            extra: vec![],
        }
    }
}

impl Redactor {
    pub fn new(redact_ips: bool, extra: Vec<Regex>) -> Self {
        Self { redact_ips, extra }
    }

    pub fn redact(&self, text: &str) -> String {
        let mut s = PRIVATE_KEY.replace_all(text, "<PRIVATE_KEY>").into_owned();
        s = JWT.replace_all(&s, "<JWT>").into_owned();
        s = BEARER.replace_all(&s, "$1 <TOKEN>").into_owned();
        s = API_KEY.replace_all(&s, "<API_KEY>").into_owned();
        s = KEY_VALUE.replace_all(&s, "${1}${2}<REDACTED>").into_owned();
        s = EMAIL.replace_all(&s, "<EMAIL>").into_owned();
        s = CARD
            .replace_all(&s, |c: &Captures| {
                let m = &c[0];
                let digits: Vec<u32> = m.chars().filter_map(|ch| ch.to_digit(10)).collect();
                if (13..=19).contains(&digits.len()) && luhn_valid(&digits) {
                    "<CARD>".to_string()
                } else {
                    m.to_string()
                }
            })
            .into_owned();
        if self.redact_ips {
            s = IPV4.replace_all(&s, "<IP>").into_owned();
        }
        for re in &self.extra {
            s = re.replace_all(&s, "<REDACTED>").into_owned();
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_common_secrets() {
        let r = Redactor::default();
        let out = r.redact(
            "login ok user=bob@example.com password=hunter2 token: abc123 Authorization: Bearer eyJabc.def.ghi from 10.0.0.12",
        );
        assert!(!out.contains("bob@example.com"), "{out}");
        assert!(!out.contains("hunter2"), "{out}");
        assert!(!out.contains("abc123"), "{out}");
        assert!(!out.contains("eyJabc"), "{out}");
        assert!(!out.contains("10.0.0.12"), "{out}");
        assert!(out.contains("<EMAIL>") && out.contains("<IP>"), "{out}");
    }

    #[test]
    fn redacts_json_style_and_api_keys() {
        let r = Redactor::default();
        let out = r.redact(
            r#"{"api_key": "sk-proj-AAAAAAAAAAAAAAAAAAAAAAAA", "aws": "AKIAIOSFODNN7EXAMPLE"}"#,
        );
        assert!(!out.contains("sk-proj"), "{out}");
        assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"), "{out}");
    }

    #[test]
    fn cards_only_when_luhn_valid() {
        let r = Redactor::default();
        assert!(
            r.redact("card 4111 1111 1111 1111 charged")
                .contains("<CARD>")
        );
        // An order number that is not a valid card number stays.
        assert!(
            r.redact("order 1234567890123 shipped")
                .contains("1234567890123")
        );
    }

    #[test]
    fn leaves_ordinary_words_alone() {
        let r = Redactor::default();
        let line = "token expired for session, retrying";
        assert_eq!(r.redact(line), line);
    }
}
