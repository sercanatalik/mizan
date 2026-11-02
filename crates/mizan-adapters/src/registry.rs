//! All loaded adapters: bundled ones (embedded at build time) and user mappings.

use std::path::Path;
use std::sync::Arc;

use crate::manifest::{AdapterKind, Manifest};
use crate::mapping::MappingAdapter;
use crate::Adapter;

/// A bundled adapter's files, embedded by `build.rs`.
pub struct Bundled {
    pub dir: &'static str,
    pub manifest: &'static str,
    pub script: Option<&'static str>,
}

include!(concat!(env!("OUT_DIR"), "/bundled.rs"));

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Bundled,
    User,
}

/// An adapter definition before it is built.
pub struct Source<'a> {
    pub origin: Origin,
    /// Where it came from, for messages (directory or file name).
    pub location: String,
    pub manifest: &'a str,
    pub script: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadError {
    pub location: String,
    pub message: String,
}

pub struct Loaded {
    pub adapter: Arc<dyn Adapter>,
    pub origin: Origin,
    pub location: String,
}

#[derive(Default)]
pub struct Registry {
    pub loaded: Vec<Loaded>,
    pub errors: Vec<LoadError>,
}

fn build(manifest: Manifest, text: &str, script: Option<&str>) -> Result<Arc<dyn Adapter>, String> {
    Ok(match manifest.kind {
        AdapterKind::Mapping => Arc::new(MappingAdapter::from_toml(manifest, text)?),
        AdapterKind::Script => crate::script::build(
            manifest,
            script.ok_or("script adapters need a script file")?,
        )?,
        AdapterKind::Native => crate::native::build(manifest)?,
    })
}

impl Registry {
    /// Build adapters in order; later sources may not reuse an earlier id.
    pub fn from_sources<'a>(sources: impl IntoIterator<Item = Source<'a>>) -> Registry {
        let mut reg = Registry::default();
        for s in sources {
            let fail = |reg: &mut Registry, message: String| {
                reg.errors.push(LoadError {
                    location: s.location.clone(),
                    message,
                })
            };
            let manifest = match Manifest::from_toml(s.manifest) {
                Ok(m) => m,
                Err(e) => {
                    fail(&mut reg, e);
                    continue;
                }
            };
            if s.origin == Origin::User && manifest.kind != AdapterKind::Mapping {
                fail(
                    &mut reg,
                    format!("user adapters must be mappings, not {:?}", manifest.kind),
                );
                continue;
            }
            if reg.get(&manifest.id).is_some() {
                fail(
                    &mut reg,
                    format!(
                        "adapter id {:?} is already used by another adapter",
                        manifest.id
                    ),
                );
                continue;
            }
            match build(manifest, s.manifest, s.script) {
                Ok(adapter) => reg.loaded.push(Loaded {
                    adapter,
                    origin: s.origin,
                    location: s.location,
                }),
                Err(e) => fail(&mut reg, e),
            }
        }
        reg
    }

    /// Bundled adapters, then every `*.toml` mapping in `user_dir` (if it exists).
    pub fn load(user_dir: Option<&Path>) -> Registry {
        let mut user: Vec<(String, String)> = Vec::new();
        if let Some(dir) = user_dir {
            let mut files: Vec<_> = std::fs::read_dir(dir)
                .map(|r| {
                    r.flatten()
                        .map(|e| e.path())
                        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
                        .collect()
                })
                .unwrap_or_default();
            files.sort();
            for f in files {
                if let Ok(text) = std::fs::read_to_string(&f) {
                    user.push((f.display().to_string(), text));
                }
            }
        }
        let bundled = BUNDLED.iter().map(|b| Source {
            origin: Origin::Bundled,
            location: format!("adapters/{}", b.dir),
            manifest: b.manifest,
            script: b.script,
        });
        let user = user.iter().map(|(loc, text)| Source {
            origin: Origin::User,
            location: loc.clone(),
            manifest: text,
            script: None,
        });
        Registry::from_sources(bundled.chain(user))
    }

    pub fn get(&self, id: &str) -> Option<&Loaded> {
        self.loaded.iter().find(|l| l.adapter.manifest().id == id)
    }
}
