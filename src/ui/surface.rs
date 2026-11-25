//! One output's layer surfaces, its input regions, and the drop outline.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gtk::gdk;
use gtk::gdk::prelude::*;
use gtk::glib::Propagation;
use gtk::prelude::*;
use gtk4_layer_shell as layer_shell;
use gtk4_layer_shell::LayerShell as LayerShellExt;

use crate::layout::{OutputGrid, OutputSpec, Slot};
use crate::scene::OutputInfo;

use super::DesktopHost;

/// What changed while polling one view.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct OutputPoll {
    /// Icons need to be laid out again (panel reservation changed).
    pub(super) rerender: bool,
    /// The set of outputs changed shape; rebuild all surfaces.
    pub(super) rebuild: bool,
}

pub(super) struct OutputView {
    connector: String,
    monitor: gdk::Monitor,
    geometry: gdk::Rectangle,
    scale_factor: i32,
    actual: gtk::Window,
    fixed: gtk::Fixed,
    probe: gtk::Window,
    cells: RefCell<Vec<Slot>>,
    input_dirty: Cell<bool>,
    probe_region_dirty: Cell<bool>,
    last_actual_size: Cell<(i32, i32)>,
    last_probe_size: Cell<(i32, i32)>,
    drop_indicator: RefCell<Option<gtk::Box>>,
    overlay_open: Cell<bool>,
    full_input: Cell<bool>,
}

impl OutputView {
    pub(super) fn new(
        app: &gtk::Application,
        monitor: &gdk::Monitor,
        connector: String,
        host: Weak<DesktopHost>,
    ) -> Rc<Self> {
        let geometry = monitor.geometry();
        let scale_factor = monitor.scale_factor();

        // The probe asks the compositor for the available area (output minus
        // exclusive zones).  We use the difference to keep icons clear of
        // panels, whatever edge they are on.
        let probe = gtk::Window::builder()
            .application(app)
            .decorated(false)
            .can_focus(false)
            .build();
        probe.add_css_class("tessera-probe");
        probe.init_layer_shell();
        probe.set_namespace(Some("tessera-inset"));
        probe.set_layer(layer_shell::Layer::Bottom);
        anchor_all(&probe);
        probe.set_exclusive_zone(0);
        probe.set_keyboard_mode(layer_shell::KeyboardMode::None);
        probe.set_monitor(Some(monitor));
        let empty = gtk::Box::new(gtk::Orientation::Vertical, 0);
        probe.set_child(Some(&empty));
        probe.set_opacity(0.0);
        let probe_for_map = probe.clone();
        probe.connect_map(move |_| {
            if let Some(surface) = probe_for_map.surface() {
                set_input_region(&surface, &[]);
            }
        });
        probe.present();

        // The icon surface covers the entire output.  exclusive_zone -1 means
        // "do not avoid panels", which makes its origin exactly the output
        // origin and therefore makes global drag coordinates exact, including
        // while crossing monitors.
        let actual = gtk::Window::builder()
            .application(app)
            .decorated(false)
            .can_focus(true)
            .build();
        actual.add_css_class("tessera-window");
        actual.init_layer_shell();
        actual.set_namespace(Some("tessera"));
        actual.set_layer(layer_shell::Layer::Bottom);
        anchor_all(&actual);
        actual.set_exclusive_zone(-1);
        actual.set_keyboard_mode(layer_shell::KeyboardMode::OnDemand);
        actual.set_monitor(Some(monitor));

        let fixed = gtk::Fixed::new();
        fixed.set_hexpand(true);
        fixed.set_vexpand(true);
        actual.set_child(Some(&fixed));

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak_host = host.clone();
        keys.connect_key_pressed(move |_, key, _, state| {
            if let Some(host) = weak_host.upgrade() {
                host.handle_key(key, state)
            } else {
                Propagation::Proceed
            }
        });
        actual.add_controller(keys);

        let weak_for_drop = host.clone();
        let connector_for_drop = connector.clone();
        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        drop.connect_drop(move |_, value, x, y| {
            let Ok(files) = value.get::<gdk::FileList>() else {
                return false;
            };
            let files = files.files();
            if files.is_empty() {
                return false;
            }
            if let Some(host) = weak_for_drop.upgrade() {
                host.accept_drop(&connector_for_drop, files, x, y);
                true
            } else {
                false
            }
        });
        actual.add_controller(drop);

        let weak_for_clear = host;
        let clear = gtk::GestureClick::new();
        clear.set_button(1);
        clear.connect_pressed(move |_, _, _, _| {
            if let Some(host) = weak_for_clear.upgrade() {
                host.clear_selection();
            }
        });
        fixed.add_controller(clear);

        actual.present();

        Rc::new(Self {
            connector,
            monitor: monitor.clone(),
            geometry,
            scale_factor,
            actual,
            fixed,
            probe,
            cells: RefCell::new(Vec::new()),
            input_dirty: Cell::new(true),
            probe_region_dirty: Cell::new(true),
            last_actual_size: Cell::new((0, 0)),
            last_probe_size: Cell::new((0, 0)),
            drop_indicator: RefCell::new(None),
            overlay_open: Cell::new(false),
            full_input: Cell::new(false),
        })
    }

    pub(super) fn connector(&self) -> &str {
        &self.connector
    }

    pub(super) fn fixed(&self) -> &gtk::Fixed {
        &self.fixed
    }

    pub(super) fn origin(&self) -> (i32, i32) {
        (self.geometry.x(), self.geometry.y())
    }

    pub(super) fn size(&self) -> (i32, i32) {
        (self.geometry.width(), self.geometry.height())
    }

