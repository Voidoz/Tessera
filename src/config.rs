//! Configuration and command-line handling.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::layout::{Corner, GridSpec};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DropTargetMode {
    /// Only icon cells accept file drops; the rest of the desktop stays
    /// click-through.
    #[default]
    Icons,
    /// The entire desktop accepts file drops, at the cost of click-through.
    Full,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Preferred output for automatically placed items, e.g. "DP-2" or
    /// "eDP-1".  Every output gets a surface; this only controls where new,
    /// unpinned items appear first.  Dragging an item to another output pins
    /// it there.
    pub output: Option<String>,
    /// Folder whose contents are shown. Defaults to the XDG Desktop folder.
    pub desktop_dir: Option<PathBuf>,
    /// Corner from which the grid grows.
    pub corner: Corner,
    /// Preferred icon image size in logical pixels.
    pub icon_size: u32,
    /// Desktop label size in logical pixels.
    pub font_size: u32,
    /// Space between slots in logical pixels.
    pub gap: u32,
    /// Distance from the output edge in logical pixels.
    pub margin: u32,
    /// Open items with a single click instead of a double click.
    pub single_click: bool,
    /// Whether external file drops are accepted only over icons or over the
    /// whole desktop.
    pub drop_target: DropTargetMode,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            output: None,
            desktop_dir: None,
            corner: Corner::TopLeft,
            icon_size: 64,
            font_size: 13,
            gap: 8,
            margin: 32,
            single_click: false,
            drop_target: DropTargetMode::Icons,
        }
    }
}

impl Config {
    /// Load configuration from an explicit path or from the usual XDG path.
    ///
    /// A missing default file is not an error.  A malformed file is reported
    /// to the caller so the application can warn and continue with defaults.
    pub fn load(explicit: Option<&Path>) -> Result<Self> {
        let (path, required) = match explicit {
            Some(path) => (path.to_path_buf(), true),
            None => (
                gtk::glib::user_config_dir().join("tessera/config.toml"),
                false,
            ),
        };

        if !path.exists() {
            if required {
                anyhow::bail!("configuration file {} does not exist", path.display());
            }
            return Ok(Self::default());
        }

        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        let config: Self = toml::from_str(&text)
            .with_context(|| format!("invalid configuration in {}", path.display()))?;
        Ok(config.sanitized())
    }

    /// Clamp values once, so the rest of the application can rely on them.
    pub fn sanitized(mut self) -> Self {
        let spec = self.grid_spec();
        self.icon_size = spec.icon_size as u32;
        self.font_size = spec.font_size as u32;
        self.gap = spec.gap as u32;
        self.margin = spec.margin as u32;
        if self
            .output
            .as_deref()
            .is_some_and(|name| name.trim().is_empty())
        {
            self.output = None;
        }
        self
    }

    pub fn grid_spec(&self) -> GridSpec {
        GridSpec::new(self.icon_size, self.font_size, self.gap, self.margin)
    }

