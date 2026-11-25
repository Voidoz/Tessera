//! Configuration and command-line handling.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use serde::Deserialize;

use crate::layout::{Corner, GridSpec, MAX_ICON_PX, MIN_ICON_PX};

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

/// A named icon size preset or an explicit pixel size.
///
/// The configuration file accepts either form:
///
/// ```toml
/// icon_size = "large"   # small | medium | large
/// icon_size = 72        # explicit pixels
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconSize {
    Small,
    Medium,
    Large,
    Custom(u32),
}

impl IconSize {
    pub const DEFAULT: Self = Self::Medium;

    pub fn pixels(self) -> u32 {
        match self {
            Self::Small => 48,
            Self::Medium => 64,
            Self::Large => 96,
            Self::Custom(pixels) => pixels,
        }
    }

    /// Parse a preset name (case-insensitive) or an explicit pixel count.
    pub fn parse(value: &str) -> Result<Self, String> {
        let value = value.trim();
        match value.to_ascii_lowercase().as_str() {
            "small" => Ok(Self::Small),
            "medium" => Ok(Self::Medium),
            "large" => Ok(Self::Large),
            _ => value.parse::<u32>().map(Self::Custom).map_err(|_| {
                format!("unknown icon size `{value}`; use small, medium, large, or a pixel size")
            }),
        }
    }

    /// Keep custom pixel sizes within the range the layout can draw.
    fn clamped(self) -> Self {
        match self {
            Self::Custom(pixels) => Self::Custom(pixels.clamp(MIN_ICON_PX, MAX_ICON_PX)),
            named => named,
        }
    }
}

impl std::str::FromStr for IconSize {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl<'de> Deserialize<'de> for IconSize {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Name(String),
            Pixels(i64),
        }

        match Repr::deserialize(deserializer)? {
            Repr::Name(name) => Self::parse(&name).map_err(serde::de::Error::custom),
            Repr::Pixels(pixels) => u32::try_from(pixels)
                .map(Self::Custom)
                .map_err(|_| serde::de::Error::custom("icon size must be a positive pixel count")),
        }
    }
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
    /// Preferred icon size as a preset name or explicit pixels.
    pub icon_size: IconSize,
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
            icon_size: IconSize::DEFAULT,
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
        self.icon_size = self.icon_size.clamped();
        let spec = self.grid_spec();
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
        GridSpec::new(
            self.icon_size.pixels(),
            self.font_size,
            self.gap,
            self.margin,
        )
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

/// Automatic desktop icons for Wayland.
///
/// The output, desktop directory, state file and icon size also read the
/// matching `TESSERA_*` environment variables when their flag is absent; an
/// explicit flag always wins.
#[derive(Debug, Parser)]
#[command(
    name = "tessera",
    version,
    after_help = "Configuration lives in ~/.config/tessera/config.toml.\n\
                  Dragged positions live in ~/.local/state/tessera/positions.toml."
)]
pub struct Cli {
    /// Preferred output for automatically placed items.
    #[arg(long, value_name = "CONNECTOR", env = "TESSERA_OUTPUT")]
    pub output: Option<String>,

    /// Folder to show instead of the XDG Desktop folder.
    #[arg(long, value_name = "PATH", env = "TESSERA_DESKTOP_DIR")]
    pub desktop_dir: Option<PathBuf>,

    /// Configuration file to use.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Where to store dragged icon positions.
    #[arg(long, value_name = "PATH", env = "TESSERA_STATE_FILE")]
    pub state_file: Option<PathBuf>,

