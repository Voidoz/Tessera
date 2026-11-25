//! Geometry for the per-output icon grids.
//!
//! Items live in grid slots, not arbitrary pixel coordinates.  Dragging snaps
//! to the nearest free slot, so positions survive font/icon-size changes and
//! are trivial to persist.  The first slot grows from the configured corner.

use serde::Deserialize;

/// Which corner of the output the icon grid grows from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Corner {
    #[default]
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    pub fn is_right(self) -> bool {
        matches!(self, Self::TopRight | Self::BottomRight)
    }

    pub fn is_bottom(self) -> bool {
        matches!(self, Self::BottomLeft | Self::BottomRight)
    }
}

/// Smallest and largest icon sizes the layout accepts, in logical pixels.
pub const MIN_ICON_PX: u32 = 32;
pub const MAX_ICON_PX: u32 = 160;

/// Sizes that control the grid.  Deliberately independent of GTK so the
/// placement rules can be tested without a compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridSpec {
    pub icon_size: i32,
    pub font_size: i32,
    pub gap: i32,
    pub margin: i32,
}

impl GridSpec {
    pub fn new(icon_size: u32, font_size: u32, gap: u32, margin: u32) -> Self {
        Self {
            icon_size: icon_size.clamp(MIN_ICON_PX, MAX_ICON_PX) as i32,
            font_size: font_size.clamp(9, 28) as i32,
            gap: gap.clamp(0, 48) as i32,
            margin: margin.clamp(0, 240) as i32,
        }
    }

    pub fn cell_width(self, icon_size: i32) -> i32 {
        icon_size + 48
    }

    pub fn cell_height(self, icon_size: i32) -> i32 {
        // Padding above and below the icon, one gap below the icon, and two
        // lines of text.
        icon_size + 22 + (self.font_size * 5 / 4) * 2
    }

    fn with_icon_size(self, icon_size: i32) -> Self {
        Self { icon_size, ..self }
    }
}

/// Size of one output's surface plus the space reserved by panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputSpec {
    pub width: i32,
    pub height: i32,
    /// Space reserved on both the left and right output edges.
    pub reserved_x: i32,
    /// Space reserved on both the top and bottom output edges.
    pub reserved_y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Insets {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridMetrics {
    pub icon_size: i32,
    pub cell_width: i32,
    pub cell_height: i32,
    pub step_x: i32,
    pub step_y: i32,
}

/// A slot on an output, in output-local logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub column: u32,
    pub row: u32,
}

#[derive(Debug, Clone)]
pub struct OutputGrid {
    pub width: i32,
    pub height: i32,
    pub insets: Insets,
    pub metrics: GridMetrics,
    pub columns: u32,
    pub rows: u32,
    pub corner: Corner,
}

impl OutputGrid {
    pub fn new(output: OutputSpec, spec: GridSpec, corner: Corner) -> Self {
        let metrics = GridMetrics {
            icon_size: spec.icon_size,
            cell_width: spec.cell_width(spec.icon_size),
            cell_height: spec.cell_height(spec.icon_size),
            step_x: spec.cell_width(spec.icon_size) + spec.gap,
            step_y: spec.cell_height(spec.icon_size) + spec.gap,
        };

        // Conservative on purpose: the protocol does not tell us which edge a
        // panel is on.  Reserving the available-area difference on all sides
        // guarantees that icons never end up under a panel, whatever its edge.
        let inset_x = spec.margin + output.reserved_x.max(0);
        let inset_y = spec.margin + output.reserved_y.max(0);
        let insets = Insets {
            left: inset_x,
            top: inset_y,
            right: inset_x,
            bottom: inset_y,
        };

        let usable_w = output.width - insets.left - insets.right;
        let usable_h = output.height - insets.top - insets.bottom;
        let columns = ((usable_w + spec.gap) / metrics.step_x).max(1) as u32;
        let rows = ((usable_h + spec.gap) / metrics.step_y).max(1) as u32;

        Self {
            width: output.width,
            height: output.height,
            insets,
            metrics,
            columns,
            rows,
            corner,
        }
    }

    pub fn capacity(&self) -> usize {
        self.columns as usize * self.rows as usize
    }

    pub fn contains_slot(&self, column: u32, row: u32) -> bool {
        column < self.columns && row < self.rows
    }

    pub fn slot(&self, column: u32, row: u32) -> Option<Slot> {
        if !self.contains_slot(column, row) {
            return None;
        }

        let x = if self.corner.is_right() {
            self.width
                - self.insets.right
                - self.metrics.cell_width
                - column as i32 * self.metrics.step_x
        } else {
            self.insets.left + column as i32 * self.metrics.step_x
        };
        let y = if self.corner.is_bottom() {
            self.height
                - self.insets.bottom
                - self.metrics.cell_height
                - row as i32 * self.metrics.step_y
        } else {
            self.insets.top + row as i32 * self.metrics.step_y
        };

        Some(Slot {
            x,
            y,
            width: self.metrics.cell_width,
            height: self.metrics.cell_height,
            column,
            row,
        })
    }

