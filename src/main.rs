use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use clap::Parser;
use gtk::glib;
use gtk::prelude::*;

use tessera::config::{Cli, Config};
use tessera::ui::DesktopHost;

fn main() -> glib::ExitCode {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("tessera=info,warn"),
    )
    .init();

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => error.exit(),
    };

    let mut startup_notice = None;
    let config = match Config::load(cli.config.as_deref()) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("tessera: {error}");
            eprintln!("tessera: continuing with default settings");
            startup_notice = Some(format!(
                "Configuration problem: {error}. Using default settings."
            ));
            Config::default().sanitized()
        }
    };
    let config = apply_overrides(config, &cli);

    let output = config.output.clone();
    let desktop_dir = config.desktop_dir();
    let state_file = cli
        .state_file
        .clone()
        .unwrap_or_else(|| default_state_dir().join("tessera/positions.toml"));

    let app = gtk::Application::builder()
        .application_id("io.github.Voidoz.Tessera")
        .build();
    let host: Rc<RefCell<Option<Rc<DesktopHost>>>> = Rc::new(RefCell::new(None));

    {
        let host = Rc::clone(&host);
        app.connect_activate(move |app| {
            if host.borrow().is_some() {
                return;
            }

            if let Some(message) = &startup_notice {
                tessera::notify::error(app, message);
            }

            if !gtk4_layer_shell::is_supported() {
                eprintln!(
                    "tessera: this compositor does not expose the wlr-layer-shell protocol"
                );
                eprintln!(
                    "tessera: use a Wayland compositor with layer-shell support (niri, Hyprland, sway, river, labwc…)"
                );
                tessera::notify::error(
                    app,
                    "This compositor does not support wlr-layer-shell, so Tessera cannot create desktop surfaces.",
                );
                // Give the notification a moment to reach the bus before the
                // process exits.
                let app = app.clone();
                glib::timeout_add_local_once(Duration::from_millis(250), move || app.quit());
                return;
            }

            let desktop = DesktopHost::start(
                app,
                config.clone(),
                output.clone(),
                desktop_dir.clone(),
                state_file.clone(),
            );
            *host.borrow_mut() = Some(desktop);
        });
    }

    app.run_with_args::<&str>(&[])
}

fn apply_overrides(mut config: Config, cli: &Cli) -> Config {
    if let Some(output) = cli.output.clone() {
        config.output = Some(output);
    }
    if let Some(desktop_dir) = cli.desktop_dir.clone() {
        config.desktop_dir = Some(desktop_dir);
    }
    config.sanitized()
}

fn default_state_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(path);
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local/state")
}
