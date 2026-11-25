//! The render plan: which item occupies which slot on which output.
//!
//! A scene is computed from the desktop listing, the output geometry, the
//! persisted positions and the configuration.  It is deliberately free of
//! GTK widgets so the placement decisions can be tested without a compositor.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::assignment::{Assignment, assign_items};
use crate::config::Config;
use crate::desktop::DesktopItem;
use crate::layout::{OutputGrid, OutputSpec, Slot, compact_spec};
use crate::state::Placement;

/// One output as the scene sees it: a connector name plus its geometry.
#[derive(Debug, Clone)]
pub struct OutputInfo {
    pub connector: String,
    /// Full logical output geometry, including the space reserved by panels.
    pub spec: OutputSpec,
    /// Output origin in the compositor's global coordinate space.
    pub origin: (i32, i32),
}

impl OutputInfo {
    fn contains(&self, x: f64, y: f64) -> bool {
        let (origin_x, origin_y) = self.origin;
        x >= origin_x as f64
            && y >= origin_y as f64
            && x < origin_x as f64 + self.spec.width as f64
            && y < origin_y as f64 + self.spec.height as f64
    }
}

/// The complete placement plan for one render.
#[derive(Debug)]
pub struct Scene {
    pub outputs: Vec<OutputInfo>,
    pub items: Vec<DesktopItem>,
    pub grids: Vec<OutputGrid>,
    pub assignments: HashMap<PathBuf, Assignment>,
    /// Occupied cells per output, used for free-slot search.
    pub occupants: Vec<HashMap<(u32, u32), PathBuf>>,
    /// Items that did not fit on any output.
    pub hidden: usize,
    pub icon_size: i32,
}

impl Scene {
    /// Index of the output called `connector`, compared case-insensitively.
    pub fn index_of(&self, connector: &str) -> Option<usize> {
        self.outputs
            .iter()
            .position(|output| output.connector.eq_ignore_ascii_case(connector))
    }

    /// The output and pixel geometry of one item's slot.
    pub fn slot_for(&self, path: &Path) -> Option<(usize, Slot)> {
        let assignment = self.assignments.get(path)?;
        let grid = self.grids.get(assignment.output)?;
        let slot = grid.slot(assignment.column, assignment.row)?;
        Some((assignment.output, slot))
    }

    /// The slot under a global pointer position, falling back to the nearest
    /// free slot on that output.  `dragged` may reuse its own cell.
    pub fn drop_target(&self, x: f64, y: f64, dragged: &Path) -> Option<(usize, u32, u32)> {
        let index = self
            .outputs
            .iter()
            .position(|output| output.contains(x, y))?;
        let output = self.outputs.get(index)?;
        let grid = self.grids.get(index)?;
        let occupied = self.occupants.get(index)?;

        let local_x = (x - output.origin.0 as f64) as i32;
        let local_y = (y - output.origin.1 as f64) as i32;
        let free = |slot: &Slot| {
            occupied
                .get(&(slot.column, slot.row))
                .is_none_or(|path| path.as_path() == dragged)
        };

        if let Some(nearest) = grid.nearest_slot(local_x, local_y)
            && free(&nearest)
        {
            return Some((index, nearest.column, nearest.row));
        }

        let best = grid.slots().into_iter().filter(free).min_by_key(|slot| {
            let dx = local_x - (slot.x + slot.width / 2);
            let dy = local_y - (slot.y + slot.height / 2);
            dx * dx + dy * dy
        })?;
        Some((index, best.column, best.row))
    }
}

/// The output that receives unpinned items first: an explicit preference,
/// then the output at the layout origin, then the first one.
fn preferred_output(outputs: &[OutputInfo], preferred: Option<&str>) -> usize {
    preferred
        .and_then(|wanted| {
            outputs
                .iter()
                .position(|output| output.connector.eq_ignore_ascii_case(wanted))
        })
        .or_else(|| outputs.iter().position(|output| output.origin == (0, 0)))
        .unwrap_or(0)
}

