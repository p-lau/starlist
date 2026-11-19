#![no_main]

use libfuzzer_sys::fuzz_target;
use starlist_core::pattern::{parse_event, parse_event_with};
use starlist_core::redact::Redactor;
use starlist_core::request::RequestKeys;

fuzz_target!(|data: &[u8]| {
    // The first byte picks the hints; the rest is the log text.
    let (flags, rest) = data.split_first().map_or((0, data), |(f, r)| (*f, r));
    let text = String::from_utf8_lossy(rest);
    let ts_hint = (flags & 1 != 0).then_some(1_700_000_000.0);
    let level_hint = match (flags >> 1) & 3 {
        0 => None,
        1 => Some("error"),
        2 => Some("warn"),
        _ => Some("not-a-level"),
    };
    let redactor = Redactor::default();

    let a = parse_event(&text, &redactor, ts_hint, level_hint);
    let b = parse_event(&text, &redactor, ts_hint, level_hint);
    assert_eq!(a.pattern_id, b.pattern_id);
    assert_eq!(a.template, b.template);

    let keys = RequestKeys::new(&["order_ref".to_string()]);
    let _ = parse_event_with(&text, &redactor, ts_hint, level_hint, &keys);
});
