//! The layer-shell desktop surfaces and drag handling.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant, UNIX_EPOCH};

use glib::Propagation;
use gtk::gdk;
use gtk::gdk::prelude::*;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk4_layer_shell as layer_shell;
use gtk4_layer_shell::LayerShell as LayerShellExt;
use log::{debug, warn};

use crate::assignment::assign_items;
use crate::config::{Config, DropTargetMode};
use crate::desktop::{self, DesktopItem};
use crate::layout::{OutputGrid, OutputSpec, Slot, compact_spec};
use crate::state::{Placement, StateStore};

const CSS: &str = r#"
window.tessera-window {
    background-color: transparent;
    color: white;
}

button.tessera-icon {
    background-color: transparent;
    background-image: none;
    border: 1px solid transparent;
    border-radius: 16px;
    box-shadow: none;
    padding: 4px;
    min-width: 0;
    min-height: 0;
    transition: background-color 120ms ease, border-color 120ms ease, opacity 100ms ease;
}

button.tessera-icon:hover {
    background-color: rgba(255, 255, 255, 0.11);
    border-color: rgba(255, 255, 255, 0.10);
}

button.tessera-icon:active {
    background-color: rgba(255, 255, 255, 0.18);
}

button.tessera-icon.selected {
    background-color: rgba(255, 255, 255, 0.20);
    border-color: rgba(255, 255, 255, 0.36);
}

button.tessera-icon.dragging {
    opacity: 0.32;
}

button.tessera-icon:focus {
    outline: none;
}

button.tessera-icon label {
    color: #ffffff;
    font-size: 13px;
    font-weight: 500;
    text-shadow: 0 1px 3px rgba(0, 0, 0, 0.95),
                 0 0 7px rgba(0, 0, 0, 0.55);
}

.tessera-drop {
    background-color: rgba(120, 170, 255, 0.20);
    border: 2px solid rgba(190, 215, 255, 0.80);
    border-radius: 16px;
}

.tessera-scrim {
    background-color: rgba(0, 0, 0, 0.16);
}

.tessera-menu,
.tessera-dialog {
    background-color: rgba(27, 29, 35, 0.97);
    border: 1px solid rgba(255, 255, 255, 0.16);
    border-radius: 12px;
    padding: 7px;
    box-shadow: 0 10px 32px rgba(0, 0, 0, 0.55);
    color: #ffffff;
}

.tessera-menu-title {
    color: rgba(255, 255, 255, 0.62);
    font-size: 11px;
    font-weight: 600;
    padding: 5px 10px 4px;
}

.tessera-dialog-title {
    font-size: 15px;
    font-weight: 600;
    padding: 4px 4px 8px;
}

button.tessera-menu-item {
    background-color: transparent;
    background-image: none;
    border: none;
    box-shadow: none;
    border-radius: 8px;
    padding: 7px 10px;
    min-height: 0;
}

button.tessera-menu-item:hover {
    background-color: rgba(255, 255, 255, 0.12);
}

button.tessera-menu-item label {
    color: #ffffff;
}

button.tessera-primary {
    background-color: rgba(110, 160, 255, 0.72);
    border: none;
    border-radius: 8px;
    padding: 6px 14px;
    color: #ffffff;
}

button.tessera-primary:hover {
    background-color: rgba(130, 176, 255, 0.88);
}

entry.tessera-entry {
    background-color: rgba(255, 255, 255, 0.10);
    border: 1px solid rgba(255, 255, 255, 0.20);
    border-radius: 8px;
    padding: 6px 8px;
    color: #ffffff;
}

label.tessera-error {
    color: #ffb4b4;
    font-size: 12px;
}

label.tessera-property-key {
    color: rgba(255, 255, 255, 0.55);
    font-size: 12px;
}

label.tessera-property-value {
    color: #ffffff;
    font-size: 12px;
}
"#;

