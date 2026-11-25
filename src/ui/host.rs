//! Lifecycle, polling, painting, keyboard handling and selection.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use log::{debug, warn};

use crate::config::{Config, DropTargetMode};
use crate::desktop::{self, DesktopItem};
use crate::scene::{OutputInfo, Scene, build_scene};
use crate::state::{Placement, StateStore};

use super::DesktopHost;
use super::css;
use super::icon::make_icon_button;
use super::surface::OutputView;

/// The icon that currently owns keyboard focus and the context menu.
#[derive(Clone)]
pub(super) struct Selection {
    item: DesktopItem,
    button: gtk::Button,
    output: usize,
    column: u32,
    row: u32,
}

impl DesktopHost {
    pub fn start(
        app: &gtk::Application,
        config: Config,
        preferred_output: Option<String>,
        desktop_dir: PathBuf,
        state_file: PathBuf,
    ) -> Rc<Self> {
        let hold = app.hold();

        let host = Rc::new(Self {
            app: app.clone(),
            config,
            preferred_output,
            desktop_dir,
            state: RefCell::new(StateStore::load(state_file)),
            views: RefCell::new(Vec::new()),
            scene: RefCell::new(None),
            selected: RefCell::new(None),
            overlay: RefCell::new(None),
            file_monitor: RefCell::new(None),
            reload_source: RefCell::new(None),
            rebuild_source: RefCell::new(None),
            drag: RefCell::new(None),
            _poller: RefCell::new(None),
            _hold: hold,
        });

        let weak = Rc::downgrade(&host);
        let poller = glib::timeout_add_local(Duration::from_millis(500), move || {
            if let Some(host) = weak.upgrade() {
                host.poll();
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });
        *host._poller.borrow_mut() = Some(poller);

        css::install();
        host.install_file_monitor();
        host.install_display_listener();
        host.rebuild_outputs();
        host
    }

    fn install_file_monitor(self: &Rc<Self>) {
        let file = gio::File::for_path(&self.desktop_dir);
        let monitor =
            match file.monitor_directory(gio::FileMonitorFlags::NONE, None::<&gio::Cancellable>) {
                Ok(monitor) => monitor,
                Err(error) => {
                    warn!(
                        "cannot watch desktop directory {}: {error}",
                        self.desktop_dir.display()
                    );
                    return;
                }
            };

        let weak = Rc::downgrade(self);
        monitor.connect_changed(move |_, _, _, _| {
            if let Some(host) = weak.upgrade() {
                host.schedule_reload();
            }
        });
        *self.file_monitor.borrow_mut() = Some(monitor);
    }

    fn install_display_listener(self: &Rc<Self>) {
        let Some(display) = gdk::Display::default() else {
            return;
        };

        let weak = Rc::downgrade(self);
        display.monitors().connect_items_changed(move |_, _, _, _| {
            if let Some(host) = weak.upgrade() {
                host.schedule_rebuild();
            }
        });
    }

    fn schedule_reload(self: &Rc<Self>) {
        if let Some(source) = self.reload_source.borrow_mut().take() {
            source.remove();
        }

        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local(Duration::from_millis(160), move || {
            if let Some(host) = weak.upgrade() {
                *host.reload_source.borrow_mut() = None;
                host.render();
            }
            glib::ControlFlow::Break
        });
        *self.reload_source.borrow_mut() = Some(source);
    }

    fn schedule_rebuild(self: &Rc<Self>) {
        if let Some(source) = self.rebuild_source.borrow_mut().take() {
            source.remove();
        }

        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local(Duration::from_millis(250), move || {
            if let Some(host) = weak.upgrade() {
                *host.rebuild_source.borrow_mut() = None;
                host.rebuild_outputs();
            }
            glib::ControlFlow::Break
        });
        *self.rebuild_source.borrow_mut() = Some(source);
    }

    fn rebuild_outputs(self: &Rc<Self>) {
        let Some(display) = gdk::Display::default() else {
            return;
        };

        let model = display.monitors();
        let mut monitors = Vec::new();
        for index in 0..model.n_items() {
            let Some(monitor) = model.item(index).and_downcast::<gdk::Monitor>() else {
                continue;
            };
            let connector = monitor
                .connector()
                .map(|connector| connector.to_string())
                .unwrap_or_else(|| format!("output-{index}"));
            monitors.push((monitor, connector));
        }
        monitors.sort_by_key(|(monitor, _)| {
            let geometry = monitor.geometry();
            (geometry.x(), geometry.y())
        });

        let old_views = self.views.replace(Vec::new());
        *self.scene.borrow_mut() = None;
        if self.drag.borrow().is_some() {
            self.cancel_drag();
        }
        self.clear_selection();
        for view in &old_views {
            view.close();
        }

        if monitors.is_empty() {
            warn!("no outputs found");
            return;
        }

        let mut views = Vec::with_capacity(monitors.len());
        for (monitor, connector) in monitors {
            debug!("creating desktop surface for {connector}");
            views.push(OutputView::new(
                &self.app,
                &monitor,
                connector,
                Rc::downgrade(self),
            ));
        }
        *self.views.borrow_mut() = views;
        self.render();
    }

    fn poll(self: &Rc<Self>) {
        let views = self.views.borrow().clone();
        let mut rerender = false;
        let mut rebuild = false;

        for view in &views {
            let poll = view.poll();
            rerender |= poll.rerender;
            rebuild |= poll.rebuild;
        }

        if rebuild {
            self.schedule_rebuild();
        } else if rerender {
            self.render();
        }
    }

    /// Compute a fresh scene and paint it over every output.
    pub(super) fn render(self: &Rc<Self>) {
        let views = self.views.borrow().clone();
        if views.is_empty() {
            return;
        }

        // A folder refresh or panel change can happen mid-drag.  Rebuilding
        // the widgets invalidates the source button, so end the gesture
        // cleanly rather than leaving stale state behind.
        if self.drag.borrow().is_some() {
            self.cancel_drag();
        }
        if self.overlay.borrow().is_some() {
            self.close_overlay();
        }

        let outputs: Vec<OutputInfo> = views.iter().map(|view| view.output_info()).collect();
        let items = desktop::scan(&self.desktop_dir);
        let pinned = self.pinned_placements(&items);
        let scene = build_scene(
            items,
            &outputs,
            &self.config,
            self.preferred_output.as_deref(),
            &pinned,
        );
        if scene.hidden > 0 {
            warn!("{} desktop item(s) do not fit on any output", scene.hidden);
        }

        self.clear_selection();
        self.paint(&views, &scene);
        debug!(
            "rendered {} items on {} output(s), icon size {}",
            scene.assignments.len(),
            views.len(),
            scene.icon_size
        );
        *self.scene.borrow_mut() = Some(scene);
    }

    fn paint(self: &Rc<Self>, views: &[Rc<OutputView>], scene: &Scene) {
        for view in views {
            view.reset();
        }

        for item in &scene.items {
            let Some((index, slot)) = scene.slot_for(&item.path) else {
                continue;
            };
            let Some(view) = views.get(index) else {
                continue;
            };
            let button = make_icon_button(
                item,
                slot,
                index,
                scene.icon_size,
                self.config.font_size,
                self.config.single_click,
                Rc::downgrade(self),
            );
            view.fixed().put(&button, slot.x as f64, slot.y as f64);
        }

        let full_input = self.config.drop_target == DropTargetMode::Full;
        for (index, view) in views.iter().enumerate() {
            let Some(grid) = scene.grids.get(index) else {
                continue;
            };
            let cells = scene
                .assignments
                .values()
                .filter(|assignment| assignment.output == index)
                .filter_map(|assignment| grid.slot(assignment.column, assignment.row))
                .collect();
            view.set_cells(cells);
            view.prepare(full_input, grid);
        }
    }

    fn pinned_placements(&self, items: &[DesktopItem]) -> HashMap<PathBuf, Placement> {
        let state = self.state.borrow();
        items
            .iter()
            .filter_map(|item| {
                state
                    .placement(&item.path)
                    .cloned()
                    .map(|placement| (item.path.clone(), placement))
            })
            .collect()
    }

    /// Persist the position store, warning instead of failing the UI.
    pub(super) fn save_state(&self) {
        if let Err(error) = self.state.borrow().save() {
            warn!("cannot save icon positions: {error}");
        }
    }

    pub(super) fn select_item(
        &self,
        item: DesktopItem,
        button: gtk::Button,
        output: usize,
        column: u32,
        row: u32,
    ) {
        if let Some(previous) = self.selected.borrow_mut().take() {
            previous.button.remove_css_class("selected");
        }
        button.add_css_class("selected");
        *self.selected.borrow_mut() = Some(Selection {
            item,
            button: button.clone(),
            output,
            column,
            row,
        });

        if let Some(view) = self.views.borrow().get(output) {
            // Layer-shell keyboard focus is OnDemand.  Presenting the surface
            // after a click is what makes Delete/F2/Enter work afterwards.
            view.focus();
            button.grab_focus();
        }
    }

    pub(super) fn clear_selection(&self) {
        if let Some(selection) = self.selected.borrow_mut().take() {
            selection.button.remove_css_class("selected");
        }
    }

    pub(super) fn selected_output(&self) -> Option<usize> {
        self.selected
            .borrow()
            .as_ref()
            .map(|selection| selection.output)
    }

    /// Return focus to the selected icon after an overlay closes.
    pub(super) fn restore_selection_focus(&self) {
        let selected = self.selected.borrow();
        let Some(selection) = selected.as_ref() else {
            return;
        };
        if let Some(view) = self.views.borrow().get(selection.output) {
            view.focus();
            selection.button.grab_focus();
        }
    }

    pub(super) fn handle_key(
        self: &Rc<Self>,
        key: gdk::Key,
        state: gdk::ModifierType,
    ) -> glib::Propagation {
        if self.overlay.borrow().is_some() {
            if key == gdk::Key::Escape {
                self.close_overlay();
                return glib::Propagation::Stop;
            }
            return glib::Propagation::Proceed;
        }

        let Some(selection) = self.selected.borrow().clone() else {
            return glib::Propagation::Proceed;
        };

        match key {
            gdk::Key::Escape => {
                self.clear_selection();
                glib::Propagation::Stop
            }
            gdk::Key::Delete | gdk::Key::BackSpace => {
                self.trash_item(&selection.item);
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter => {
                if let Err(error) = selection.item.open() {
                    warn!("{error}");
                }
                glib::Propagation::Stop
            }
            gdk::Key::F2 => {
                self.open_rename_modal(selection.item.clone());
                glib::Propagation::Stop
            }
            gdk::Key::Menu => {
                self.open_context_menu_for_selection();
                glib::Propagation::Stop
            }
            gdk::Key::F10 if state.contains(gdk::ModifierType::SHIFT_MASK) => {
                self.open_context_menu_for_selection();
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    }

    fn open_context_menu_for_selection(self: &Rc<Self>) {
        let Some(selection) = self.selected.borrow().clone() else {
            return;
        };
        let slot = {
            let scene = self.scene.borrow();
            scene
                .as_ref()
                .and_then(|scene| scene.grids.get(selection.output).cloned())
                .and_then(|grid| grid.slot(selection.column, selection.row))
        };
        let Some(slot) = slot else {
            return;
        };
        self.open_context_menu(
            selection.item,
            selection.output,
            slot.x as f64 + 24.0,
            slot.y as f64 + 24.0,
        );
    }
}
