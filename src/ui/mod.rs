//! The desktop surfaces, the widgets drawn on them, and the controller that
//! ties everything together.
//!
//! `DesktopHost` owns the long-lived state: the per-output views, the current
//! scene, the pinned positions, and the transient selection, drag and overlay
//! state.  Each feature lives in its own module and implements its part as
//! `impl DesktopHost` blocks:
//!
//! - `host`: lifecycle, polling, painting, keyboard handling and selection;
//! - `surface`: one output's layer surfaces and input regions;
//! - `icon`: the icon button and its pointer gestures;
//! - `drag`: rearranging icons within Tessera;
//! - `drop`: accepting files dragged in from other applications;
//! - `overlay`: the scrim and floating card shared by menus and dialogs;
//! - `menu`: the item context menu;
//! - `dialogs`: Rename, Properties and Open With;
//! - `actions`: file operations triggered from the UI;
//! - `css`: the GTK style sheet.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk::gio;
use gtk::glib;

use crate::config::Config;
use crate::scene::Scene;
use crate::state::StateStore;

use self::drag::DragState;
use self::host::Selection;
use self::overlay::OverlayState;
use self::surface::OutputView;

mod actions;
mod css;
mod dialogs;
mod drag;
mod drop;
mod host;
mod icon;
mod menu;
mod overlay;
mod surface;

/// The controller for every desktop surface in the application.
pub struct DesktopHost {
    app: gtk::Application,
    config: Config,
    /// Output the user asked to fill first; the scene falls back to the
    /// layout origin when it is unset or missing.
    preferred_output: Option<String>,
    desktop_dir: PathBuf,
    state: RefCell<StateStore>,
    views: RefCell<Vec<Rc<OutputView>>>,
    scene: RefCell<Option<Scene>>,
    selected: RefCell<Option<Selection>>,
    overlay: RefCell<Option<OverlayState>>,
    file_monitor: RefCell<Option<gio::FileMonitor>>,
    reload_source: RefCell<Option<glib::SourceId>>,
    rebuild_source: RefCell<Option<glib::SourceId>>,
    drag: RefCell<Option<DragState>>,
    _poller: RefCell<Option<glib::SourceId>>,
    _hold: gio::ApplicationHoldGuard,
}
