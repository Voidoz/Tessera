//! Persistent, per-output icon positions.
//!
//! Only manually placed items are stored.  Everything else is auto-arranged
//! around the pinned items, which keeps the desktop deterministic when files
//! are added or removed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use log::warn;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    pub output: String,
    pub column: u32,
    pub row: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct StateFile {
    version: u32,
    #[serde(default)]
    items: Vec<StoredItem>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredItem {
    path: String,
    output: String,
    column: u32,
    row: u32,
}

impl StateFile {
    fn into_placements(self) -> HashMap<PathBuf, Placement> {
        self.items
            .into_iter()
            .map(|item| {
                (
                    PathBuf::from(item.path),
                    Placement {
                        output: item.output,
                        column: item.column,
                        row: item.row,
                    },
                )
            })
            .collect()
    }
}

#[derive(Debug)]
pub struct StateStore {
    path: PathBuf,
    placements: HashMap<PathBuf, Placement>,
}

impl StateStore {
    pub fn load(path: PathBuf) -> Self {
        let placements = match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<StateFile>(&text) {
                Ok(state) => {
                    if state.version > 1 {
                        warn!(
                            "state file {} was written by a newer Tessera; ignoring it",
                            path.display()
                        );
                        HashMap::new()
                    } else {
                        state.into_placements()
                    }
                }
                Err(error) => {
                    warn!("invalid icon state in {}: {error}", path.display());
                    HashMap::new()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(error) => {
                warn!("cannot read icon state {}: {error}", path.display());
                HashMap::new()
            }
        };

        Self { path, placements }
    }

    pub fn placement(&self, path: &Path) -> Option<&Placement> {
        self.placements.get(path)
    }

    pub fn set(&mut self, path: PathBuf, placement: Placement) {
        self.placements.insert(path, placement);
    }

    pub fn remove(&mut self, path: &Path) {
        self.placements.remove(path);
    }

    pub fn rename(&mut self, old: &Path, new: &Path) {
        if let Some(placement) = self.placements.remove(old) {
            self.placements.insert(new.to_path_buf(), placement);
        }
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }

        let mut items: Vec<StoredItem> = self
            .placements
            .iter()
            .map(|(path, placement)| StoredItem {
                path: path.to_string_lossy().into_owned(),
                output: placement.output.clone(),
                column: placement.column,
                row: placement.row,
            })
            .collect();
        items.sort_by(|a, b| a.path.cmp(&b.path));

        let text = toml::to_string_pretty(&StateFile { version: 1, items })
            .context("cannot serialise icon positions")?;
        let temporary = self.path.with_extension("toml.tmp");
        std::fs::write(&temporary, text)
            .with_context(|| format!("cannot write {}", temporary.display()))?;
        std::fs::rename(&temporary, &self.path)
            .with_context(|| format!("cannot replace {}", self.path.display()))?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir()
            .join(format!("tessera-state-{name}-{unique}"))
            .join("positions.toml")
    }

    #[test]
    fn round_trips_placements() {
        let path = temp_path("roundtrip");
        let mut state = StateStore::load(path.clone());
        state.set(
            PathBuf::from("/home/me/Desktop/Notes.txt"),
            Placement {
                output: "DP-2".into(),
                column: 3,
                row: 5,
            },
        );
        state.save().expect("save state");

        let loaded = StateStore::load(path.clone());
        assert_eq!(
            loaded.placement(Path::new("/home/me/Desktop/Notes.txt")),
            Some(&Placement {
                output: "DP-2".into(),
                column: 3,
                row: 5,
            })
        );

        let _ = std::fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn renaming_moves_the_stored_placement() {
        let path = temp_path("rename");
        let mut state = StateStore::load(path.clone());
        state.set(
            PathBuf::from("/home/me/Desktop/Old Name.txt"),
            Placement {
                output: "DP-1".into(),
                column: 1,
                row: 2,
            },
        );
        state.rename(
            Path::new("/home/me/Desktop/Old Name.txt"),
            Path::new("/home/me/Desktop/New Name.txt"),
        );
        assert_eq!(
            state.placement(Path::new("/home/me/Desktop/Old Name.txt")),
            None
        );
        assert_eq!(
            state.placement(Path::new("/home/me/Desktop/New Name.txt")),
            Some(&Placement {
                output: "DP-1".into(),
                column: 1,
                row: 2,
            })
        );
    }

    #[test]
    fn missing_state_is_empty() {
        let state = StateStore::load(temp_path("missing"));
        assert_eq!(state.placement(Path::new("/nope")), None);
    }

    #[test]
    fn malformed_state_falls_back_to_empty() {
        let path = temp_path("malformed");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, "not valid = [").expect("write");
        let state = StateStore::load(path.clone());
        assert_eq!(state.placement(Path::new("/nope")), None);
        let _ = std::fs::remove_dir_all(path.parent().expect("parent"));
    }
}