    pub(super) fn output_info(&self) -> OutputInfo {
        OutputInfo {
            connector: self.connector.clone(),
            spec: self.output_spec(),
            origin: self.origin(),
        }
    }

    /// Give the icon surface keyboard focus for on-demand layer-shell focus.
    pub(super) fn focus(&self) {
        self.actual.present();
    }

    pub(super) fn set_cells(&self, cells: Vec<Slot>) {
        *self.cells.borrow_mut() = cells;
    }

    pub(super) fn set_overlay_open(&self, open: bool) {
        self.overlay_open.set(open);
        self.apply_input_region();
    }

    /// Remove every widget and reset the per-render bookkeeping.
    pub(super) fn reset(&self) {
        while let Some(child) = self.fixed.first_child() {
            self.fixed.remove(&child);
        }
        *self.drop_indicator.borrow_mut() = None;
        self.cells.borrow_mut().clear();
    }

    /// Apply this render's input mode and install the drop outline.
    pub(super) fn prepare(&self, full_input: bool, grid: &OutputGrid) {
        self.full_input.set(full_input);

        let indicator = gtk::Box::new(gtk::Orientation::Vertical, 0);
        indicator.add_css_class("tessera-drop");
        indicator.set_can_target(false);
        indicator.set_visible(false);
        indicator.set_size_request(grid.metrics.cell_width, grid.metrics.cell_height);
        self.fixed.put(&indicator, 0.0, 0.0);
        *self.drop_indicator.borrow_mut() = Some(indicator);

        self.input_dirty.set(true);
        self.apply_input_region();
    }

    pub(super) fn show_drop_target(&self, slot: Slot) {
        if let Some(indicator) = self.drop_indicator.borrow().as_ref() {
            self.fixed.move_(indicator, slot.x as f64, slot.y as f64);
            indicator.set_size_request(slot.width, slot.height);
            indicator.set_visible(true);
        }
    }

    pub(super) fn hide_drop_target(&self) {
        if let Some(indicator) = self.drop_indicator.borrow().as_ref() {
            indicator.set_visible(false);
        }
    }

    /// Apply pending region updates and report what the host needs to redo.
    pub(super) fn poll(&self) -> OutputPoll {
        let mut poll = OutputPoll::default();

        if self.probe_region_dirty.get() {
            self.apply_probe_region();
        }
        if self.input_dirty.get() {
            self.apply_input_region();
        }

        let probe_size = self.probe_size().unwrap_or((0, 0));
        if probe_size != self.last_probe_size.get() {
            self.last_probe_size.set(probe_size);
            self.probe_region_dirty.set(true);
            if probe_size.0 > 1 {
                poll.rerender = true;
            }
        }

        let actual_size = self.actual_size().unwrap_or((0, 0));
        if actual_size != self.last_actual_size.get() {
            self.last_actual_size.set(actual_size);
            self.input_dirty.set(true);
            self.apply_input_region();
        }

        let geometry = self.monitor.geometry();
        if geometry.x() != self.geometry.x()
            || geometry.y() != self.geometry.y()
            || geometry.width() != self.geometry.width()
            || geometry.height() != self.geometry.height()
            || self.monitor.scale_factor() != self.scale_factor
        {
            poll.rebuild = true;
        }

        poll
    }

    pub(super) fn close(&self) {
        self.actual.close();
        self.probe.close();
    }

    fn probe_size(&self) -> Option<(i32, i32)> {
        let surface = self.probe.surface()?;
        let size = (surface.width(), surface.height());
        (size.0 > 1 && size.1 > 1).then_some(size)
    }

    fn actual_size(&self) -> Option<(i32, i32)> {
        let surface = self.actual.surface()?;
        let size = (surface.width(), surface.height());
        (size.0 > 1 && size.1 > 1).then_some(size)
    }

    fn output_spec(&self) -> OutputSpec {
        let (probe_w, probe_h) = self
            .probe_size()
            .unwrap_or((self.geometry.width(), self.geometry.height()));
        OutputSpec {
            width: self.geometry.width(),
            height: self.geometry.height(),
            reserved_x: (self.geometry.width() - probe_w).max(0),
            reserved_y: (self.geometry.height() - probe_h).max(0),
        }
    }

    fn apply_input_region(&self) {
        let Some(surface) = self.actual.surface() else {
            return;
        };
        if self.overlay_open.get() || self.full_input.get() {
            set_full_input_region(&surface);
        } else {
            set_input_region(&surface, &self.cells.borrow());
        }
        self.input_dirty.set(false);
    }

    fn apply_probe_region(&self) {
        let Some(surface) = self.probe.surface() else {
            return;
        };
        set_input_region(&surface, &[]);
        self.probe_region_dirty.set(false);
    }
}

fn set_full_input_region(surface: &gdk::Surface) {
    let rectangle = cairo::RectangleInt::new(0, 0, surface.width(), surface.height());
    let region = cairo::Region::create_rectangle(&rectangle);
    surface.set_input_region(Some(&region));
}

fn set_input_region(surface: &gdk::Surface, cells: &[Slot]) {
    let rectangles: Vec<cairo::RectangleInt> = cells
        .iter()
        .map(|cell| cairo::RectangleInt::new(cell.x, cell.y, cell.width, cell.height))
        .collect();
    let region = if rectangles.is_empty() {
        cairo::Region::create()
    } else {
        cairo::Region::create_rectangles(&rectangles)
    };
    surface.set_input_region(Some(&region));
}

fn anchor_all(window: &gtk::Window) {
    for edge in [
        layer_shell::Edge::Top,
        layer_shell::Edge::Bottom,
        layer_shell::Edge::Left,
        layer_shell::Edge::Right,
    ] {
        window.set_anchor(edge, true);
    }
}