    /// All slots in column-major order, which is the order used when placing
    /// new items automatically.
    pub fn slots(&self) -> Vec<Slot> {
        let mut slots = Vec::with_capacity(self.capacity());
        for column in 0..self.columns {
            for row in 0..self.rows {
                if let Some(slot) = self.slot(column, row) {
                    slots.push(slot);
                }
            }
        }
        slots
    }

    /// The slot whose center is nearest to `(x, y)`.
    pub fn nearest_slot(&self, x: i32, y: i32) -> Option<Slot> {
        self.slots().into_iter().min_by_key(|slot| {
            let dx = x - (slot.x + slot.width / 2);
            let dy = y - (slot.y + slot.height / 2);
            dx * dx + dy * dy
        })
    }
}

/// Choose the largest icon size (in four-pixel steps) whose combined capacity
/// across all outputs can hold every item.
pub fn compact_spec(
    outputs: &[OutputSpec],
    spec: GridSpec,
    corner: Corner,
    item_count: usize,
) -> GridSpec {
    if outputs.is_empty() || item_count == 0 {
        return spec;
    }

    let minimum = MIN_ICON_PX as i32;
    let mut icon_size = spec.icon_size;
    loop {
        let candidate = spec.with_icon_size(icon_size);
        let capacity: usize = outputs
            .iter()
            .map(|output| OutputGrid::new(*output, candidate, corner).capacity())
            .sum();
        if capacity >= item_count || icon_size <= minimum {
            return candidate;
        }
        icon_size = (icon_size - 4).max(minimum);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> GridSpec {
        GridSpec::new(64, 13, 8, 32)
    }

    fn output(width: i32, height: i32) -> OutputSpec {
        OutputSpec {
            width,
            height,
            reserved_x: 0,
            reserved_y: 0,
        }
    }

    #[test]
    fn fills_columns_from_top_left() {
        let grid = OutputGrid::new(output(1080, 1920), spec(), Corner::TopLeft);
        assert_eq!(grid.insets.left, 32);
        assert_eq!(grid.insets.top, 32);
        let first = grid.slot(0, 0).expect("slot");
        let below = grid.slot(0, 1).expect("slot");
        let next_column = grid.slot(1, 0).expect("slot");
        assert_eq!(first.x, 32);
        assert_eq!(first.y, 32);
        assert!(below.y > first.y);
        assert_eq!(below.x, first.x);
        assert!(next_column.x > first.x);
        assert_eq!(next_column.y, first.y);
    }

    #[test]
    fn respects_other_corners() {
        let grid = OutputGrid::new(output(1000, 800), spec(), Corner::BottomRight);
        let first = grid.slot(0, 0).expect("slot");
        assert_eq!(first.x + first.width, 1000 - 32);
        assert_eq!(first.y + first.height, 800 - 32);
        let second = grid.slot(0, 1).expect("slot");
        assert!(second.y < first.y);
        let next_column = grid.slot(1, 0).expect("slot");
        assert!(next_column.x < first.x);
    }

    #[test]
    fn panel_reservations_reduce_the_safe_area() {
        let grid = OutputGrid::new(
            OutputSpec {
                width: 1080,
                height: 1920,
                reserved_x: 0,
                reserved_y: 44,
            },
            spec(),
            Corner::TopLeft,
        );
        assert_eq!(grid.insets.top, 76);
        assert_eq!(grid.insets.bottom, 76);
        let first = grid.slot(0, 0).expect("slot");
        assert_eq!(first.y, 76);
    }

    #[test]
    fn nearest_slot_finds_the_slot_under_the_pointer() {
        let grid = OutputGrid::new(output(1080, 1920), spec(), Corner::TopLeft);
        let target = grid.slot(2, 3).expect("slot");
        let found = grid
            .nearest_slot(target.x + target.width / 2, target.y + target.height / 2)
            .expect("nearest");
        assert_eq!((found.column, found.row), (2, 3));
    }

    #[test]
    fn compacts_across_multiple_screens() {
        let outputs = [output(800, 600), output(800, 600)];
        let compacted = compact_spec(&outputs, spec(), Corner::TopLeft, 70);
        assert!(compacted.icon_size < spec().icon_size);
        let capacity: usize = outputs
            .iter()
            .map(|output| OutputGrid::new(*output, compacted, Corner::TopLeft).capacity())
            .sum();
        assert!(capacity >= 70);
    }

    #[test]
    fn one_cell_capacity_on_a_tiny_output() {
        let grid = OutputGrid::new(output(100, 100), spec(), Corner::TopLeft);
        assert_eq!(grid.capacity(), 1);
    }
}
