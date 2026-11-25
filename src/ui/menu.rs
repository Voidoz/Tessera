//! The item context menu.

use std::rc::Rc;

use gtk::prelude::*;

use crate::desktop::DesktopItem;

use super::DesktopHost;
use super::overlay::{OverlayPosition, OverlayRequest};

impl DesktopHost {
    pub(super) fn open_context_menu(
        self: &Rc<Self>,
        item: DesktopItem,
        output: usize,
        x: f64,
        y: f64,
    ) {
        let can_open_with =
            !item.is_dir && item.content_type.as_deref() != Some("application/x-desktop");
        let rows = if can_open_with { 7 } else { 6 };
        let height = 48 + rows * 31 + 8;
        let Some(card) = self.begin_overlay(OverlayRequest {
            output,
            width: 238,
            height,
            position: OverlayPosition::At(x, y),
            class: "tessera-menu",
        }) else {
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
                host.open_item(&item);
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
}
