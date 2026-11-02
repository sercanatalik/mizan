//! `adapter.toml`: the manifest every adapter has.

use regex::Regex;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AdapterKind {
    Mapping,
    Script,
    Native,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    Csv,
    Xlsx,
    Pdf,
    Xml,
    Ofx,
    Text,
}

impl FileKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            FileKind::Csv => "csv",
            FileKind::Xlsx => "xlsx",
            FileKind::Pdf => "pdf",
            FileKind::Xml => "xml",
            FileKind::Ofx => "ofx",
            FileKind::Text => "text",
        }
    }
}

/// The `[adapter]` section.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: AdapterKind,
    pub files: Vec<FileKind>,
    /// For native adapters: which built-in parser.
    #[serde(default)]
    pub native: Option<String>,
    /// For script adapters: the script file next to the manifest.
    #[serde(default)]
    pub script: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Deserialize)]
struct ManifestFile {
    adapter: Manifest,
}

impl Manifest {
    /// Read the `[adapter]` section of an `adapter.toml` (other sections are ignored here).
    pub fn from_toml(text: &str) -> Result<Manifest, String> {
        let raw: toml::Table = toml::from_str(text).map_err(|e| format!("invalid TOML: {e}"))?;
        let section = raw
            .get("adapter")
            .ok_or("missing [adapter] section")?
            .clone();
        let m: ManifestFile = toml::Table::from_iter([("adapter".to_string(), section)])
            .try_into()
            .map_err(|e: toml::de::Error| format!("invalid [adapter] section: {}", e.message()))?;
        m.adapter.validate()?;
        Ok(m.adapter)
    }

    pub fn validate(&self) -> Result<(), String> {
        let id = Regex::new(r"^[a-z0-9][a-z0-9-]{1,48}$").expect("valid regex");
        if !id.is_match(&self.id) {
            return Err(format!(
                "invalid adapter id {:?}: use 2-49 lower-case letters, digits and dashes",
                self.id
            ));
        }
        let semver = Regex::new(r"^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$").expect("valid regex");
        if !semver.is_match(&self.version) {
            return Err(format!(
                "invalid version {:?}: expected semantic version like 1.0.0",
                self.version
            ));
        }
        if self.name.trim().is_empty() {
            return Err("adapter name must not be empty".into());
        }
        if self.files.is_empty() {
            return Err("adapter must declare at least one file type".into());
        }
        match self.kind {
            AdapterKind::Native if self.native.as_deref().is_none_or(str::is_empty) => {
                Err("native adapters must name their parser in `native`".into())
            }
            _ => Ok(()),
        }
    }

    pub fn label(&self) -> String {
        format!("{} {}", self.id, self.version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_manifest() {
        let m = Manifest::from_toml(
            r#"
[adapter]
id = "ibkr-flex"
version = "1.0.0"
name = "Interactive Brokers Flex Query"
type = "native"
native = "ibkr-flex"
files = ["xml"]

[detect]
anything = "ignored here"
"#,
        )
        .unwrap();
        assert_eq!(
            (m.id.as_str(), m.kind, m.files.clone()),
            ("ibkr-flex", AdapterKind::Native, vec![FileKind::Xml])
        );
        assert_eq!(m.label(), "ibkr-flex 1.0.0");
    }

    #[test]
    fn invalid_manifests() {
        let base = |extra: &str| format!("[adapter]\n{extra}");
        for (toml, why) in [
            (base("id = \"X\"\nversion = \"1.0.0\"\nname = \"n\"\ntype = \"mapping\"\nfiles = [\"csv\"]"), "invalid adapter id"),
            (base("id = \"ok\"\nversion = \"1.0\"\nname = \"n\"\ntype = \"mapping\"\nfiles = [\"csv\"]"), "invalid version"),
            (base("id = \"ok\"\nversion = \"1.0.0\"\nname = \"n\"\ntype = \"native\"\nfiles = [\"xml\"]"), "native adapters"),
            (base("id = \"ok\"\nversion = \"1.0.0\"\nname = \"n\"\ntype = \"mapping\"\nfiles = []"), "at least one file type"),
            (base("id = \"ok\"\nversion = \"1.0.0\"\nname = \"n\"\ntype = \"wasm\"\nfiles = [\"csv\"]"), "invalid [adapter]"),
            ("[other]\nx = 1".to_string(), "missing [adapter]"),
        ] {
            let err = Manifest::from_toml(&toml).unwrap_err();
            assert!(err.contains(why), "{err} should mention {why}");
        }
    }
}
