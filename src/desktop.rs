//! Reading the Desktop folder and turning its entries into displayable items.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use log::{debug, warn};

/// What happens when an item is activated.
#[derive(Debug, Clone)]
pub enum OpenAction {
    /// A freedesktop application entry, launched through GIO so that Exec,
    /// Terminal, DBusActivatable and friends are handled by the standard
    /// implementation.
    DesktopEntry(PathBuf),
    /// A normal file, folder or Link entry.
    Uri(String),
}

#[derive(Debug, Clone)]
pub struct DesktopItem {
    pub path: PathBuf,
    pub name: String,
    pub icon: gio::Icon,
    pub action: OpenAction,
    pub is_dir: bool,
    pub content_type: Option<String>,
    pub sort_key: glib::FilenameCollationKey,
}

impl DesktopItem {
    /// Activate the item using the desktop's default handling.
    pub fn open(&self) -> Result<(), String> {
        match &self.action {
            OpenAction::DesktopEntry(path) => {
                let quoted = glib::shell_quote(path.as_os_str());
                let mut command = std::ffi::OsString::from("gio launch ");
                command.push(quoted);
                glib::spawn_command_line_async(command)
                    .map_err(|error| format!("could not launch {}: {error}", path.display()))
            }
            OpenAction::Uri(uri) => {
                gio::AppInfo::launch_default_for_uri(uri, None::<&gio::AppLaunchContext>)
                    .map_err(|error| format!("could not open {uri}: {error}"))
            }
        }
    }
}

/// Read and sort all visible items in `directory`.
///
/// A missing folder is treated as an empty desktop.  Individual unreadable
/// entries are skipped instead of taking the whole desktop down.
pub fn scan(directory: &Path) -> Vec<DesktopItem> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            warn!("desktop directory {} does not exist", directory.display());
            return Vec::new();
        }
        Err(error) => {
            warn!(
                "cannot read desktop directory {}: {error}",
                directory.display()
            );
            return Vec::new();
        }
    };

    let mut items = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warn!("cannot read a desktop entry: {error}");
                continue;
            }
        };

        let path = entry.path();
        if is_hidden(&path) {
            continue;
        }

        match item_for(&path) {
            Ok(Some(item)) => items.push(item),
            Ok(None) => {}
            Err(error) => warn!("skipping {}: {error}", path.display()),
        }
    }

    items.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.sort_key.cmp(&b.sort_key))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.path.cmp(&b.path))
    });
    items
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| name.starts_with('.'))
}

fn item_for(path: &Path) -> Result<Option<DesktopItem>, String> {
    if path
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("desktop"))
    {
        match desktop_entry_item(path)? {
            EntryOutcome::Item(item) => return Ok(Some(item)),
            EntryOutcome::Skip => return Ok(None),
            EntryOutcome::Fallback => {}
        }
    }

    let file = gio::File::for_path(path);
    let info = file
        .query_info(
            "standard::type,standard::content-type,standard::icon",
            gio::FileQueryInfoFlags::NONE,
            None::<&gio::Cancellable>,
        )
        .map_err(|error| error.to_string())?;

    let is_dir = info.file_type() == gio::FileType::Directory;
    let name = display_name(path);
    let icon = if is_dir {
        gio::content_type_get_icon("inode/directory")
    } else {
        info.icon()
            .or_else(|| {
                info.content_type()
                    .map(|content_type| gio::content_type_get_icon(content_type.as_str()))
            })
            .unwrap_or_else(|| gio::ThemedIcon::new("text-x-generic").upcast())
    };

    Ok(Some(DesktopItem {
        path: path.to_path_buf(),
        name: clean_label(&name),
        icon,
        action: OpenAction::Uri(file.uri().to_string()),
        is_dir,
        content_type: info
            .content_type()
            .map(|content_type| content_type.to_string()),
        sort_key: glib::FilenameCollationKey::from(name),
    }))
}

enum EntryOutcome {
    Item(DesktopItem),
    /// Explicitly hidden by the entry itself.
    Skip,
    /// Not a usable application entry, so show it as a plain file instead.
    Fallback,
}

