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

#[test]
fn split_undoes_a_merge_and_stays_split() {
    let db = std::env::temp_dir().join(format!("starlist-split-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&db);
    let store = Arc::new(Store::open(&db).unwrap());
    let line = |n: &str| format!("2026-11-06T10:00:00Z INFO job {n} finished");
    let names = ["alpha", "beta", "gamma", "delta", "eps"];
    let p = Pipeline::new(store.clone(), Redactor::default(), None);
    p.ingest(&names.iter().map(|n| line(n)).collect::<Vec<_>>());
    let first = id_of(&line("alpha"));
    let beta = id_of(&line("beta"));
    assert_eq!(
        store.pattern(&first).unwrap().unwrap().template,
        "INFO job <*> finished"
    );

    // Split one ID out by naming it: it gets its template back.
    let (row, split) = starlist::issues::split(&store, &beta, None).unwrap();
    assert_eq!(row.id, first);
    assert_eq!(split.len(), 1);
    assert_eq!(split[0].id, beta);
    assert_eq!(split[0].template, "INFO job beta finished");
    assert_eq!(store.merged_ids(&first).unwrap().len(), 3);

    // The running pipeline doesn't fold it back in.
    p.ingest(&[line("beta"), line("zeta")]);
    assert_eq!(store.pattern(&beta).unwrap().unwrap().total, 1);
    assert_eq!(store.pattern(&first).unwrap().unwrap().total, 6);

    // Split the rest: the first pattern gets its own template back.
    let (row, split) = starlist::issues::split(&store, &first, None).unwrap();
    assert_eq!(row.template, "INFO job alpha finished");
    // gamma, delta and eps. zeta only ever matched the merged template, so
    // it was never a pattern of its own.
    assert_eq!(split.len(), 3);
    assert!(store.merged_ids(&first).unwrap().is_empty());
    assert!(starlist::issues::split(&store, &first, None).is_err());

    // After a restart nothing merges again, and each name counts on its own;
    // zeta too, now that the template that covered it is gone.
    let p = Pipeline::new(store.clone(), Redactor::default(), None);
    p.ingest(&names.iter().map(|n| line(n)).collect::<Vec<_>>());
    p.ingest(&[line("zeta")]);
    assert_eq!(store.pattern_templates().unwrap().len(), 6);
    assert_eq!(store.pattern(&beta).unwrap().unwrap().total, 2);

    // A manual merge still works on split patterns.
    starlist::issues::merge(&store, &[first.clone(), beta.clone()]).unwrap();
    assert_eq!(store.pattern(&beta).unwrap().unwrap().id, first);
}