    /// Icon size: small, medium, large, or a pixel count.
    #[arg(long, value_name = "SIZE", env = "TESSERA_ICON_SIZE")]
    pub icon_size: Option<IconSize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        let config = Config::default().sanitized();
        assert_eq!(config.corner, Corner::TopLeft);
        assert_eq!(config.icon_size, IconSize::Medium);
        assert!(!config.single_click);
        assert_eq!(config.drop_target, DropTargetMode::Icons);
    }

    #[test]
    fn clamps_absurd_values() {
        let config = Config {
            icon_size: IconSize::Custom(1),
            font_size: 100,
            gap: 500,
            margin: 1000,
            ..Config::default()
        }
        .sanitized();
        assert_eq!(config.icon_size, IconSize::Custom(32));
        assert_eq!(config.font_size, 28);
        assert_eq!(config.gap, 48);
        assert_eq!(config.margin, 240);
    }

    #[test]
    fn clamps_large_icon_sizes() {
        let config = Config {
            icon_size: IconSize::Custom(999),
            ..Config::default()
        }
        .sanitized();
        assert_eq!(config.icon_size, IconSize::Custom(160));
    }

    #[test]
    fn icon_size_presets_and_numbers_parse() {
        assert_eq!(IconSize::parse("small"), Ok(IconSize::Small));
        assert_eq!(IconSize::parse(" Medium "), Ok(IconSize::Medium));
        assert_eq!(IconSize::parse("LARGE"), Ok(IconSize::Large));
        assert_eq!(IconSize::parse("72"), Ok(IconSize::Custom(72)));
        assert_eq!(
            IconSize::parse("huge"),
            Err("unknown icon size `huge`; use small, medium, large, or a pixel size".to_owned())
        );
    }

    #[test]
    fn preset_pixel_sizes() {
        assert_eq!(IconSize::Small.pixels(), 48);
        assert_eq!(IconSize::Medium.pixels(), 64);
        assert_eq!(IconSize::Large.pixels(), 96);
        assert_eq!(IconSize::Custom(72).pixels(), 72);
    }

    #[test]
    fn preset_sizes_reach_the_grid_spec() {
        let large = Config {
            icon_size: IconSize::Large,
            ..Config::default()
        }
        .sanitized();
        assert_eq!(large.grid_spec().icon_size, 96);
    }

    #[test]
    fn parses_output_forms() {
        for args in [
            vec!["tessera", "--output", "DP-2"],
            vec!["tessera", "--output=DP-2"],
        ] {
            let cli = Cli::try_parse_from(args).expect("valid args");
            assert_eq!(cli.output.as_deref(), Some("DP-2"));
        }
    }

    #[test]
    fn parses_every_flag() {
        let cli = Cli::try_parse_from([
            "tessera",
            "--output",
            "DP-2",
            "--desktop-dir",
            "/tmp/desk",
            "--config",
            "/tmp/config.toml",
            "--state-file=/tmp/state.toml",
            "--icon-size",
            "large",
        ])
        .expect("valid args");
        assert_eq!(cli.output.as_deref(), Some("DP-2"));
        assert_eq!(cli.icon_size, Some(IconSize::Large));
        assert_eq!(cli.desktop_dir.as_deref(), Some(Path::new("/tmp/desk")));
        assert_eq!(cli.config.as_deref(), Some(Path::new("/tmp/config.toml")));
        assert_eq!(
            cli.state_file.as_deref(),
            Some(Path::new("/tmp/state.toml"))
        );
    }

    #[test]
    fn parses_numeric_icon_size_flag() {
        let cli = Cli::try_parse_from(["tessera", "--icon-size=72"]).expect("valid args");
        assert_eq!(cli.icon_size, Some(IconSize::Custom(72)));

        let error =
            Cli::try_parse_from(["tessera", "--icon-size", "huge"]).expect_err("invalid size");
        assert!(error.to_string().contains("unknown icon size"));
    }

    #[test]
    fn rejects_unknown_arguments() {
        let error = Cli::try_parse_from(["tessera", "--wat"]).expect_err("unknown flag");
        assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
    }

    #[test]
    fn flags_read_environment_variables() {
        use clap::CommandFactory;

        let command = Cli::command();
        let envs: Vec<(String, String)> = command
            .get_arguments()
            .filter_map(|arg| {
                let env = arg.get_env()?;
                Some((arg.get_id().to_string(), env.to_string_lossy().into_owned()))
            })
            .collect();
        assert!(envs.contains(&("output".to_owned(), "TESSERA_OUTPUT".to_owned())));
        assert!(envs.contains(&("desktop_dir".to_owned(), "TESSERA_DESKTOP_DIR".to_owned())));
        assert!(envs.contains(&("state_file".to_owned(), "TESSERA_STATE_FILE".to_owned())));
        assert!(envs.contains(&("icon_size".to_owned(), "TESSERA_ICON_SIZE".to_owned())));
    }

    #[test]
    fn help_and_version_are_display_errors() {
        for args in [["tessera", "--help"], ["tessera", "-h"]] {
            let error = Cli::try_parse_from(args).expect_err("help exits early");
            assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
        }
        for args in [["tessera", "--version"], ["tessera", "-V"]] {
            let error = Cli::try_parse_from(args).expect_err("version exits early");
            assert_eq!(error.kind(), clap::error::ErrorKind::DisplayVersion);
        }
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
        assert_eq!(config.icon_size, IconSize::Custom(72));
        assert!(config.single_click);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn reads_a_named_icon_size() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tessera-config-size-{unique}.toml"));
        std::fs::write(&path, "icon_size = \"large\"\n").expect("write config");

        let config = Config::load(Some(&path)).expect("load config");
        assert_eq!(config.icon_size, IconSize::Large);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rejects_an_unknown_icon_size() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tessera-config-bad-size-{unique}.toml"));
        std::fs::write(&path, "icon_size = \"gigantic\"\n").expect("write config");

        assert!(Config::load(Some(&path)).is_err());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn reports_a_missing_explicit_config() {
        assert!(Config::load(Some(Path::new("/definitely/not/here.toml"))).is_err());
    }
}
