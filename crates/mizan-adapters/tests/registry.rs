use mizan_adapters::registry::{Origin, Source};
use mizan_adapters::{detect, Detection, FileInput, Registry};

fn mapping(id: &str, required: &[&str], file_name: Option<&str>) -> String {
    let cols = required
        .iter()
        .map(|c| format!("{c:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let fname = file_name
        .map(|f| format!("file_name = {f:?}\n"))
        .unwrap_or_default();
    format!(
        r#"[adapter]
id = "{id}"
version = "1.0.0"
name = "{id} name"
type = "mapping"
files = ["csv"]

[detect]
required_columns = [{cols}]
{fname}
[account]
hint = {{ const = "A1" }}

[[rules]]
column = "Desc"
matches = "."
kind = "deposit"

[fields]
date = {{ column = "Date" }}
net_cash = {{ amount = {{ column = "Amount" }}, currency = {{ const = "EUR" }} }}
"#
    )
}

fn src(origin: Origin, loc: &str, text: &str) -> Source<'static> {
    Source {
        origin,
        location: loc.into(),
        manifest: Box::leak(text.to_string().into_boxed_str()),
        script: None,
    }
}

#[test]
fn valid_invalid_and_duplicate_adapters() {
    let reg = Registry::from_sources([
        src(
            Origin::Bundled,
            "adapters/bank-a",
            &mapping("bank-a", &["Date", "Desc", "Amount"], None),
        ),
        src(
            Origin::Bundled,
            "adapters/broken",
            "[adapter]\nid = \"Bad Id\"",
        ),
        src(
            Origin::User,
            "user/dup.toml",
            &mapping("bank-a", &["Date"], None),
        ),
        src(
            Origin::User,
            "user/script.toml",
            &mapping("my-script", &["Date"], None)
                .replace("type = \"mapping\"", "type = \"script\""),
        ),
    ]);
    assert_eq!(reg.loaded.len(), 1);
    assert_eq!(reg.loaded[0].adapter.manifest().id, "bank-a");
    let msgs: Vec<(&str, &str)> = reg
        .errors
        .iter()
        .map(|e| (e.location.as_str(), e.message.as_str()))
        .collect();
    assert_eq!(msgs.len(), 3, "{msgs:?}");
    assert!(msgs[0].0 == "adapters/broken");
    assert!(msgs[1].1.contains("\"bank-a\" is already used"), "{msgs:?}");
    assert!(
        msgs[2].1.contains("user adapters must be mappings"),
        "{msgs:?}"
    );
}

#[test]
fn load_reads_user_mappings_from_a_directory() {
    let dir = std::env::temp_dir().join(format!("mizan-user-adapters-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("mine.toml"),
        mapping("my-bank", &["Date", "Desc", "Amount"], None),
    )
    .unwrap();
    std::fs::write(dir.join("notes.txt"), "ignored").unwrap();
    let reg = Registry::load(Some(&dir));
    std::fs::remove_dir_all(&dir).unwrap();
    let mine = reg.get("my-bank").expect("user mapping loaded");
    assert_eq!(mine.origin, Origin::User);
    assert!(
        reg.errors.iter().all(|e| !e.location.contains("mine.toml")),
        "{:?}",
        reg.errors
    );
}

#[test]
fn detection_ranks_matches_and_reports_unrecognized() {
    let reg = Registry::from_sources([
        src(
            Origin::Bundled,
            "a",
            &mapping("generic", &["Date", "Amount"], None),
        ),
        src(
            Origin::Bundled,
            "b",
            &mapping("specific", &["Date", "Desc", "Amount"], Some("(?i)^export")),
        ),
        src(Origin::Bundled, "c", &mapping("other", &["Tarih"], None)),
    ]);
    let csv = FileInput::new(
        "export-2026.csv",
        b"Date;Desc;Amount\n2026-10-01;x;1\n".to_vec(),
    );
    match detect(&reg, &csv) {
        Detection::Matches(m) => {
            let ids: Vec<(&str, u8)> = m.iter().map(|x| (x.id.as_str(), x.score)).collect();
            assert_eq!(ids, vec![("specific", 95), ("generic", 90)]);
        }
        other => panic!("{other:?}"),
    }
    let unknown = FileInput::new("x.csv", b"Foo;Bar\n1;2\n".to_vec());
    assert_eq!(detect(&reg, &unknown), Detection::Unrecognized);
    let batch = FileInput::new(
        "b.json",
        br#"{"schema": "mizan.import/1", "records": []}"#.to_vec(),
    );
    assert_eq!(detect(&reg, &batch), Detection::Batch);
}
