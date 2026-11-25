//! Rearranging icons inside Tessera by dragging them.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;

use crate::desktop::DesktopItem;
use crate::layout::Slot;
use crate::state::Placement;

use super::DesktopHost;

pub(super) struct DragState {
    path: PathBuf,
    source_view: usize,
    pointer_start: (f64, f64),
    source_button: gtk::Button,
    suppress_until: Rc<Cell<Option<Instant>>>,
    target: Option<(usize, u32, u32)>,
}

impl DesktopHost {
    pub(super) fn begin_drag(
        self: &Rc<Self>,
        item: DesktopItem,
        source_view: usize,
        slot: Slot,
        start: (f64, f64),
        source_button: gtk::Button,
        suppress_until: Rc<Cell<Option<Instant>>>,
    ) {
        if self.drag.borrow().is_some() {
            return;
        }

        self.select_item(
            item.clone(),
            source_button.clone(),
            source_view,
            slot.column,
            slot.row,
        );
        source_button.add_css_class("dragging");
        suppress_until.set(Some(Instant::now() + Duration::from_secs(2)));

        *self.drag.borrow_mut() = Some(DragState {
            path: item.path,
            source_view,
            pointer_start: (slot.x as f64 + start.0, slot.y as f64 + start.1),
            source_button,
            suppress_until,
            target: None,
        });
        self.update_drag(0.0, 0.0);
    }

    pub(super) fn update_drag(self: &Rc<Self>, offset_x: f64, offset_y: f64) {
        let Some((path, pointer_start, source_view)) = ({
            let drag = self.drag.borrow();
            drag.as_ref()
                .map(|drag| (drag.path.clone(), drag.pointer_start, drag.source_view))
        }) else {
            return;
        };

        let Some(source) = self.views.borrow().get(source_view).cloned() else {
            return;
        };
        let (origin_x, origin_y) = source.origin();
        let global_x = origin_x as f64 + pointer_start.0 + offset_x;
        let global_y = origin_y as f64 + pointer_start.1 + offset_y;

        let target = {
            let scene = self.scene.borrow();
            let Some(scene) = scene.as_ref() else {
                return;
            };
            scene.drop_target(global_x, global_y, &path)
        };

        if let Some(drag) = self.drag.borrow_mut().as_mut() {
            drag.target = target;
        }

        let views = self.views.borrow().clone();
        for view in &views {
            view.hide_drop_target();
        }
        if let Some((index, column, row)) = target
            && let Some(scene) = self.scene.borrow().as_ref()
            && let Some(grid) = scene.grids.get(index)
            && let Some(slot) = grid.slot(column, row)
            && let Some(view) = views.get(index)
        {
            view.show_drop_target(slot);
        }
    }

    pub(super) fn end_drag(self: &Rc<Self>) {
        let Some(drag) = self.drag.borrow_mut().take() else {
            return;
        };

        for view in self.views.borrow().iter() {
            view.hide_drop_target();
        }
        drag.source_button.remove_css_class("dragging");
        drag.suppress_until
            .set(Some(Instant::now() + Duration::from_millis(150)));

        if let Some((output, column, row)) = drag.target {
            let connector = self
                .views
                .borrow()
                .get(output)
                .map(|view| view.connector().to_owned());
            if let Some(connector) = connector {
                self.state.borrow_mut().set(
                    drag.path,
                    Placement {
                        output: connector,
                        column,
                        row,
                    },
                );
                self.save_state();
            }
        }

        self.render();
    }

    pub(super) fn cancel_drag(self: &Rc<Self>) {
        let Some(drag) = self.drag.borrow_mut().take() else {
            return;
        };
        for view in self.views.borrow().iter() {
            view.hide_drop_target();
        }
        drag.source_button.remove_css_class("dragging");
        drag.suppress_until
            .set(Some(Instant::now() + Duration::from_millis(150)));
    }
}
