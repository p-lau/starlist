//! Pattern merging through the real pipeline and store.

use starlist::collector::Pipeline;
use starlist::pattern::parse_event;
use starlist::redact::Redactor;
use starlist::store::Store;
use std::sync::Arc;

fn id_of(text: &str) -> String {
    parse_event(text, &Redactor::default(), Some(0.0), None).pattern_id
}

#[test]
fn usernames_merge_into_one_pattern_and_old_ids_still_work() {
    let db = std::env::temp_dir().join(format!("starlist-merge-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&db);
    let store = Arc::new(Store::open(&db).unwrap());
    let names = ["alice", "bob", "carol", "dave", "erin", "frank", "grace"];
    {
        let p = Pipeline::new(store.clone(), Redactor::default(), None);
        // Two batches, so the merge happens partway through.
        let lines: Vec<String> = names
            .iter()
            .map(|n| format!("2026-11-06T10:00:00Z WARN password reset requested for {n}"))
            .collect();
        p.ingest(&lines[..3]);
        p.ingest(&lines[3..]);
    }
    let first = id_of("2026-11-06T10:00:00Z WARN password reset requested for alice");
    let row = store.pattern(&first).unwrap().unwrap();
    assert_eq!(row.template, "WARN password reset requested for <*>");
    assert_eq!(row.total, 7);
    // An ID handed out before the merge resolves to the merged pattern.
    let bob = id_of("2026-11-06T10:00:00Z WARN password reset requested for bob");
    assert_eq!(store.pattern(&bob).unwrap().unwrap().id, first);
    assert_eq!(store.pattern_templates().unwrap().len(), 1);

    // After a restart, new names keep landing on the same pattern.
    let p = Pipeline::new(store.clone(), Redactor::default(), None);
    p.ingest(&["2026-11-06T10:01:00Z WARN password reset requested for zoe".to_string()]);
    assert_eq!(store.pattern(&first).unwrap().unwrap().total, 8);
    assert_eq!(store.pattern_templates().unwrap().len(), 1);
}

#[test]
fn merging_can_be_turned_off() {
    let db = std::env::temp_dir().join(format!("starlist-nomerge-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&db);
    let store = Arc::new(Store::open(&db).unwrap());
    let p = Pipeline::with_merging(store.clone(), Redactor::default(), None, None);
    let lines: Vec<String> = ["a", "b", "c", "d", "e", "f"]
        .iter()
        .map(|n| format!("INFO hello {n}x"))
        .collect();
    p.ingest(&lines);
    assert_eq!(store.pattern_templates().unwrap().len(), 6);
}
