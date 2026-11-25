//! Tessera — an automatic desktop icon layer for Wayland.
//!
//! The crate is split into a few layers:
//!
//! - `layout`, `assignment` and `scene` turn a folder listing and output
//!   geometry into a placement plan, without any GTK widgets;
//! - `desktop`, `state` and `fileops` read the folder, persist pinned
//!   positions, and perform the file operations the UI asks for;
//! - `ui` builds the layer surfaces and widgets and is the only place that
//!   owns GTK windows;
//! - `config` and `main` deal with configuration and startup.

pub mod assignment;
pub mod config;
pub mod desktop;
pub mod fileops;
pub mod layout;
pub mod scene;
pub mod state;
pub mod ui;
