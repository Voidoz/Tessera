//! File operations triggered from the desktop UI.

use std::rc::Rc;

use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use log::warn;

use crate::desktop::DesktopItem;
use crate::fileops;

use super::DesktopHost;

impl DesktopHost {
    /// Open an item with the desktop's default handler, reporting failures.
    pub(super) fn open_item(&self, item: &DesktopItem) {
        if let Err(error) = item.open() {
            self.report_error(&error);
        }
    }

    pub(super) fn trash_item(self: &Rc<Self>, item: &DesktopItem) {
        let path = item.path.clone();
        let file = gio::File::for_path(&path);
        let weak = Rc::downgrade(self);
        file.trash_async(
            glib::Priority::DEFAULT,
            None::<&gio::Cancellable>,
            move |result| match result {
                Ok(()) => {
                    if let Some(host) = weak.upgrade() {
                        host.state.borrow_mut().remove(&path);
                        host.save_state();
                        host.clear_selection();
                        host.render();
                    }
                }
                Err(error) => {
                    let message = format!("Could not move {} to Trash: {error}", path.display());
                    if let Some(host) = weak.upgrade() {
                        host.report_error(&message);
                    } else {
                        warn!("{message}");
                    }
                }
            },
        );
    }

    pub(super) fn show_in_files(&self, item: &DesktopItem) {
        let Some(parent) = item.path.parent() else {
            return;
        };
        let uri = gio::File::for_path(parent).uri().to_string();
        if let Err(error) =
            gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>)
        {
            self.report_error(&format!("Could not open {}: {error}", parent.display()));
        }
    }

    pub(super) fn copy_path(&self, item: &DesktopItem) {
        if let Some(display) = gdk::Display::default() {
            display.clipboard().set_text(&item.path.to_string_lossy());
        }
    }

    pub(super) fn rename_item(&self, item: &DesktopItem, new_name: &str) -> Result<(), String> {
        let name = fileops::validated_name(new_name)?;

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

        self.state.borrow_mut().rename(&item.path, &target);
        self.save_state();
        self.clear_selection();
        Ok(())
    }
}