struct OutputView {
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
    fn new(
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

    fn contains_global(&self, x: f64, y: f64) -> bool {
        let gx = self.geometry.x() as f64;
        let gy = self.geometry.y() as f64;
        x >= gx
            && y >= gy
            && x < gx + self.geometry.width() as f64
            && y < gy + self.geometry.height() as f64
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

    fn show_drop_target(&self, slot: Slot) {
        if let Some(indicator) = self.drop_indicator.borrow().as_ref() {
            self.fixed.move_(indicator, slot.x as f64, slot.y as f64);
            indicator.set_size_request(slot.width, slot.height);
            indicator.set_visible(true);
        }
    }

    fn hide_drop_target(&self) {
        if let Some(indicator) = self.drop_indicator.borrow().as_ref() {
            indicator.set_visible(false);
        }
    }

    fn close(&self) {
        self.actual.close();
        self.probe.close();
    }
}

struct Rendered {
    grids: Vec<OutputGrid>,
    occupants: Vec<HashMap<(u32, u32), PathBuf>>,
}

struct DragState {
    path: PathBuf,
    source_view: usize,
    pointer_start: (f64, f64),
    source_button: gtk::Button,
    suppress_until: Rc<Cell<Option<Instant>>>,
    target: Option<(usize, u32, u32)>,
}

#[derive(Clone)]
struct Selection {
    item: DesktopItem,
    button: gtk::Button,
    output: usize,
    column: u32,
    row: u32,
}

struct OverlayState {
    output: usize,
    scrim: gtk::Box,
    card: gtk::Box,
}

pub struct DesktopHost {
    app: gtk::Application,
    config: Config,
    preferred_output: Option<String>,
    desktop_dir: PathBuf,
    state: RefCell<StateStore>,
    views: RefCell<Vec<Rc<OutputView>>>,
    rendered: RefCell<Option<Rendered>>,
    selected: Rc<RefCell<Option<Selection>>>,
    overlay: RefCell<Option<OverlayState>>,
    file_monitor: RefCell<Option<gio::FileMonitor>>,
    reload_source: Rc<RefCell<Option<glib::SourceId>>>,
    rebuild_source: RefCell<Option<glib::SourceId>>,
    drag: Rc<RefCell<Option<DragState>>>,
    _poller: RefCell<Option<glib::SourceId>>,
    _hold: gio::ApplicationHoldGuard,
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
            rendered: RefCell::new(None),
            selected: Rc::new(RefCell::new(None)),
            overlay: RefCell::new(None),
            file_monitor: RefCell::new(None),
            reload_source: Rc::new(RefCell::new(None)),
            rebuild_source: RefCell::new(None),
            drag: Rc::new(RefCell::new(None)),
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

        Self::install_css();
        host.install_file_monitor();
        host.install_display_listener();
        host.rebuild_outputs();
        host
    }

    fn install_css() {
        let display = gdk::Display::default().expect("a GDK display is required");
        let provider = gtk::CssProvider::new();
        provider.load_from_data(CSS);
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
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
        *self.rendered.borrow_mut() = None;
        *self.drag.borrow_mut() = None;
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
            if view.probe_region_dirty.get() {
                view.apply_probe_region();
            }
            if view.input_dirty.get() {
                view.apply_input_region();
            }

            let probe_size = view.probe_size().unwrap_or((0, 0));
            if probe_size != view.last_probe_size.get() {
                view.last_probe_size.set(probe_size);
                view.probe_region_dirty.set(true);
                if probe_size.0 > 1 {
                    rerender = true;
                }
            }

            let actual_size = view.actual_size().unwrap_or((0, 0));
            if actual_size != view.last_actual_size.get() {
                view.last_actual_size.set(actual_size);
                view.input_dirty.set(true);
                view.apply_input_region();
            }

            let geometry = view.monitor.geometry();
            if geometry.x() != view.geometry.x()
                || geometry.y() != view.geometry.y()
                || geometry.width() != view.geometry.width()
                || geometry.height() != view.geometry.height()
                || view.monitor.scale_factor() != view.scale_factor
            {
                rebuild = true;
            }
        }

        if rebuild {
            self.schedule_rebuild();
        } else if rerender {
            self.render();
        }
    }

    fn render(self: &Rc<Self>) {
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

        let items = desktop::scan(&self.desktop_dir);
        let specs: Vec<OutputSpec> = views.iter().map(|view| view.output_spec()).collect();
        let spec = compact_spec(
            &specs,
            self.config.grid_spec(),
            self.config.corner,
            items.len(),
        );
        let grids: Vec<OutputGrid> = specs
            .iter()
            .map(|output| OutputGrid::new(*output, spec, self.config.corner))
            .collect();
        let connectors: Vec<String> = views.iter().map(|view| view.connector.clone()).collect();

        let preferred = self
            .preferred_output
            .as_deref()
            .and_then(|wanted| {
                connectors
                    .iter()
                    .position(|connector| connector.eq_ignore_ascii_case(wanted))
            })
            .or_else(|| {
                views
                    .iter()
                    .position(|view| view.geometry.x() == 0 && view.geometry.y() == 0)
            })
            .unwrap_or(0);
        let mut output_order: Vec<usize> = (0..views.len()).collect();
        output_order.retain(|index| *index != preferred);
        output_order.insert(0, preferred);

        let paths: Vec<PathBuf> = items.iter().map(|item| item.path.clone()).collect();
        let pinned: HashMap<PathBuf, Placement> = {
            let state = self.state.borrow();
            paths
                .iter()
                .filter_map(|path| {
                    state
                        .placement(path)
                        .cloned()
                        .map(|placement| (path.clone(), placement))
                })
                .collect()
        };
        let (assignments, hidden) =
            assign_items(&paths, &connectors, &grids, &pinned, &output_order);
        if hidden > 0 {
            warn!("{hidden} desktop item(s) do not fit on any output");
        }

        let mut occupants: Vec<HashMap<(u32, u32), PathBuf>> =
            grids.iter().map(|_| HashMap::new()).collect();
        for (path, assignment) in &assignments {
            occupants[assignment.output].insert((assignment.column, assignment.row), path.clone());
        }

        self.clear_selection();
        for view in &views {
            while let Some(child) = view.fixed.first_child() {
                view.fixed.remove(&child);
            }
            *view.drop_indicator.borrow_mut() = None;
            view.cells.borrow_mut().clear();
        }

        for item in items {
            let Some(assignment) = assignments.get(&item.path) else {
                continue;
            };
            let Some(grid) = grids.get(assignment.output) else {
                continue;
            };
            let Some(slot) = grid.slot(assignment.column, assignment.row) else {
                continue;
            };
            let view = &views[assignment.output];
            let button = make_icon_button(
                &item,
                slot,
                assignment.output,
                grid.metrics.icon_size,
                self.config.font_size,
                self.config.single_click,
                Rc::downgrade(self),
            );
            view.fixed.put(&button, slot.x as f64, slot.y as f64);
            view.cells.borrow_mut().push(slot);
        }

        for (view, grid) in views.iter().zip(grids.iter()) {
            view.full_input
                .set(self.config.drop_target == DropTargetMode::Full);
            let indicator = gtk::Box::new(gtk::Orientation::Vertical, 0);
            indicator.add_css_class("tessera-drop");
            indicator.set_can_target(false);
            indicator.set_visible(false);
            indicator.set_size_request(grid.metrics.cell_width, grid.metrics.cell_height);
            view.fixed.put(&indicator, 0.0, 0.0);
            *view.drop_indicator.borrow_mut() = Some(indicator);
            view.input_dirty.set(true);
            view.apply_input_region();
        }

        *self.rendered.borrow_mut() = Some(Rendered { grids, occupants });
        debug!(
            "rendered {} items on {} output(s), icon size {}",
            assignments.len(),
            views.len(),
            spec.icon_size
        );
    }

