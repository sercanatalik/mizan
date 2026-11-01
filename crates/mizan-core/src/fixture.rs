//! Adapter fixture harness.
//!
//! Layout, one directory per case:
//!
//! ```text
//! fixtures/<adapter-id>/<case>/
//!     input.<ext>      anonymized source file (csv, xlsx, pdf, xml, ...)
//!     expected.json    the exact mizan.import/1 batch the adapter must produce
//! ```
//!
//! The adapter receives the input bytes and their SHA-256. Its output must be a
//! clean batch and must equal `expected.json` exactly; otherwise the failure
//! carries a line diff.

use std::fmt;
use std::path::{Path, PathBuf};

use mizan_import::parse_batch_value;
use serde_json::Value;
use sha2::{Digest, Sha256};
use similar::TextDiff;

/// What an adapter gets to work with.
pub struct AdapterInput<'a> {
    pub bytes: &'a [u8],
    /// Lower-case hex SHA-256 of `bytes`, for provenance.
    pub file_sha256: String,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureCase {
    /// `<adapter-id>/<case>` or just `<case>`, relative to the discovered root.
    pub name: String,
    pub input: PathBuf,
    pub expected: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureFailure {
    pub case: String,
    pub reason: String,
}

impl fmt::Display for FixtureFailure {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "fixture {} failed:\n{}", self.case, self.reason)
    }
}

/// Find every case under `root`: any directory holding `expected.json` and one `input.*` file.
pub fn discover(root: &Path) -> std::io::Result<Vec<FixtureCase>> {
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<FixtureCase>) -> std::io::Result<()> {
    let mut input = None;
    let mut has_expected = false;
    let mut subdirs = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            subdirs.push(path);
        } else if path.file_name().is_some_and(|n| n == "expected.json") {
            has_expected = true;
        } else if path.file_stem().is_some_and(|s| s == "input") {
            input = Some(path);
        }
    }
    if let (true, Some(input)) = (has_expected, input) {
        let name = dir
            .strip_prefix(root)
            .unwrap_or(dir)
            .to_string_lossy()
            .replace('\\', "/");
        out.push(FixtureCase {
            name,
            input,
            expected: dir.join("expected.json"),
        });
    }
    for d in subdirs {
        walk(root, &d, out)?;
    }
    Ok(())
}

fn pretty(v: &Value) -> String {
    let mut s = serde_json::to_string_pretty(v).expect("JSON value serializes");
    s.push('\n');
    s
}

/// Run `adapter` on one case and compare its output to the expected batch.
pub fn check_case<F>(case: &FixtureCase, adapter: F) -> Result<(), FixtureFailure>
where
    F: Fn(&AdapterInput) -> Result<Value, String>,
{
    let fail = |reason: String| FixtureFailure {
        case: case.name.clone(),
        reason,
    };
    let bytes = std::fs::read(&case.input).map_err(|e| fail(format!("cannot read input: {e}")))?;
    let expected_text = std::fs::read_to_string(&case.expected)
        .map_err(|e| fail(format!("cannot read expected.json: {e}")))?;
    let expected: Value = serde_json::from_str(&expected_text)
        .map_err(|e| fail(format!("expected.json is not JSON: {e}")))?;

    let input = AdapterInput {
        bytes: &bytes,
        file_sha256: sha256_hex(&bytes),
    };
    let actual = adapter(&input).map_err(|e| fail(format!("adapter error: {e}")))?;

    match parse_batch_value(actual.clone()) {
        Err(e) => return Err(fail(format!("adapter output is not a valid batch: {e}"))),
        Ok(p) if !p.is_clean() => {
            let errs: Vec<String> = p.errors.iter().map(|e| format!("  {e}")).collect();
            return Err(fail(format!(
                "adapter output has invalid items:\n{}",
                errs.join("\n")
            )));
        }
        Ok(_) => {}
    }

    let (want, got) = (pretty(&expected), pretty(&actual));
    if want == got {
        return Ok(());
    }
    let diff = TextDiff::from_lines(&want, &got)
        .unified_diff()
        .context_radius(3)
        .header("expected.json", "adapter output")
        .to_string();
    Err(fail(diff))
}

/// Check every case under `root`. Returns the number of cases checked, or all failures.
pub fn check_all<F>(root: &Path, adapter: F) -> Result<usize, Vec<FixtureFailure>>
where
    F: Fn(&AdapterInput) -> Result<Value, String>,
{
    let cases = discover(root).map_err(|e| {
        vec![FixtureFailure {
            case: root.display().to_string(),
            reason: format!("cannot list fixtures: {e}"),
        }]
    })?;
    let failures: Vec<FixtureFailure> = cases
        .iter()
        .filter_map(|c| check_case(c, &adapter).err())
        .collect();
    if failures.is_empty() {
        Ok(cases.len())
    } else {
        Err(failures)
    }
}
