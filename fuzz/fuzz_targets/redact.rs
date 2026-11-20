#![no_main]

use libfuzzer_sys::fuzz_target;
use starlist_core::redact::Redactor;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    // Only checks that redaction never panics: it is a chain of regex passes,
    // so neither idempotence nor "no secret left" is promised.
    let _ = Redactor::default().redact(&text);
    let _ = Redactor::new(false, vec![]).redact(&text);
});
