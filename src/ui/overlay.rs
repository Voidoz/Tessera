//! The in-surface scrim and floating card shared by menus and dialogs.
//!
//! Overlays are drawn inside the desktop fixed layout rather than as
//! xdg-popups, so they work on compositors without popup support.  Opening one
//! temporarily makes the whole surface accept input, and closing it restores
//! the icon-cell-only input region.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use super::DesktopHost;

pub(super) struct OverlayState {
    output: usize,
    scrim: gtk::Box,
    card: gtk::Box,
}

/// Where an overlay card should appear on its output.
pub(super) enum OverlayPosition {
    /// Surface-local pixels, adjusted so the card stays on screen.
    At(f64, f64),
    Centered,
}

pub(super) struct OverlayRequest {
    pub(super) output: usize,
    pub(super) width: i32,
    pub(super) height: i32,
    pub(super) position: OverlayPosition,
    /// CSS class for the card, e.g. `tessera-menu` or `tessera-dialog`.
    pub(super) class: &'static str,
}

impl DesktopHost {
    pub(super) fn begin_overlay(self: &Rc<Self>, request: OverlayRequest) -> Option<gtk::Box> {
        self.close_overlay();
        let view = self.views.borrow().get(request.output).cloned()?;
        let (surface_width, surface_height) = view.size();

        let scrim = gtk::Box::new(gtk::Orientation::Vertical, 0);
        scrim.add_css_class("tessera-scrim");
        scrim.set_size_request(surface_width, surface_height);
        let weak = Rc::downgrade(self);
        let click = gtk::GestureClick::new();
        click.connect_pressed(move |_, _, _, _| {
            if let Some(host) = weak.upgrade() {
                host.close_overlay();
            }
        });
        scrim.add_controller(click);
        view.fixed().put(&scrim, 0.0, 0.0);

        let card = gtk::Box::new(gtk::Orientation::Vertical, 2);
        card.add_css_class(request.class);
        card.set_size_request(request.width, request.height);
        card.set_halign(gtk::Align::Start);
        card.set_valign(gtk::Align::Start);
        let (desired_x, desired_y) = match request.position {
            OverlayPosition::At(x, y) => (x, y),
            OverlayPosition::Centered => (0.0, 0.0),
        };
        view.fixed().put(&card, desired_x, desired_y);
        view.set_overlay_open(true);

        let centered = matches!(request.position, OverlayPosition::Centered);
        *self.overlay.borrow_mut() = Some(OverlayState {
            output: request.output,
            scrim,
            card: card.clone(),
        });

        let fixed = view.fixed().clone();
        let card_for_position = card.clone();
        glib::idle_add_local_once(move || {
            let card_width = card_for_position.width().max(1);
            let card_height = card_for_position.height().max(1);
            let x = if centered {
                (surface_width - card_width) / 2
            } else {
                (desired_x as i32)
                    .min(surface_width - card_width - 8)
                    .max(8)
            };
            let y = if centered {
                (surface_height - card_height) / 2
            } else {
                (desired_y as i32)
                    .min(surface_height - card_height - 8)
                    .max(8)
            };
            fixed.move_(&card_for_position, x as f64, y as f64);
        });

        Some(card)
    }

    pub(super) fn close_overlay(&self) {
        let Some(overlay) = self.overlay.borrow_mut().take() else {
            return;
        };

        if let Some(view) = self.views.borrow().get(overlay.output) {
            if overlay.scrim.parent().is_some() {
                view.fixed().remove(&overlay.scrim);
            }
            if overlay.card.parent().is_some() {
                view.fixed().remove(&overlay.card);
            }
            view.set_overlay_open(false);
        }

        self.restore_selection_focus();
    }
}
