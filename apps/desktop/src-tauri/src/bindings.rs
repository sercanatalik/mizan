//! TypeScript bindings for the desktop-only DTOs, written next to the core bindings.
//! Regenerate with: `MIZAN_BLESS=1 cargo test -p mizan-desktop bindings`

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ts_rs::{Config, TS};

fn committed_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/bridge/generated")
}

fn read_ts(dir: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "ts") {
            out.insert(
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(&p).unwrap(),
            );
        }
    }
    out
}

#[test]
fn committed_desktop_bindings_match_generated() {
    let tmp = std::env::temp_dir().join(format!("mizan-desktop-bindings-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let cfg = Config::new().with_out_dir(&tmp).with_large_int("number");
    crate::session::ImportView::export_all(&cfg).unwrap();
    crate::session::CommitView::export_all(&cfg).unwrap();
    crate::commands::BatchView::export_all(&cfg).unwrap();
    crate::commands::AccountView::export_all(&cfg).unwrap();
    crate::market::SettingsView::export_all(&cfg).unwrap();
    crate::market::ProviderView::export_all(&cfg).unwrap();
    mizan_core::assets::AssetView::export_all(&cfg).unwrap();
    mizan_market::RefreshReport::export_all(&cfg).unwrap();
    let generated = read_ts(&tmp);
    std::fs::remove_dir_all(&tmp).unwrap();

    if std::env::var_os("MIZAN_BLESS").is_some() {
        for (name, body) in &generated {
            std::fs::write(committed_dir().join(name), body).unwrap();
        }
        return;
    }
    let committed = read_ts(&committed_dir());
    let drifted: Vec<&String> = generated
        .iter()
        .filter(|(n, b)| committed.get(*n) != Some(*b))
        .map(|(n, _)| n)
        .collect();
    assert!(
        drifted.is_empty(),
        "TypeScript bindings are out of date: {drifted:?}\nRegenerate with:\n  MIZAN_BLESS=1 cargo test -p mizan-desktop bindings"
    );
}
