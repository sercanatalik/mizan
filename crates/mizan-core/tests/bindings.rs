//! TypeScript bindings for the desktop app, generated from the Rust types.
//! Fails when the committed files in `apps/desktop/src/bridge/generated/` drift.
//! Regenerate with: `MIZAN_BLESS=1 cargo test -p mizan-core --features ts --test bindings`
#![cfg(feature = "ts")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ts_rs::{Config, TS};

fn committed_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/src/bridge/generated")
}

fn read_dir(dir: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "ts") {
                out.insert(
                    p.file_name().unwrap().to_string_lossy().into_owned(),
                    std::fs::read_to_string(&p).unwrap(),
                );
            }
        }
    }
    out
}

fn generate(dir: &Path) {
    let cfg = Config::new().with_out_dir(dir).with_large_int("number");
    mizan_core::ReviewReport::export_all(&cfg).unwrap();
    mizan_core::Decision::export_all(&cfg).unwrap();
    mizan_core::portfolio::Holdings::export_all(&cfg).unwrap();
    mizan_core::portfolio::Overview::export_all(&cfg).unwrap();
    mizan_core::portfolio::Contributions::export_all(&cfg).unwrap();
    mizan_core::returns::ReturnsView::export_all(&cfg).unwrap();
    mizan_core::returns::ValueHistory::export_all(&cfg).unwrap();
    mizan_core::tax::report::TaxReport::export_all(&cfg).unwrap();
    mizan_core::tax::report::TaxYears::export_all(&cfg).unwrap();
    mizan_core::activity::ActivityPage::export_all(&cfg).unwrap();
    mizan_core::activity::ActivityDetail::export_all(&cfg).unwrap();
    mizan_core::activity::ActivityQuery::export_all(&cfg).unwrap();
    mizan_core::activity::ActivityForm::export_all(&cfg).unwrap();
    mizan_core::activity::FormCheck::export_all(&cfg).unwrap();
    mizan_core::market_data::SeriesPage::export_all(&cfg).unwrap();
    mizan_core::market_data::SeriesDetail::export_all(&cfg).unwrap();
    mizan_core::market_data::SeriesFilter::export_all(&cfg).unwrap();
    mizan_core::accounts::AccountsList::export_all(&cfg).unwrap();
    mizan_core::accounts::AccountDetail::export_all(&cfg).unwrap();
    mizan_core::accounts::CurrentUnitsChart::export_all(&cfg).unwrap();
    mizan_core::income::IncomeView::export_all(&cfg).unwrap();
}

#[test]
fn committed_bindings_match_generated() {
    let tmp = std::env::temp_dir().join(format!("mizan-bindings-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    generate(&tmp);
    let generated = read_dir(&tmp);
    std::fs::remove_dir_all(&tmp).unwrap();
    assert!(
        generated.contains_key("ReviewReport.ts"),
        "{:?}",
        generated.keys()
    );

    if std::env::var_os("MIZAN_BLESS").is_some() {
        std::fs::create_dir_all(committed_dir()).unwrap();
        for (name, body) in &generated {
            std::fs::write(committed_dir().join(name), body).unwrap();
        }
        return;
    }
    let committed = read_dir(&committed_dir());
    let drifted: Vec<&String> = generated
        .iter()
        .filter(|(n, b)| committed.get(*n) != Some(*b))
        .map(|(n, _)| n)
        .collect();
    assert!(
        drifted.is_empty(),
        "TypeScript bindings are out of date: {drifted:?}\nRegenerate with:\n  MIZAN_BLESS=1 cargo test -p mizan-core --features ts --test bindings"
    );
}