    fn begin_drag(
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

    fn update_drag(self: &Rc<Self>, offset_x: f64, offset_y: f64) {
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
        let global_x = source.geometry.x() as f64 + pointer_start.0 + offset_x;
        let global_y = source.geometry.y() as f64 + pointer_start.1 + offset_y;

        let target = {
            let rendered = self.rendered.borrow();
            let Some(rendered) = rendered.as_ref() else {
                return;
            };
            let views = self.views.borrow();
            find_drop_target(&views, rendered, global_x, global_y, &path)
        };

        if let Some(drag) = self.drag.borrow_mut().as_mut() {
            drag.target = target;
        }

        let views = self.views.borrow().clone();
        for view in &views {
            view.hide_drop_target();
        }
        if let Some((index, column, row)) = target {
            let rendered = self.rendered.borrow();
            if let Some(grid) = rendered
                .as_ref()
                .and_then(|rendered| rendered.grids.get(index))
                && let Some(slot) = grid.slot(column, row)
                && let Some(view) = views.get(index)
            {
                view.show_drop_target(slot);
            }
        }
    }

    fn end_drag(self: &Rc<Self>) {
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
                .map(|view| view.connector.clone());
            if let Some(connector) = connector {
                let mut state = self.state.borrow_mut();
                state.set(
                    drag.path,
                    Placement {
                        output: connector,
                        column,
                        row,
                    },
                );
                if let Err(error) = state.save() {
                    warn!("cannot save icon positions: {error}");
                }
            }
        }

        self.render();
    }

    fn cancel_drag(self: &Rc<Self>) {
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

    fn select_item(
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
            view.actual.present();
            button.grab_focus();
        }
    }

    fn clear_selection(&self) {
        if let Some(selection) = self.selected.borrow_mut().take() {
            selection.button.remove_css_class("selected");
        }
    }

