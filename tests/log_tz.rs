use starlist::collector::Pipeline;
use starlist::redact::Redactor;
use starlist::store::Store;
use std::sync::Arc;

#[test]
fn log_tz_shifts_only_zoneless_timestamps() {
    let db = std::env::temp_dir().join(format!("starlist-tz-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&db);
    let store = Arc::new(Store::open(&db).unwrap());
    let p = Pipeline::with_merging(store.clone(), Redactor::default(), None, None)
        .with_log_tz(2 * 3600);
    let naive = p.parse("2026-11-06 12:00:00 INFO local time", 0.0);
    let zoned = p.parse("2026-11-06T12:00:00Z INFO utc time", 0.0);
    // 12:00 at UTC+2 is 10:00 UTC, two hours before the zoned 12:00Z.
    assert_eq!(zoned.ts.unwrap() - naive.ts.unwrap(), 7200.0);
}