fn desktop_entry_item(path: &Path) -> Result<EntryOutcome, String> {
    let key_file = glib::KeyFile::new();
    if let Err(error) = key_file.load_from_file(path, glib::KeyFileFlags::NONE) {
        debug!("{} is not a valid desktop entry: {error}", path.display());
        return Ok(EntryOutcome::Fallback);
    }
    if !key_file.has_group("Desktop Entry") {
        return Ok(EntryOutcome::Fallback);
    }
    if key_file.boolean("Desktop Entry", "Hidden").unwrap_or(false) {
        return Ok(EntryOutcome::Skip);
    }

    let entry_type = key_file
        .string("Desktop Entry", "Type")
        .unwrap_or_else(|_| glib::GString::from("Application"));
    if entry_type.as_str() != "Application" && entry_type.as_str() != "Link" {
        return Ok(EntryOutcome::Fallback);
    }

    let name = key_file
        .locale_string("Desktop Entry", "Name", None)
        .ok()
        .map(|name| name.to_string())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| display_name(path));

    let icon = key_file
        .locale_string("Desktop Entry", "Icon", None)
        .ok()
        .filter(|name| !name.trim().is_empty())
        .map(|name| icon_for_name(name.as_str()))
        .unwrap_or_else(|| gio::ThemedIcon::new("application-x-executable").upcast());

    let action = if entry_type.as_str() == "Link" {
        key_file
            .string("Desktop Entry", "URL")
            .ok()
            .filter(|url| !url.is_empty())
            .map(|url| OpenAction::Uri(url.to_string()))
            .unwrap_or_else(|| OpenAction::DesktopEntry(path.to_path_buf()))
    } else {
        OpenAction::DesktopEntry(path.to_path_buf())
    };

    Ok(EntryOutcome::Item(DesktopItem {
        path: path.to_path_buf(),
        name: clean_label(&name),
        icon,
        action,
        is_dir: false,
        content_type: Some("application/x-desktop".to_owned()),
        sort_key: glib::FilenameCollationKey::from(name),
    }))
}

fn icon_for_name(name: &str) -> gio::Icon {
    if Path::new(name).is_absolute() {
        let file = gio::File::for_path(name);
        if file.query_exists(None::<&gio::Cancellable>) {
            return gio::FileIcon::new(&file).upcast();
        }
        return gio::ThemedIcon::new("application-x-executable").upcast();
    }
    gio::ThemedIcon::new(name).upcast()
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.display().to_string())
}

fn clean_label(name: &str) -> String {
    let name = name.replace(['\n', '\r', '\t'], " ");
    if name.trim().is_empty() {
        "Untitled".to_owned()
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_never_contain_newlines() {
        assert_eq!(clean_label("hello\nworld"), "hello world");
        assert_eq!(clean_label("   "), "Untitled");
    }

    #[test]
    fn hidden_names_are_detected() {
        assert!(is_hidden(Path::new("/home/me/Desktop/.directory")));
        assert!(!is_hidden(Path::new("/home/me/Desktop/visible")));
    }
}

#[cfg(test)]
mod scan_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_desktop(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tessera-test-{name}-{unique}"));
        std::fs::create_dir_all(&path).expect("create test desktop");
        path
    }

    #[test]
    fn folders_come_first_and_hidden_entries_are_skipped() {
        let dir = temp_desktop("sort");
        std::fs::create_dir(dir.join("Folder")).expect("folder");
        std::fs::write(dir.join("zebra.txt"), "z").expect("file");
        std::fs::write(dir.join("alpha.txt"), "a").expect("file");
        std::fs::write(dir.join(".hidden"), "hidden").expect("file");

        let items = scan(&dir);
        let names: Vec<&str> = items.iter().map(|item| item.name.as_str()).collect();
        assert_eq!(names, ["Folder", "alpha.txt", "zebra.txt"]);
        assert!(items[0].is_dir);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn desktop_entries_use_their_localized_name() {
        let dir = temp_desktop("entry");
        std::fs::write(
            dir.join("whatever.desktop"),
            "[Desktop Entry]\nType=Application\nName=Pretty Name\nIcon=utilities-terminal\nExec=true\n",
        )
        .expect("desktop file");

        let items = scan(&dir);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "Pretty Name");
        assert!(matches!(items[0].action, OpenAction::DesktopEntry(_)));
        assert!(!items[0].is_dir);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn hidden_desktop_entries_are_not_shown() {
        let dir = temp_desktop("hidden-entry");
        std::fs::write(
            dir.join("hidden.desktop"),
            "[Desktop Entry]\nType=Application\nName=Hidden\nHidden=true\n",
        )
        .expect("desktop file");

        assert!(scan(&dir).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
