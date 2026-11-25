//! The Rename, Properties and Open With dialogs.

use std::rc::Rc;
use std::time::UNIX_EPOCH;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use log::warn;

use crate::desktop::DesktopItem;

use super::DesktopHost;
use super::overlay::{OverlayPosition, OverlayRequest};

impl DesktopHost {
    pub(super) fn open_rename_modal(self: &Rc<Self>, item: DesktopItem) {
        let Some(output) = self.selected_output() else {
            return;
        };
        let Some(card) = self.begin_overlay(OverlayRequest {
            output,
            width: 360,
            height: -1,
            position: OverlayPosition::Centered,
            class: "tessera-dialog",
        }) else {
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
            view.focus();
        }
        glib::idle_add_local_once(move || {
            entry_for_focus.grab_focus();
            entry_for_focus.select_region(0, stem_len as i32);
        });
    }

    pub(super) fn open_properties_modal(self: &Rc<Self>, item: DesktopItem) {
        let Some(output) = self.selected_output() else {
            return;
        };
        let Some(card) = self.begin_overlay(OverlayRequest {
            output,
            width: 430,
            height: -1,
            position: OverlayPosition::Centered,
            class: "tessera-dialog",
        }) else {
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

    pub(super) fn open_open_with_modal(self: &Rc<Self>, item: DesktopItem) {
        let Some(content_type) = item.content_type.clone() else {
            return;
        };
        let Some(output) = self.selected_output() else {
            return;
        };
        let Some(card) = self.begin_overlay(OverlayRequest {
            output,
            width: 390,
            height: -1,
            position: OverlayPosition::Centered,
            class: "tessera-dialog",
        }) else {
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
                match app.launch_uris(&[uri.as_str()], None::<&gio::AppLaunchContext>) {
                    Ok(()) => {
                        if let Some(host) = weak.upgrade() {
                            host.close_overlay();
                        }
                    }
                    Err(error) => {
                        let message = format!("Could not launch {}: {error}", app.name());
                        if let Some(host) = weak.upgrade() {
                            host.report_error(&message);
                        } else {
                            warn!("{message}");
                        }
                    }
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
