//! Every bundled adapter runs against its fixtures with the real runtime.
//! Regenerate expected batches after an intended change with:
//! `MIZAN_BLESS=1 cargo test -p mizan-adapters --test fixtures_gate`

use std::path::{Path, PathBuf};

use mizan_adapters::registry::Origin;
use mizan_adapters::{run_adapter, FileInput, Registry};
use mizan_core::fixture::{check_case, discover};

fn adapters_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../adapters")
}

#[test]
fn every_bundled_adapter_matches_its_fixtures() {
    let reg = Registry::load(None);
    assert!(
        reg.errors.is_empty(),
        "bundled adapters failed to load: {:?}",
        reg.errors
    );
    let bless = std::env::var_os("MIZAN_BLESS").is_some();
    let mut failures = Vec::new();
    let mut checked = 0;
    for l in reg.loaded.iter().filter(|l| l.origin == Origin::Bundled) {
        let dir = adapters_root()
            .join(l.location.trim_start_matches("adapters/"))
            .join("fixtures");
        if bless {
            // New cases: an input file without expected.json yet.
            for case_dir in std::fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
            {
                let has_input = std::fs::read_dir(&case_dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .any(|e| e.path().file_stem().is_some_and(|s| s == "input"));
                if has_input && !case_dir.join("expected.json").exists() {
                    std::fs::write(case_dir.join("expected.json"), "{}\n").unwrap();
                }
            }
        }
        let cases = discover(&dir).unwrap_or_default();
        assert!(
            !cases.is_empty(),
            "adapter {} has no fixtures in {}",
            l.adapter.manifest().id,
            dir.display()
        );
        for case in cases {
            let file_name = case
                .input
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string();
            let run = |bytes: &[u8]| -> Result<serde_json::Value, String> {
                let file = FileInput::new(file_name.clone(), bytes.to_vec());
                run_adapter(l.adapter.as_ref(), &file).map_err(|e| e.to_string())
            };
            if bless {
                let out = run(&std::fs::read(&case.input).unwrap()).unwrap();
                let mut text = serde_json::to_string_pretty(&out).unwrap();
                text.push('\n');
                std::fs::write(&case.expected, text).unwrap();
                continue;
            }
            checked += 1;
            if let Err(f) = check_case(&case, |input| run(input.bytes)) {
                failures.push(format!(
                    "{} / {}: {}",
                    l.adapter.manifest().id,
                    case.name,
                    f.reason
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    if !bless {
        assert!(checked > 0, "no fixture cases ran");
    }
}
