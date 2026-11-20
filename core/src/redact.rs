//! Redaction: strips secrets and personal data from log text before anything is
//! stored or shown to an agent. Runs on the collector, so raw secrets never leave
//! the host.

use regex::{Captures, Regex};
use std::sync::LazyLock;

static PRIVATE_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----")
        .expect("valid regex")
});
static JWT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+").expect("valid regex")
});
static BEARER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(bearer|basic)\s+[A-Za-z0-9\-._~+/]+=*").expect("valid regex")
});
static API_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\b(?:(?:sk|pk|rk)_(?:live|test)_[A-Za-z0-9]{10,}|gh[pousr]_[A-Za-z0-9]{20,}|xox[abprs]-[A-Za-z0-9-]{10,}|sk-[A-Za-z0-9_-]{20,}|(?:AKIA|ASIA)[0-9A-Z]{16})\b",
    )
    .expect("valid regex")
});
static KEY_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b(password|passwd|pwd|secret|client[_-]?secret|token|access[_-]?token|refresh[_-]?token|api[_-]?key|apikey|access[_-]?key|private[_-]?key|authorization|auth|session[_-]?id|sessionid|cookie|set-cookie)(["']?\s*[:=]\s*["']?)([^\s"',;&}]+)"#,
    )
    .expect("valid regex")
});
static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b").expect("valid regex")
});
static IPV4: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)\b")
        .expect("valid regex")
});
static CARD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:\d[ -]?){12,18}\d\b").expect("valid regex"));

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

    #[test]
    fn every_pattern_redacts_its_value_and_spares_look_alikes() {
        let r = Redactor::default();
        let key = "-----BEGIN RSA PRIVATE KEY-----\nMIIB\n-----END RSA PRIVATE KEY-----";
        let public = "-----BEGIN PUBLIC KEY-----\nabc\n-----END PUBLIC KEY-----";
        let cases = [
            // Private key blocks, whole; public keys stay.
            (format!("k {key} end"), "k <PRIVATE_KEY> end"),
            (public.to_string(), public),
            // JWT needs three dot-separated parts.
            ("t eyJhbGci.eyJzdWIi.sig_-9 x".into(), "t <JWT> x"),
            ("t eyJhbGci.eyJzdWIi x".into(), "t eyJhbGci.eyJzdWIi x"),
            // Bearer and basic need a credential after them.
            ("Bearer abc.DEF-1_2~+/==".into(), "Bearer <TOKEN>"),
            ("basic dXNlcjpwYXNz".into(), "basic <TOKEN>"),
            ("bearer".into(), "bearer"),
            // API keys by vendor prefix and length.
            ("sk_live_abcdefghij12".into(), "<API_KEY>"),
            ("pk_test_abcdefghij12".into(), "<API_KEY>"),
            ("sk_live_short".into(), "sk_live_short"),
            ("ghp_abcdefghijklmnopqrstuv".into(), "<API_KEY>"),
            ("ghp_tooshort".into(), "ghp_tooshort"),
            ("xoxb-1234567890-abc".into(), "<API_KEY>"),
            ("sk-abcdefghijklmnopqrstuvwx".into(), "<API_KEY>"),
            ("sk-short".into(), "sk-short"),
            ("AKIAIOSFODNN7EXAMPLE".into(), "<API_KEY>"),
            ("AKIAlowercase0123456789".into(), "AKIAlowercase0123456789"),
            // key=value pairs keep the key and separator.
            ("password=hunter2".into(), "password=<REDACTED>"),
            ("PASSWD: hunter2".into(), "PASSWD: <REDACTED>"),
            (
                r#"{"client_secret": "s3cr3t"}"#.into(),
                r#"{"client_secret": "<REDACTED>"}"#,
            ),
            ("apikey='abc' next".into(), "apikey='<REDACTED>' next"),
            ("session_id=42&x=1".into(), "session_id=<REDACTED>&x=1"),
            ("password is required".into(), "password is required"),
            ("password:".into(), "password:"),
            ("tokens=3".into(), "tokens=3"),
            // Emails need a TLD.
            (
                "mail bob.smith+tag@example.co.uk now".into(),
                "mail <EMAIL> now",
            ),
            ("user@host".into(), "user@host"),
            ("@handle".into(), "@handle"),
            // Cards: Luhn-valid, 13-19 digits.
            ("4111-1111-1111-1111".into(), "<CARD>"),
            ("4111111111111111".into(), "<CARD>"),
            ("4111111111111112".into(), "4111111111111112"),
            ("411111111111".into(), "411111111111"),
            // IPv4 addresses, not versions or out-of-range octets.
            ("from 192.168.1.20:8080".into(), "from <IP>:8080"),
            ("from 256.1.1.1".into(), "from 256.1.1.1"),
            ("version 1.2.3".into(), "version 1.2.3"),
            ("version 1.2.3.4.5".into(), "version <IP>.5"),
        ];
        for (input, want) in cases {
            assert_eq!(r.redact(&input), want, "input: {input:?}");
        }
    }

    #[test]
    fn ips_are_redacted_only_when_enabled() {
        let on = Redactor::new(true, vec![]);
        let off = Redactor::new(false, vec![]);
        let line = "peer 10.0.0.12 up";
        assert_eq!(on.redact(line), "peer <IP> up");
        assert_eq!(off.redact(line), line);
        // Other patterns still apply with IPs off.
        assert_eq!(off.redact("10.0.0.12 a@b.io"), "10.0.0.12 <EMAIL>");
    }

    #[test]
    fn extra_patterns_apply_after_the_built_ins() {
        let extra = vec![
            Regex::new(r"acct-\d+").unwrap(),
            Regex::new("SECRET[A-Z]+").unwrap(),
        ];
        let r = Redactor::new(true, extra);
        assert_eq!(
            r.redact("acct-123 and SECRETXYZ and acct-x"),
            "<REDACTED> and <REDACTED> and acct-x"
        );
        // Without them the same text is untouched.
        let line = "acct-123 and SECRETXYZ";
        assert_eq!(Redactor::default().redact(line), line);
    }
}