    pub fn desktop_dir(&self) -> PathBuf {
        if let Some(path) = &self.desktop_dir {
            return path.clone();
        }
        if let Some(path) = gtk::glib::user_special_dir(gtk::glib::UserDirectory::Desktop) {
            return path;
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"))
            .join("Desktop")
    }
}

#[derive(Debug, Clone, Default)]
pub struct Cli {
    pub config: Option<PathBuf>,
    pub output: Option<String>,
    pub desktop_dir: Option<PathBuf>,
    pub state_file: Option<PathBuf>,
}

#[derive(Debug)]
pub enum Action {
    Run(Cli),
    Help,
    Version,
}

pub fn parse_args<I>(args: I) -> std::result::Result<Action, String>
where
    I: IntoIterator<Item = OsString>,
{
    let mut cli = Cli::default();
    let mut args = args.into_iter().skip(1).peekable();

    while let Some(arg) = args.next() {
        let arg = arg.to_string_lossy();
        match arg.as_ref() {
            "-h" | "--help" => return Ok(Action::Help),
            "-V" | "--version" => return Ok(Action::Version),
            "--output" => {
                cli.output = Some(value_for(&mut args, "--output")?);
            }
            "--desktop-dir" => {
                cli.desktop_dir = Some(PathBuf::from(value_for(&mut args, "--desktop-dir")?));
            }
            "--config" => {
                cli.config = Some(PathBuf::from(value_for(&mut args, "--config")?));
            }
            "--state-file" => {
                cli.state_file = Some(PathBuf::from(value_for(&mut args, "--state-file")?));
            }
            _ if arg.starts_with("--output=") => {
                cli.output = Some(arg["--output=".len()..].to_owned());
            }
            _ if arg.starts_with("--desktop-dir=") => {
                cli.desktop_dir = Some(PathBuf::from(&arg["--desktop-dir=".len()..]));
            }
            _ if arg.starts_with("--config=") => {
                cli.config = Some(PathBuf::from(&arg["--config=".len()..]));
            }
            _ if arg.starts_with("--state-file=") => {
                cli.state_file = Some(PathBuf::from(&arg["--state-file=".len()..]));
            }
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }

    Ok(Action::Run(cli))
}

fn value_for<I>(
    args: &mut std::iter::Peekable<I>,
    flag: &str,
) -> std::result::Result<String, String>
where
    I: Iterator<Item = OsString>,
{
    match args.next() {
        Some(value) if !value.is_empty() => Ok(value.to_string_lossy().into_owned()),
        _ => Err(format!("{flag} requires a value")),
    }
}

pub const HELP: &str = "\
Tessera — automatic desktop icons for Wayland

Usage: tessera [OPTIONS]

Options:
      --output <CONNECTOR>     Preferred output for automatically placed items.
      --desktop-dir <PATH>     Folder to show instead of the XDG Desktop folder.
      --config <PATH>          Configuration file to use.
      --state-file <PATH>      Where to store dragged icon positions.
  -h, --help                   Print this help.
  -V, --version                Print the version.

Configuration lives in ~/.config/tessera/config.toml.  Dragged positions live in
~/.local/state/tessera/positions.toml.  Environment variables TESSERA_OUTPUT,
TESSERA_DESKTOP_DIR and TESSERA_STATE_FILE are also accepted.
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        let config = Config::default().sanitized();
        assert_eq!(config.corner, Corner::TopLeft);
        assert_eq!(config.icon_size, 64);
        assert!(!config.single_click);
        assert_eq!(config.drop_target, DropTargetMode::Icons);
    }

    #[test]
    fn clamps_absurd_values() {
        let config = Config {
            icon_size: 1,
            font_size: 100,
            gap: 500,
            margin: 1000,
            ..Config::default()
        }
        .sanitized();
        assert_eq!(config.icon_size, 32);
        assert_eq!(config.font_size, 28);
        assert_eq!(config.gap, 48);
        assert_eq!(config.margin, 240);
    }

    #[test]
    fn parses_output_forms() {
        let action = parse_args([OsString::from("tessera"), OsString::from("--output=DP-2")])
            .expect("valid args");
        let Action::Run(cli) = action else {
            panic!("expected run action");
        };
        assert_eq!(cli.output.as_deref(), Some("DP-2"));
    }

    #[test]
    fn rejects_unknown_arguments() {
        assert!(parse_args([OsString::from("tessera"), OsString::from("--wat")]).is_err());
    }
}

#[cfg(test)]
mod load_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn reads_a_config_file() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tessera-config-{unique}.toml"));
        std::fs::write(
            &path,
            "corner = \"top-right\"\nicon_size = 72\nsingle_click = true\n",
        )
        .expect("write config");

        let config = Config::load(Some(&path)).expect("load config");
        assert_eq!(config.corner, Corner::TopRight);
        assert_eq!(config.icon_size, 72);
        assert!(config.single_click);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn reports_a_missing_explicit_config() {
        assert!(Config::load(Some(Path::new("/definitely/not/here.toml"))).is_err());
    }
}