/// Build the placement plan for one render.
pub fn build_scene(
    items: Vec<DesktopItem>,
    outputs: &[OutputInfo],
    config: &Config,
    preferred: Option<&str>,
    pinned: &HashMap<PathBuf, Placement>,
) -> Scene {
    let specs: Vec<OutputSpec> = outputs.iter().map(|output| output.spec).collect();
    let spec = compact_spec(&specs, config.grid_spec(), config.corner, items.len());
    let grids: Vec<OutputGrid> = specs
        .iter()
        .map(|output| OutputGrid::new(*output, spec, config.corner))
        .collect();
    let connectors: Vec<String> = outputs
        .iter()
        .map(|output| output.connector.clone())
        .collect();

    let preferred = preferred_output(outputs, preferred);
    let mut output_order: Vec<usize> = (0..outputs.len()).collect();
    output_order.retain(|index| *index != preferred);
    output_order.insert(0, preferred);

    let paths: Vec<PathBuf> = items.iter().map(|item| item.path.clone()).collect();
    let (assignments, hidden) = assign_items(&paths, &connectors, &grids, pinned, &output_order);

    let mut occupants: Vec<HashMap<(u32, u32), PathBuf>> =
        grids.iter().map(|_| HashMap::new()).collect();
    for (path, assignment) in &assignments {
        occupants[assignment.output].insert((assignment.column, assignment.row), path.clone());
    }

    Scene {
        outputs: outputs.to_vec(),
        items,
        grids,
        assignments,
        occupants,
        hidden,
        icon_size: spec.icon_size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::OpenAction;
    use gtk::prelude::*;

    fn item(path: &str) -> DesktopItem {
        let name = Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_owned());
        DesktopItem {
            path: PathBuf::from(path),
            name: name.clone(),
            icon: gtk::gio::ThemedIcon::new("text-x-generic").upcast(),
            action: OpenAction::Uri(String::new()),
            is_dir: false,
            content_type: None,
            sort_key: gtk::glib::FilenameCollationKey::from(name),
        }
    }

    fn output(connector: &str, x: i32, width: i32, height: i32) -> OutputInfo {
        OutputInfo {
            connector: connector.to_owned(),
            spec: OutputSpec {
                width,
                height,
                reserved_x: 0,
                reserved_y: 0,
            },
            origin: (x, 0),
        }
    }

    fn small_outputs() -> [OutputInfo; 2] {
        [output("DP-1", 0, 200, 200), output("DP-2", 200, 200, 200)]
    }

    #[test]
    fn named_output_wins_over_layout_position() {
        let outputs = small_outputs();
        assert_eq!(preferred_output(&outputs, Some("dp-2")), 1);
        assert_eq!(preferred_output(&outputs, Some("HDMI-1")), 0);
    }

    #[test]
    fn falls_back_to_the_layout_origin_then_the_first_output() {
        let offset = [output("DP-1", 100, 200, 200), output("DP-2", 300, 200, 200)];
        assert_eq!(preferred_output(&offset, None), 0);

        let origin_second = [output("DP-1", 200, 200, 200), output("DP-2", 0, 200, 200)];
        assert_eq!(preferred_output(&origin_second, None), 1);
    }

    #[test]
    fn spills_to_later_outputs_in_order() {
        let outputs = small_outputs();
        let items = vec![item("/desktop/a"), item("/desktop/b")];
        let scene = build_scene(items, &outputs, &Config::default(), None, &HashMap::new());

        assert_eq!(scene.hidden, 0);
        assert_eq!(scene.assignments[Path::new("/desktop/a")].output, 0);
        assert_eq!(scene.assignments[Path::new("/desktop/b")].output, 1);
        assert_eq!(scene.occupants[1].len(), 1);
    }

    #[test]
    fn a_preferred_output_receives_the_first_item() {
        let outputs = small_outputs();
        let items = vec![item("/desktop/a"), item("/desktop/b")];
        let scene = build_scene(
            items,
            &outputs,
            &Config::default(),
            Some("DP-2"),
            &HashMap::new(),
        );

        assert_eq!(scene.assignments[Path::new("/desktop/a")].output, 1);
        assert_eq!(scene.assignments[Path::new("/desktop/b")].output, 0);
    }

    #[test]
    fn drop_target_prefers_the_slot_under_the_pointer() {
        let outputs = [output("DP-1", 0, 1080, 1920)];
        let items = vec![item("/desktop/a"), item("/desktop/b")];
        let scene = build_scene(items, &outputs, &Config::default(), None, &HashMap::new());

        // Both upper slots are taken; the dragged item keeps its own cell.
        assert_eq!(
            scene.drop_target(40.0, 40.0, Path::new("/desktop/a")),
            Some((0, 0, 0))
        );
        // An item without a slot gets the nearest free one: the top of the
        // next column is closer than the third row of the first column.
        assert_eq!(
            scene.drop_target(40.0, 40.0, Path::new("/desktop/c")),
            Some((0, 1, 0))
        );
    }

    #[test]
    fn drop_target_uses_the_output_under_the_pointer() {
        let outputs = small_outputs();
        let scene = build_scene(
            Vec::new(),
            &outputs,
            &Config::default(),
            None,
            &HashMap::new(),
        );

        assert_eq!(
            scene.drop_target(250.0, 50.0, Path::new("/desktop/a")),
            Some((1, 0, 0))
        );
        // Outside every output.
        assert_eq!(
            scene.drop_target(1000.0, 50.0, Path::new("/desktop/a")),
            None
        );
    }

    #[test]
    fn slot_for_maps_an_item_to_pixels() {
        let outputs = [output("DP-1", 0, 200, 200)];
        let scene = build_scene(
            vec![item("/desktop/a")],
            &outputs,
            &Config::default(),
            None,
            &HashMap::new(),
        );

        let (index, slot) = scene.slot_for(Path::new("/desktop/a")).expect("slot");
        assert_eq!(index, 0);
        assert_eq!((slot.column, slot.row), (0, 0));
        assert_eq!(slot.x, 32);
        assert_eq!(slot.y, 32);
    }
}
