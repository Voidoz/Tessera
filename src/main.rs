use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use tessera::config::{Action, Cli, Config};
use tessera::ui::DesktopHost;

fn main() -> glib::ExitCode {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("tessera=info,warn"),
    )
    .init();

    let cli = match tessera::config::parse_args(std::env::args_os()) {
        Ok(Action::Run(cli)) => cli,
        Ok(Action::Help) => {
            print!("{}", tessera::config::HELP);
            return glib::ExitCode::SUCCESS;
        }
        Ok(Action::Version) => {
            println!("tessera {}", env!("CARGO_PKG_VERSION"));
            return glib::ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!("tessera: {error}");
            eprintln!("Try `tessera --help`.");
            return glib::ExitCode::FAILURE;
        }
    };

    let config = match Config::load(cli.config.as_deref()) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("tessera: {error}");
            eprintln!("tessera: continuing with default settings");
            Config::default().sanitized()
        }
    };
    let config = apply_overrides(config, &cli);

    let output = config.output.clone();
    let desktop_dir = config.desktop_dir();
    let state_file = cli
        .state_file
        .clone()
        .or_else(|| std::env::var_os("TESSERA_STATE_FILE").map(Into::into))
        .unwrap_or_else(|| default_state_dir().join("tessera/positions.toml"));

    let app = gtk::Application::builder()
        .application_id("io.github.tessera.Tessera")
        .build();
    let host: Rc<RefCell<Option<Rc<DesktopHost>>>> = Rc::new(RefCell::new(None));

    {
        let host = Rc::clone(&host);
        app.connect_activate(move |app| {
            if host.borrow().is_some() {
                return;
            }

            if !gtk4_layer_shell::is_supported() {
                eprintln!(
                    "tessera: this compositor does not expose the wlr-layer-shell protocol"
                );
                eprintln!(
                    "tessera: use a Wayland compositor with layer-shell support (niri, Hyprland, sway, river, labwc…)"
                );
                app.quit();
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
    let env_output = std::env::var("TESSERA_OUTPUT").ok();
    let env_desktop = std::env::var_os("TESSERA_DESKTOP_DIR");

    if let Some(output) = cli.output.clone().or(env_output) {
        config.output = Some(output);
    }
    if let Some(desktop_dir) = cli
        .desktop_dir
        .clone()
        .or_else(|| env_desktop.map(Into::into))
    {
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