    fn handle_key(self: &Rc<Self>, key: gdk::Key, state: gdk::ModifierType) -> Propagation {
        if self.overlay.borrow().is_some() {
            if key == gdk::Key::Escape {
                self.close_overlay();
                return Propagation::Stop;
            }
            return Propagation::Proceed;
        }

        let Some(selection) = self.selected.borrow().clone() else {
            return Propagation::Proceed;
        };

        match key {
            gdk::Key::Escape => {
                self.clear_selection();
                Propagation::Stop
            }
            gdk::Key::Delete | gdk::Key::BackSpace => {
                self.trash_item(&selection.item);
                Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter => {
                if let Err(error) = selection.item.open() {
                    warn!("{error}");
                }
                Propagation::Stop
            }
            gdk::Key::F2 => {
                self.open_rename_modal(selection.item.clone());
                Propagation::Stop
            }
            gdk::Key::Menu => {
                self.open_context_menu_for_selection();
                Propagation::Stop
            }
            gdk::Key::F10 if state.contains(gdk::ModifierType::SHIFT_MASK) => {
                self.open_context_menu_for_selection();
                Propagation::Stop
            }
            _ => Propagation::Proceed,
        }
    }

    fn open_context_menu_for_selection(self: &Rc<Self>) {
        let Some(selection) = self.selected.borrow().clone() else {
            return;
        };
        let rendered = self.rendered.borrow();
        let Some(grid) = rendered
            .as_ref()
            .and_then(|rendered| rendered.grids.get(selection.output).cloned())
        else {
            return;
        };
        let Some(slot) = grid.slot(selection.column, selection.row) else {
            return;
        };
        drop(rendered);
        self.open_context_menu(
            selection.item,
            selection.output,
            slot.x as f64 + 24.0,
            slot.y as f64 + 24.0,
        );
    }

    fn trash_item(self: &Rc<Self>, item: &DesktopItem) {
        let path = item.path.clone();
        let file = gio::File::for_path(&path);
        let weak = Rc::downgrade(self);
        file.trash_async(
            glib::Priority::DEFAULT,
            None::<&gio::Cancellable>,
            move |result| match result {
                Ok(()) => {
                    if let Some(host) = weak.upgrade() {
                        {
                            let mut state = host.state.borrow_mut();
                            state.remove(&path);
                            if let Err(error) = state.save() {
                                warn!("cannot save icon positions: {error}");
                            }
                        }
                        host.clear_selection();
                        host.render();
                    }
                }
                Err(error) => warn!("cannot move {} to Trash: {error}", path.display()),
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn begin_overlay(
        self: &Rc<Self>,
        output: usize,
        width: i32,
        height: i32,
        desired_x: f64,
        desired_y: f64,
        centered: bool,
        class: &str,
    ) -> Option<gtk::Box> {
        self.close_overlay();
        let view = self.views.borrow().get(output).cloned()?;
        let surface_width = view.geometry.width();
        let surface_height = view.geometry.height();

        let scrim = gtk::Box::new(gtk::Orientation::Vertical, 0);
        scrim.add_css_class("tessera-scrim");
        scrim.set_size_request(surface_width, surface_height);
        let weak = Rc::downgrade(self);
        let click = gtk::GestureClick::new();
        click.connect_pressed(move |_, _, _, _| {
            if let Some(host) = weak.upgrade() {
                host.close_overlay();
            }
        });
        scrim.add_controller(click);
        view.fixed.put(&scrim, 0.0, 0.0);

        let card = gtk::Box::new(gtk::Orientation::Vertical, 2);
        card.add_css_class(class);
        card.set_size_request(width, height);
        card.set_halign(gtk::Align::Start);
        card.set_valign(gtk::Align::Start);
        view.fixed.put(&card, desired_x, desired_y);
        view.overlay_open.set(true);
        view.apply_input_region();

        *self.overlay.borrow_mut() = Some(OverlayState {
            output,
            scrim,
            card: card.clone(),
        });

        let fixed = view.fixed.clone();
        let card_for_position = card.clone();
        glib::idle_add_local_once(move || {
            let card_width = card_for_position.width().max(1);
            let card_height = card_for_position.height().max(1);
            let x = if centered {
                (surface_width - card_width) / 2
            } else {
                (desired_x as i32)
                    .min(surface_width - card_width - 8)
                    .max(8)
            };
            let y = if centered {
                (surface_height - card_height) / 2
            } else {
                (desired_y as i32)
                    .min(surface_height - card_height - 8)
                    .max(8)
            };
            fixed.move_(&card_for_position, x as f64, y as f64);
        });

        Some(card)
    }

    fn close_overlay(&self) {
        let Some(overlay) = self.overlay.borrow_mut().take() else {
            return;
        };

        {
            let views = self.views.borrow();
            if let Some(view) = views.get(overlay.output) {
                if overlay.scrim.parent().is_some() {
                    view.fixed.remove(&overlay.scrim);
                }
                if overlay.card.parent().is_some() {
                    view.fixed.remove(&overlay.card);
                }
                view.overlay_open.set(false);
                view.apply_input_region();
            }
        }

        if let Some(selection) = self.selected.borrow().as_ref()
            && let Some(view) = self.views.borrow().get(selection.output)
        {
            view.actual.present();
            selection.button.grab_focus();
        }
    }

    fn add_menu_item<F>(self: &Rc<Self>, card: &gtk::Box, text: &str, action: F)
    where
        F: Fn(&Rc<DesktopHost>) + 'static,
    {
        let label = gtk::Label::new(Some(text));
        label.set_xalign(0.0);
        label.set_halign(gtk::Align::Start);
        label.set_hexpand(true);

        let button = gtk::Button::new();
        button.add_css_class("tessera-menu-item");
        button.set_child(Some(&label));
        button.set_has_frame(false);
        button.set_size_request(-1, 31);

        let weak = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            if let Some(host) = weak.upgrade() {
                action(&host);
            }
        });
        card.append(&button);
    }

    fn open_context_menu(self: &Rc<Self>, item: DesktopItem, output: usize, x: f64, y: f64) {
        let can_open_with =
            !item.is_dir && item.content_type.as_deref() != Some("application/x-desktop");
        let rows = if can_open_with { 7 } else { 6 };
        let height = 48 + rows * 31 + 8;
        let Some(card) = self.begin_overlay(output, 238, height, x, y, false, "tessera-menu")
        else {
            return;
        };

        let title = gtk::Label::new(Some(&item.name));
        title.add_css_class("tessera-menu-title");
        title.set_xalign(0.0);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_halign(gtk::Align::Fill);
        card.append(&title);

        {
            let item = item.clone();
            self.add_menu_item(&card, "Open", move |host| {
                host.close_overlay();
                if let Err(error) = item.open() {
                    warn!("{error}");
                }
            });
        }
        if can_open_with {
            let item = item.clone();
            self.add_menu_item(&card, "Open With…", move |host| {
                host.close_overlay();
                host.open_open_with_modal(item.clone());
            });
        }
        {
            let item = item.clone();
            self.add_menu_item(&card, "Show in Files", move |host| {
                host.close_overlay();
                host.show_in_files(&item);
            });
        }
        {
            let item = item.clone();
            self.add_menu_item(&card, "Copy Path", move |host| {
                host.close_overlay();
                host.copy_path(&item);
            });
        }
        {
            let item = item.clone();
            self.add_menu_item(&card, "Rename…", move |host| {
                host.close_overlay();
                host.open_rename_modal(item.clone());
            });
        }
        {
            let item = item.clone();
            self.add_menu_item(&card, "Move to Trash", move |host| {
                host.close_overlay();
                host.trash_item(&item);
            });
        }
        {
            let item = item.clone();
            self.add_menu_item(&card, "Properties", move |host| {
                host.close_overlay();
                host.open_properties_modal(item.clone());
            });
        }
    }

    fn show_in_files(&self, item: &DesktopItem) {
        let Some(parent) = item.path.parent() else {
            return;
        };
        let uri = gio::File::for_path(parent).uri().to_string();
        if let Err(error) =
            gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>)
        {
            warn!("cannot open {}: {error}", parent.display());
        }
    }

    fn copy_path(&self, item: &DesktopItem) {
        if let Some(display) = gdk::Display::default() {
            display.clipboard().set_text(&item.path.to_string_lossy());
        }
    }

    fn rename_item(&self, item: &DesktopItem, new_name: &str) -> Result<(), String> {
        let name = new_name.trim();
        if name.is_empty() {
            return Err("The name cannot be empty.".to_owned());
        }
        if name == "." || name == ".." || name.contains('/') || name.contains('\0') {
            return Err("That is not a valid file name.".to_owned());
        }

        let parent = item
            .path
            .parent()
            .ok_or_else(|| "This item has no parent folder.".to_owned())?;
        let target = parent.join(name);
        if target == item.path {
            return Ok(());
        }
        if target.exists() {
            return Err("An item with that name already exists.".to_owned());
        }

        std::fs::rename(&item.path, &target)
            .map_err(|error| format!("Could not rename the item: {error}"))?;

        let mut state = self.state.borrow_mut();
        state.rename(&item.path, &target);
        if let Err(error) = state.save() {
            warn!("cannot save icon positions: {error}");
        }
        self.clear_selection();
        Ok(())
    }

    fn open_rename_modal(self: &Rc<Self>, item: DesktopItem) {
        let Some(output) = self
            .selected
            .borrow()
            .as_ref()
            .map(|selection| selection.output)
        else {
            return;
        };
        let Some(card) = self.begin_overlay(output, 360, -1, 0.0, 0.0, true, "tessera-dialog")
        else {
            return;
        };

        let title = gtk::Label::new(Some("Rename"));
        title.add_css_class("tessera-dialog-title");
        title.set_xalign(0.0);
        card.append(&title);

        let entry = gtk::Entry::new();
        entry.add_css_class("tessera-entry");
        entry.set_text(&item.name);
        entry.set_hexpand(true);
        card.append(&entry);

        let error_label = gtk::Label::new(None);
        error_label.add_css_class("tessera-error");
        error_label.set_xalign(0.0);
        error_label.set_wrap(true);
        error_label.set_visible(false);
        card.append(&error_label);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.set_halign(gtk::Align::End);
        actions.set_margin_top(6);
        let cancel = gtk::Button::with_label("Cancel");
        let rename = gtk::Button::with_label("Rename");
        rename.add_css_class("tessera-primary");
        actions.append(&cancel);
        actions.append(&rename);
        card.append(&actions);

        let weak = Rc::downgrade(self);
        let item_for_commit = item.clone();
        let entry_for_commit = entry.clone();
        let error_for_commit = error_label.clone();
        let commit = Rc::new(move || {
            let Some(host) = weak.upgrade() else {
                return;
            };
            match host.rename_item(&item_for_commit, entry_for_commit.text().as_str()) {
                Ok(()) => host.render(),
                Err(error) => {
                    error_for_commit.set_text(&error);
                    error_for_commit.set_visible(true);
                }
            }
        });

        {
            let commit = Rc::clone(&commit);
            rename.connect_clicked(move |_| commit());
        }
        {
            let commit = Rc::clone(&commit);
            entry.connect_activate(move |_| commit());
        }
        {
            let weak = Rc::downgrade(self);
            cancel.connect_clicked(move |_| {
                if let Some(host) = weak.upgrade() {
                    host.close_overlay();
                }
            });
        }

        let stem_len = if item.is_dir {
            item.name.chars().count()
        } else {
            item.path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(|stem| stem.chars().count())
                .unwrap_or_else(|| item.name.chars().count())
        };
        let entry_for_focus = entry.clone();
        if let Some(view) = self.views.borrow().get(output) {
            view.actual.present();
        }
        glib::idle_add_local_once(move || {
            entry_for_focus.grab_focus();
            entry_for_focus.select_region(0, stem_len as i32);
        });
    }

    fn open_properties_modal(self: &Rc<Self>, item: DesktopItem) {
        let Some(output) = self
            .selected
            .borrow()
            .as_ref()
            .map(|selection| selection.output)
        else {
            return;
        };
        let Some(card) = self.begin_overlay(output, 430, -1, 0.0, 0.0, true, "tessera-dialog")
        else {
            return;
        };

        let title = gtk::Label::new(Some("Properties"));
        title.add_css_class("tessera-dialog-title");
        title.set_xalign(0.0);
        card.append(&title);

        let symlink_meta = std::fs::symlink_metadata(&item.path).ok();
        let metadata = std::fs::metadata(&item.path).ok();
        let kind = if item.content_type.as_deref() == Some("application/x-desktop") {
            "Application shortcut".to_owned()
        } else if item.is_dir {
            "Folder".to_owned()
        } else if let Some(content_type) = item.content_type.as_deref() {
            gio::content_type_get_description(content_type).to_string()
        } else {
            "File".to_owned()
        };
        let size = metadata
            .as_ref()
            .filter(|metadata| metadata.is_file())
            .map(|metadata| format_size(metadata.len()))
            .unwrap_or_else(|| "—".to_owned());
        let modified = metadata
            .as_ref()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .and_then(|duration| glib::DateTime::from_unix_local(duration.as_secs() as i64).ok())
            .and_then(|date_time| date_time.format("%Y-%m-%d %H:%M").ok())
            .map(|formatted| formatted.to_string())
            .unwrap_or_else(|| "—".to_owned());
        let symlink_target = symlink_meta
            .as_ref()
            .filter(|metadata| metadata.file_type().is_symlink())
            .and_then(|_| std::fs::read_link(&item.path).ok())
            .map(|target| target.display().to_string());

        properties_row(&card, "Name", &item.name);
        properties_row(&card, "Type", &kind);
        properties_row(&card, "Size", &size);
        properties_row(&card, "Modified", &modified);
        properties_row(&card, "Path", &item.path.to_string_lossy());
        if let Some(target) = symlink_target {
            properties_row(&card, "Link target", &target);
        }

        let close = gtk::Button::with_label("Close");
        close.set_halign(gtk::Align::End);
        close.set_margin_top(8);
        {
            let weak = Rc::downgrade(self);
            close.connect_clicked(move |_| {
                if let Some(host) = weak.upgrade() {
                    host.close_overlay();
                }
            });
        }
        card.append(&close);
    }

    fn open_open_with_modal(self: &Rc<Self>, item: DesktopItem) {
        let Some(content_type) = item.content_type.clone() else {
            return;
        };
        let Some(output) = self
            .selected
            .borrow()
            .as_ref()
            .map(|selection| selection.output)
        else {
            return;
        };
        let Some(card) = self.begin_overlay(output, 390, -1, 0.0, 0.0, true, "tessera-dialog")
        else {
            return;
        };

        let title = gtk::Label::new(Some("Open With"));
        title.add_css_class("tessera-dialog-title");
        title.set_xalign(0.0);
        card.append(&title);

        let mut seen = std::collections::HashSet::new();
        let mut apps = gio::AppInfo::recommended_for_type(&content_type);
        apps.retain(|app| app.should_show() && seen.insert(app.name().to_string()));
        apps.truncate(8);

        if apps.is_empty() {
            let empty = gtk::Label::new(Some("No applications are available for this file type."));
            empty.set_xalign(0.0);
            empty.set_wrap(true);
            card.append(&empty);
        }

        let uri = gio::File::for_path(&item.path).uri().to_string();
        for app in apps {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            if let Some(icon) = app.icon() {
                let image = gtk::Image::from_gicon(&icon);
                image.set_pixel_size(24);
                row.append(&image);
            }
            let label = gtk::Label::new(Some(app.name().as_str()));
            label.set_xalign(0.0);
            label.set_halign(gtk::Align::Start);
            label.set_hexpand(true);
            row.append(&label);

            let button = gtk::Button::new();
            button.add_css_class("tessera-menu-item");
            button.set_child(Some(&row));
            button.set_has_frame(false);
            let weak = Rc::downgrade(self);
            let uri = uri.clone();
            button.connect_clicked(move |_| {
                if let Err(error) = app.launch_uris(&[uri.as_str()], None::<&gio::AppLaunchContext>)
                {
                    warn!("cannot launch application: {error}");
                } else if let Some(host) = weak.upgrade() {
                    host.close_overlay();
                }
            });
            card.append(&button);
        }

        let close = gtk::Button::with_label("Cancel");
        close.set_halign(gtk::Align::End);
        close.set_margin_top(8);
        {
            let weak = Rc::downgrade(self);
            close.connect_clicked(move |_| {
                if let Some(host) = weak.upgrade() {
                    host.close_overlay();
                }
            });
        }
        card.append(&close);
    }

    fn accept_drop(self: &Rc<Self>, connector: &str, files: Vec<gio::File>, x: f64, y: f64) {
        let mut copied = Vec::new();
        for file in files {
            let Some(source) = file.path() else {
                warn!("ignoring non-local drop: {}", file.uri());
                continue;
            };
            let Some(name) = source.file_name() else {
                continue;
            };
            let destination = unique_destination(&self.desktop_dir, name);
            match copy_path(&source, &destination) {
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

        let mut placements = Vec::new();
        {
            let rendered = self.rendered.borrow();
            if let Some(rendered) = rendered.as_ref()
                && let Some(index) = self
                    .views
                    .borrow()
                    .iter()
                    .position(|view| view.connector.eq_ignore_ascii_case(connector))
                && let Some(grid) = rendered.grids.get(index)
                && let Some(occupied) = rendered.occupants.get(index)
            {
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
                for (path, slot) in copied.iter().zip(free.iter()) {
                    placements.push((
                        path.clone(),
                        Placement {
                            output: connector.to_owned(),
                            column: slot.column,
                            row: slot.row,
                        },
                    ));
                }
            }
        }

        if !placements.is_empty() {
            let mut state = self.state.borrow_mut();
            for (path, placement) in placements {
                state.set(path, placement);
            }
            if let Err(error) = state.save() {
                warn!("cannot save icon positions: {error}");
            }
        }

        self.render();
    }
}

fn find_drop_target(
    views: &[Rc<OutputView>],
    rendered: &Rendered,
    global_x: f64,
    global_y: f64,
    dragged: &Path,
) -> Option<(usize, u32, u32)> {
    let index = views
        .iter()
        .position(|view| view.contains_global(global_x, global_y))?;
    let view = views.get(index)?;
    let grid = rendered.grids.get(index)?;
    let occupied = rendered.occupants.get(index)?;

    let local_x = (global_x - view.geometry.x() as f64) as i32;
    let local_y = (global_y - view.geometry.y() as f64) as i32;
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

#[allow(clippy::too_many_arguments)]
fn make_icon_button(
    item: &DesktopItem,
    slot: Slot,
    source_view: usize,
    icon_size: i32,
    font_size: u32,
    single_click: bool,
    host: Weak<DesktopHost>,
) -> gtk::Button {
    let image = gtk::Image::from_gicon(&item.icon);
    image.set_pixel_size(icon_size);
    image.set_size_request(icon_size, icon_size);
    image.set_halign(gtk::Align::Center);

    let label = gtk::Label::new(Some(&item.name));
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_lines(2);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_justify(gtk::Justification::Center);
    label.set_xalign(0.5);
    label.set_halign(gtk::Align::Fill);
    label.set_hexpand(true);
    label.add_css_class("tessera-label");
    label.set_attributes(Some(&label_attributes(font_size)));

    let content = gtk::Box::new(gtk::Orientation::Vertical, 5);
    content.set_halign(gtk::Align::Fill);
    content.set_valign(gtk::Align::Center);
    content.append(&image);
    content.append(&label);

    let button = gtk::Button::new();
    button.add_css_class("tessera-icon");
    button.set_has_frame(false);
    button.set_child(Some(&content));
    button.set_size_request(slot.width, slot.height);

    let item = item.clone();
    let button_ref = button.clone();
    let last_click = Rc::new(Cell::new(None::<Instant>));
    let suppress_until = Rc::new(Cell::new(None::<Instant>));
    let host_for_click = host.clone();
    let item_for_click = item.clone();
    let item_for_menu = item.clone();
    let item_for_drag = item.clone();

    let suppress_for_click = Rc::clone(&suppress_until);
    button.connect_clicked(move |_| {
        if let Some(until) = suppress_for_click.get() {
            if Instant::now() < until {
                return;
            }
            suppress_for_click.set(None);
        }

        if let Some(host) = host_for_click.upgrade() {
            host.select_item(
                item_for_click.clone(),
                button_ref.clone(),
                source_view,
                slot.column,
                slot.row,
            );
        }

        let now = Instant::now();
        let activate = single_click
            || last_click
                .get()
                .is_some_and(|last| now.duration_since(last) <= Duration::from_millis(450));
        last_click.set(if activate { None } else { Some(now) });

        if activate && let Err(error) = item_for_click.open() {
            warn!("{error}");
        }
    });

    let drag = gtk::GestureDrag::new();
    drag.set_button(1);
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);

    let drag_start = Rc::new(Cell::new(None::<(f64, f64)>));
    let drag_started = Rc::new(Cell::new(false));
    let start_for_begin = Rc::clone(&drag_start);

    let started_for_begin = Rc::clone(&drag_started);
    drag.connect_drag_begin(move |_gesture, start_x, start_y| {
        // GtkGestureDrag starts on press.  Do not claim the sequence or begin
        // a move until the pointer has actually travelled far enough to be a
        // drag; otherwise every ordinary click would move the icon.
        start_for_begin.set(Some((start_x, start_y)));
        started_for_begin.set(false);
    });

    let weak = host.clone();
    let start_for_update = Rc::clone(&drag_start);
    let started_for_update = Rc::clone(&drag_started);
    let button_for_update = button.clone();
    let suppress_for_update = Rc::clone(&suppress_until);
    let item_for_update = item_for_drag;
    drag.connect_drag_update(move |gesture, offset_x, offset_y| {
        if !started_for_update.get() {
            if offset_x.abs() < 8.0 && offset_y.abs() < 8.0 {
                return;
            }
            // Shift+drag is reserved for exporting files through the
            // compositor's drag-and-drop protocol.
            if gesture
                .current_event_state()
                .contains(gdk::ModifierType::SHIFT_MASK)
            {
                return;
            }
            let Some(start) = start_for_update.get() else {
                return;
            };
            started_for_update.set(true);
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Some(host) = weak.upgrade() {
                host.begin_drag(
                    item_for_update.clone(),
                    source_view,
                    slot,
                    start,
                    button_for_update.clone(),
                    Rc::clone(&suppress_for_update),
                );
            }
        }

        if let Some(host) = weak.upgrade() {
            host.update_drag(offset_x, offset_y);
        }
    });

    let weak = host.clone();
    let started_for_end = Rc::clone(&drag_started);
    let start_for_end = Rc::clone(&drag_start);
    drag.connect_drag_end(move |_, _, _| {
        let was_dragging = started_for_end.replace(false);
        start_for_end.set(None);
        if was_dragging && let Some(host) = weak.upgrade() {
            host.end_drag();
        }
    });

    let weak = host.clone();
    let started_for_cancel = Rc::clone(&drag_started);
    let start_for_cancel = drag_start;
    drag.connect_cancel(move |_, _| {
        let was_dragging = started_for_cancel.replace(false);
        start_for_cancel.set(None);
        if was_dragging && let Some(host) = weak.upgrade() {
            host.cancel_drag();
        }
    });

    button.add_controller(drag);

    // Shift+drag exports the file through Wayland DND.  A plain drag stays
    // inside Tessera and rearranges the icon grid.
    let source = gtk::DragSource::new();
    source.set_actions(gdk::DragAction::COPY);
    let uri_path = item.path.clone();
    source.connect_prepare(move |source, _, _| {
        if !source
            .current_event_state()
            .contains(gdk::ModifierType::SHIFT_MASK)
        {
            return None;
        }
        let uri = gio::File::for_path(&uri_path).uri().to_string();
        let bytes = glib::Bytes::from_owned(format!("{uri}\r\n").into_bytes());
        Some(gdk::ContentProvider::for_bytes("text/uri-list", &bytes))
    });
    button.add_controller(source);

    let right_click = gtk::GestureClick::new();
    right_click.set_button(3);
    let button_ref = button.clone();
    let host_for_menu = host.clone();

    right_click.connect_pressed(move |_, _, x, y| {
        if let Some(host) = host_for_menu.upgrade() {
            host.select_item(
                item_for_menu.clone(),
                button_ref.clone(),
                source_view,
                slot.column,
                slot.row,
            );
            host.open_context_menu(
                item_for_menu.clone(),
                source_view,
                slot.x as f64 + x,
                slot.y as f64 + y,
            );
        }
    });
    button.add_controller(right_click);

    button
}

fn unique_destination(directory: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let candidate = directory.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_owned());
    let extension = path
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy()))
        .unwrap_or_default();
    for index in 1..10_000 {
        let candidate = directory.join(format!("{stem} ({index}){extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    directory.join(format!("{stem} (copy){extension}"))
}

fn copy_path(source: &Path, destination: &Path) -> std::io::Result<()> {
    let metadata = std::fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        let target = std::fs::read_link(source)?;
        std::os::unix::fs::symlink(target, destination)
    } else if metadata.is_dir() {
        std::fs::create_dir(destination)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            copy_path(&entry.path(), &destination.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(source, destination).map(|_| ())
    }
}

fn format_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn properties_row(card: &gtk::Box, key: &str, value: &str) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let key_label = gtk::Label::new(Some(key));
    key_label.add_css_class("tessera-property-key");
    key_label.set_xalign(0.0);
    key_label.set_size_request(86, -1);
    let value_label = gtk::Label::new(Some(value));
    value_label.add_css_class("tessera-property-value");
    value_label.set_xalign(0.0);
    value_label.set_halign(gtk::Align::Start);
    value_label.set_hexpand(true);
    value_label.set_wrap(true);
    value_label.set_selectable(true);
    row.append(&key_label);
    row.append(&value_label);
    card.append(&row);
}

fn label_attributes(font_size: u32) -> gtk::pango::AttrList {
    let attributes = gtk::pango::AttrList::new();
    let size = gtk::pango::AttrSize::new((font_size as i32) * gtk::pango::SCALE);
    attributes.insert(size);
    attributes
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
