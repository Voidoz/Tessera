//! Accepting files dragged in from other applications.

use std::rc::Rc;

use gtk::gio;
use gtk::prelude::*;
use log::warn;

use crate::fileops;
use crate::layout::Slot;
use crate::state::Placement;

use super::DesktopHost;

impl DesktopHost {
    /// Copy incoming files into the desktop folder and pin them near the drop
    /// point.  Copies never overwrite an existing name.
    pub(super) fn accept_drop(
        self: &Rc<Self>,
        connector: &str,
        files: Vec<gio::File>,
        x: f64,
        y: f64,
    ) {
        let mut copied = Vec::new();
        for file in files {
            let Some(source) = file.path() else {
                warn!("ignoring non-local drop: {}", file.uri());
                continue;
            };
            let Some(name) = source.file_name() else {
                continue;
            };
            let destination = fileops::unique_destination(&self.desktop_dir, name);
            match fileops::copy_path(&source, &destination) {
                Ok(()) => copied.push(destination),
                Err(error) => warn!(
                    "cannot copy {} to {}: {error}",
                    source.display(),
                    destination.display()
                ),
            }
        }

        if copied.is_empty() {
            return;
        }

        let placements = self.drop_placements(connector, x, y, &copied);
        if !placements.is_empty() {
            {
                let mut state = self.state.borrow_mut();
                for (path, placement) in placements {
                    state.set(path, placement);
                }
            }
            self.save_state();
        }

        self.render();
    }

    /// Assign the first free slots nearest to the drop point.
    fn drop_placements(
        &self,
        connector: &str,
        x: f64,
        y: f64,
        copied: &[std::path::PathBuf],
    ) -> Vec<(std::path::PathBuf, Placement)> {
        let scene = self.scene.borrow();
        let Some(scene) = scene.as_ref() else {
            return Vec::new();
        };
        let Some(index) = scene.index_of(connector) else {
            return Vec::new();
        };
        let Some(grid) = scene.grids.get(index) else {
            return Vec::new();
        };
        let Some(occupied) = scene.occupants.get(index) else {
            return Vec::new();
        };

        let local_x = x as i32;
        let local_y = y as i32;
        let mut free: Vec<Slot> = grid
            .slots()
            .into_iter()
            .filter(|slot| !occupied.contains_key(&(slot.column, slot.row)))
            .collect();
        free.sort_by_key(|slot| {
            let dx = local_x - (slot.x + slot.width / 2);
            let dy = local_y - (slot.y + slot.height / 2);
            dx * dx + dy * dy
        });

        copied
            .iter()
            .zip(free.iter())
            .map(|(path, slot)| {
                (
                    path.clone(),
                    Placement {
                        output: connector.to_owned(),
                        column: slot.column,
                        row: slot.row,
                    },
                )
            })
            .collect()
    }
}
