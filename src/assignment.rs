//! Assigning items to output grids, respecting manually pinned slots.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::layout::OutputGrid;
use crate::state::Placement;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assignment {
    pub output: usize,
    pub column: u32,
    pub row: u32,
}

/// Place pinned items first, then fill the remaining items in the supplied
/// output order.  Pinned entries with a missing output, an out-of-range slot,
/// or a collision are treated as unpinned.
pub fn assign_items(
    items: &[PathBuf],
    connectors: &[String],
    grids: &[OutputGrid],
    pinned: &HashMap<PathBuf, Placement>,
    output_order: &[usize],
) -> (HashMap<PathBuf, Assignment>, usize) {
    let mut assignments = HashMap::new();
    let mut occupied: Vec<HashSet<(u32, u32)>> = grids.iter().map(|_| HashSet::new()).collect();

    for path in items {
        let Some(placement) = pinned.get(path) else {
            continue;
        };
        let Some(output) = connectors
            .iter()
            .position(|connector| connector.eq_ignore_ascii_case(&placement.output))
        else {
            continue;
        };
        let Some(grid) = grids.get(output) else {
            continue;
        };
        if !grid.contains_slot(placement.column, placement.row) {
            continue;
        }
        let cell = (placement.column, placement.row);
        if !occupied[output].insert(cell) {
            continue;
        }
        assignments.insert(
            path.clone(),
            Assignment {
                output,
                column: placement.column,
                row: placement.row,
            },
        );
    }

    for path in items {
        if assignments.contains_key(path) {
            continue;
        }

        'outputs: for &output in output_order {
            let Some(grid) = grids.get(output) else {
                continue;
            };
            for slot in grid.slots() {
                if occupied[output].insert((slot.column, slot.row)) {
                    assignments.insert(
                        path.clone(),
                        Assignment {
                            output,
                            column: slot.column,
                            row: slot.row,
                        },
                    );
                    break 'outputs;
                }
            }
        }
    }

    let hidden = items.len().saturating_sub(assignments.len());
    (assignments, hidden)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{Corner, GridSpec, OutputGrid, OutputSpec};
    use std::path::Path;

    fn grid(width: i32, height: i32) -> OutputGrid {
        OutputGrid::new(
            OutputSpec {
                width,
                height,
                reserved_x: 0,
                reserved_y: 0,
            },
            GridSpec::new(64, 13, 8, 32),
            Corner::TopLeft,
        )
    }

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn fills_preferred_output_then_spills() {
        // Each 200x200 grid has one slot at the base metrics.
        let grids = [grid(200, 200), grid(200, 200)];
        let connectors = ["DP-1".to_owned(), "DP-2".to_owned()];
        let items = paths(&["a", "b"]);
        let (assigned, hidden) =
            assign_items(&items, &connectors, &grids, &HashMap::new(), &[1, 0]);
        assert_eq!(hidden, 0);
        assert_eq!(assigned[Path::new("a")].output, 1);
        assert_eq!(assigned[Path::new("b")].output, 0);
    }

    #[test]
    fn pinned_slots_are_reserved() {
        let grids = [grid(1080, 1920)];
        let connectors = ["DP-1".to_owned()];
        let items = paths(&["pinned", "auto"]);
        let mut pinned = HashMap::new();
        pinned.insert(
            PathBuf::from("pinned"),
            Placement {
                output: "DP-1".into(),
                column: 0,
                row: 0,
            },
        );
        let (assigned, hidden) = assign_items(&items, &connectors, &grids, &pinned, &[0]);
        assert_eq!(hidden, 0);
        assert_eq!(
            (
                assigned[Path::new("pinned")].column,
                assigned[Path::new("pinned")].row
            ),
            (0, 0)
        );
        assert_ne!(
            (
                assigned[Path::new("auto")].column,
                assigned[Path::new("auto")].row
            ),
            (0, 0)
        );
    }

    #[test]
    fn collisions_fall_back_to_auto_placement() {
        let grids = [grid(1080, 1920)];
        let connectors = ["DP-1".to_owned()];
        let items = paths(&["one", "two"]);
        let mut pinned = HashMap::new();
        for name in ["one", "two"] {
            pinned.insert(
                PathBuf::from(name),
                Placement {
                    output: "DP-1".into(),
                    column: 0,
                    row: 0,
                },
            );
        }
        let (assigned, hidden) = assign_items(&items, &connectors, &grids, &pinned, &[0]);
        assert_eq!(hidden, 0);
        assert_ne!(
            (
                assigned[Path::new("one")].column,
                assigned[Path::new("one")].row
            ),
            (
                assigned[Path::new("two")].column,
                assigned[Path::new("two")].row
            )
        );
    }

    #[test]
    fn reports_items_that_have_nowhere_to_go() {
        let grids = [grid(100, 100)];
        let connectors = ["DP-1".to_owned()];
        let items = paths(&["a", "b"]);
        let (assigned, hidden) = assign_items(&items, &connectors, &grids, &HashMap::new(), &[0]);
        assert_eq!(assigned.len(), 1);
        assert_eq!(hidden, 1);
    }
}
