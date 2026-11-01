//! End-to-end review and commit over the fixture batch in `tests/data/e2e`.

use mizan_core::*;
use mizan_import::parse_batch;
use mizan_storage::Store;

fn data(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/data/e2e/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn prepare_with(store: &mut Store, json: &str, policy: CreatePolicy) -> Prepared {
    let parsed = parse_batch(json).unwrap();
    let rb = {
        let mut r = MappingResolver { store, policy };
        resolve_batch(parsed, &mut r).unwrap()
    };
    prepare(store, rb).unwrap()
}

const CREATE: CreatePolicy = CreatePolicy {
    create_accounts: true,
    create_assets: true,
};

#[test]
fn review_report_covers_every_section_and_commit_persists_with_provenance() {
    let mut store = Store::open_in_memory().unwrap();
    let seed = prepare_with(&mut store, &data("seed.json"), CREATE);
    commit(&mut store, &seed, &Decisions::new()).unwrap();
    assert_eq!(store.count("activities").unwrap(), 3);

    // Without permission to create assets, the new instruments are unresolved.
    let p = prepare_with(&mut store, &data("batch.json"), CreatePolicy::default());
    let r = report(&store, &p, &Decisions::new()).unwrap();
    let unresolved: Vec<String> = r
        .unresolved
        .iter()
        .map(|i| match i {
            ReviewItem::UnresolvedInstrument { key, .. } => key.clone().unwrap_or_default(),
            ReviewItem::UnresolvedAccount { hint } => hint.hint.clone(),
        })
        .collect();
    assert_eq!(
        unresolved,
        vec!["isin:DE0007164600"],
        "ASML and VWCE exist from the seed"
    );
    assert!(matches!(
        commit(&mut store, &p, &Decisions::new()),
        Err(CommitError::Unresolved(_))
    ));

    // Allow creating assets and review again.
    let p = prepare_with(&mut store, &data("batch.json"), CREATE);
    let r = report(&store, &p, &Decisions::new()).unwrap();

    // Validation errors: the record with kind "unknown" is left out, with its locator.
    assert_eq!(r.validation_errors.len(), 1);
    assert!(
        r.validation_errors[0].starts_with("record #6 (row 150)"),
        "{}",
        r.validation_errors[0]
    );

    // Adapter diagnostics, with locators.
    assert_eq!(r.adapter_diagnostics.len(), 1);
    assert_eq!(r.adapter_diagnostics[0].locator, "row 87");

    // Duplicates: DIV-1 was already imported (external id).
    assert_eq!(
        r.duplicates
            .iter()
            .map(|d| d.locator.as_str())
            .collect::<Vec<_>>(),
        vec!["row 20"]
    );

    // Confirmation: the ASML buy matches row 188 of the seed file, no external id.
    assert_eq!(r.confirmations.len(), 1);
    assert_eq!(r.confirmations[0].record.locator, "row 12");
    assert_eq!(r.confirmations[0].existing_locator, "row 188");
    assert_eq!(r.confirmations[0].decision, None);

    // New records (undecided confirmation not included).
    let new: Vec<&str> = r.new_records.iter().map(|s| s.locator.as_str()).collect();
    assert_eq!(new, vec!["row 25", "row 131", "row 140", "row 141"]);

    // Consistency warning on the SAP sell: 8 × 214.50 − 3.90 = 1712.10, statement 1702.10.
    assert_eq!(r.consistency_warnings.len(), 1);
    assert_eq!(r.consistency_warnings[0].locator, "row 131");

    // Ordering: ASML sold today in file B, bought the same day in file A.
    assert_eq!(r.ordering_diagnostics.len(), 1);
    assert_eq!(
        r.ordering_diagnostics[0].instrument_key,
        "isin:NL0010273215"
    );

    // Flags.
    assert_eq!(r.flagged_adjustments[0].locator, "row 141");
    assert_eq!(r.basis_unknown[0].locator, "row 140");

    // Assertion fails until the ASML buy is confirmed as a second trade.
    assert!(!r.assertion_results[0].passed);
    let asml = r.assertion_results[0]
        .lines
        .iter()
        .find(|l| l.label == "isin:NL0010273215")
        .unwrap();
    assert_eq!((asml.expected.as_str(), asml.computed.as_str()), ("3", "1"));
    assert_eq!(r.summary.pending_confirmations, 1);
    assert_eq!(r.summary.assertions_failed, 1);

    let decisions = Decisions::from([(0, Decision::New)]);
    let r = report(&store, &p, &decisions).unwrap();
    assert!(
        r.assertion_results[0].passed,
        "{:?}",
        r.assertion_results[0]
    );
    assert_eq!(r.summary.new, 5);
    assert_eq!(r.summary.pending_confirmations, 0);

    // Commit persists exactly the confirmed records, with provenance.
    let res = commit(&mut store, &p, &decisions).unwrap();
    assert_eq!(res.committed, 5);
    assert_eq!(res.skipped_undecided, 0);
    assert_eq!(store.count("activities").unwrap(), 8);
    assert_eq!(store.count("import_batches").unwrap(), 2);
    assert_eq!(store.count("assertion_results").unwrap(), 1);

    let acct = store.account_for_hint("DEG-4417").unwrap().unwrap();
    let stored = store.activities_until(acct, "2026-12-31").unwrap();
    let batch_b: Vec<_> = stored
        .iter()
        .filter(|s| s.batch_id == res.batch_id)
        .collect();
    assert_eq!(batch_b.len(), 5);
    for s in &batch_b {
        assert_eq!(s.record.source.file_sha256.as_str(), "b".repeat(64));
        assert_eq!(s.record.source.adapter, "degiro-csv");
    }
    let vwce_in = batch_b
        .iter()
        .find(|s| s.record.source.locator == "row 140")
        .unwrap();
    assert!(vwce_in.basis_unknown);
    assert!(batch_b
        .iter()
        .all(|s| s.record.source.locator != "row 20" && s.record.source.locator != "row 150"));
}
