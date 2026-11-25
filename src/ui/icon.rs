//! The icon button and the pointer gestures attached to it.

use std::cell::Cell;
use std::path::Path;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;

use crate::desktop::DesktopItem;
use crate::layout::Slot;

use super::DesktopHost;

/// Everything a gesture handler needs about the item behind a button.
#[derive(Clone)]
struct IconContext {
    item: DesktopItem,
    slot: Slot,
    source_view: usize,
    host: Weak<DesktopHost>,
    single_click: bool,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn make_icon_button(
    item: &DesktopItem,
    slot: Slot,
    source_view: usize,
    icon_size: i32,
    font_size: u32,
    single_click: bool,
    host: Weak<DesktopHost>,
) -> gtk::Button {
    let image = gtk::Image::from_gicon(&item.icon);
    image.set_pixel_size(icon_size);
    image.set_size_request(icon_size, icon_size);
    image.set_halign(gtk::Align::Center);

    let label = gtk::Label::new(Some(&item.name));
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_lines(2);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_justify(gtk::Justification::Center);
    label.set_xalign(0.5);
    label.set_halign(gtk::Align::Fill);
    label.set_hexpand(true);
    label.add_css_class("tessera-label");
    label.set_attributes(Some(&label_attributes(font_size)));

    let content = gtk::Box::new(gtk::Orientation::Vertical, 5);
    content.set_halign(gtk::Align::Fill);
    content.set_valign(gtk::Align::Center);
    content.append(&image);
    content.append(&label);

    let button = gtk::Button::new();
    button.add_css_class("tessera-icon");
    button.set_has_frame(false);
    button.set_child(Some(&content));
    button.set_size_request(slot.width, slot.height);

    let context = IconContext {
        item: item.clone(),
        slot,
        source_view,
        host,
        single_click,
    };

    // Shared between activation and dragging: a drag must not also count as
    // a click or a double-click when it ends over the same button.
    let suppress_until = Rc::new(Cell::new(None::<Instant>));
    wire_activate(&button, &context, Rc::clone(&suppress_until));
    wire_rearrange(&button, &context, suppress_until);
    wire_export(&button, &context.item.path);
    wire_context_menu(&button, &context);

    button
}

fn label_attributes(font_size: u32) -> gtk::pango::AttrList {
    let attributes = gtk::pango::AttrList::new();
    let size = gtk::pango::AttrSize::new((font_size as i32) * gtk::pango::SCALE);
    attributes.insert(size);
    attributes
}

fn wire_activate(
    button: &gtk::Button,
    context: &IconContext,
    suppress_until: Rc<Cell<Option<Instant>>>,
) {
    let button_ref = button.clone();
    let item = context.item.clone();
    let host = context.host.clone();
    let source_view = context.source_view;
    let slot = context.slot;
    let single_click = context.single_click;
    let last_click = Rc::new(Cell::new(None::<Instant>));

    button.connect_clicked(move |_| {
        if let Some(until) = suppress_until.get() {
            if Instant::now() < until {
                return;
            }
            suppress_until.set(None);
        }

        if let Some(host) = host.upgrade() {
            host.select_item(
                item.clone(),
                button_ref.clone(),
                source_view,
                slot.column,
                slot.row,
            );
        }

        let now = Instant::now();
        let activate = single_click
            || last_click
                .get()
                .is_some_and(|last| now.duration_since(last) <= Duration::from_millis(450));
        last_click.set(if activate { None } else { Some(now) });

        if activate && let Some(host) = host.upgrade() {
            host.open_item(&item);
        }
    });
}

fn wire_rearrange(
    button: &gtk::Button,
    context: &IconContext,
    suppress_until: Rc<Cell<Option<Instant>>>,
) {
    let drag = gtk::GestureDrag::new();
    drag.set_button(1);
    drag.set_propagation_phase(gtk::PropagationPhase::Capture);

    let drag_start = Rc::new(Cell::new(None::<(f64, f64)>));
    let drag_started = Rc::new(Cell::new(false));
    {
        let start = Rc::clone(&drag_start);
        let started = Rc::clone(&drag_started);
        drag.connect_drag_begin(move |_gesture, start_x, start_y| {
            // GtkGestureDrag starts on press.  Do not claim the sequence or
            // begin a move until the pointer has actually travelled far
            // enough to be a drag; otherwise every click would move the icon.
            start.set(Some((start_x, start_y)));
            started.set(false);
        });
    }

    let weak = context.host.clone();
    let source_view = context.source_view;
    let slot = context.slot;
    let item = context.item.clone();
    let button_for_update = button.clone();
    let start_for_update = Rc::clone(&drag_start);
    let started_for_update = Rc::clone(&drag_started);
    let suppress_for_update = Rc::clone(&suppress_until);
    drag.connect_drag_update(move |gesture, offset_x, offset_y| {
        if !started_for_update.get() {
            if offset_x.abs() < 8.0 && offset_y.abs() < 8.0 {
                return;
            }
            // Shift+drag is reserved for exporting files through the
            // compositor's drag-and-drop protocol.
            if gesture
                .current_event_state()
                .contains(gdk::ModifierType::SHIFT_MASK)
            {
                return;
            }
            let Some(start) = start_for_update.get() else {
                return;
            };
            started_for_update.set(true);
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Some(host) = weak.upgrade() {
                host.begin_drag(
                    item.clone(),
                    source_view,
                    slot,
                    start,
                    button_for_update.clone(),
                    Rc::clone(&suppress_for_update),
                );
            }
        }

        if let Some(host) = weak.upgrade() {
            host.update_drag(offset_x, offset_y);
        }
    });

    {
        let weak = context.host.clone();
        let started = Rc::clone(&drag_started);
        let start = Rc::clone(&drag_start);
        drag.connect_drag_end(move |_, _, _| {
            let was_dragging = started.replace(false);
            start.set(None);
            if was_dragging && let Some(host) = weak.upgrade() {
                host.end_drag();
            }
        });
    }

    {
        let weak = context.host.clone();
        let started = Rc::clone(&drag_started);
        let start = Rc::clone(&drag_start);
        drag.connect_cancel(move |_, _| {
            let was_dragging = started.replace(false);
            start.set(None);
            if was_dragging && let Some(host) = weak.upgrade() {
                host.cancel_drag();
            }
        });
    }

    button.add_controller(drag);
}

/// Shift+drag exports the file through Wayland DND.  A plain drag stays
/// inside Tessera and rearranges the icon grid.
fn wire_export(button: &gtk::Button, path: &Path) {
    let source = gtk::DragSource::new();
    source.set_actions(gdk::DragAction::COPY);
    let uri_path = path.to_path_buf();
    source.connect_prepare(move |source, _, _| {
        if !source
            .current_event_state()
            .contains(gdk::ModifierType::SHIFT_MASK)
        {
            return None;
        }
        let uri = gio::File::for_path(&uri_path).uri().to_string();
        let bytes = glib::Bytes::from_owned(format!("{uri}\r\n").into_bytes());
        Some(gdk::ContentProvider::for_bytes("text/uri-list", &bytes))
    });
    button.add_controller(source);
}

fn wire_context_menu(button: &gtk::Button, context: &IconContext) {
    let right_click = gtk::GestureClick::new();
    right_click.set_button(3);

    let button_ref = button.clone();
    let host = context.host.clone();
    let item = context.item.clone();
    let source_view = context.source_view;
    let slot = context.slot;

    right_click.connect_pressed(move |_, _, x, y| {
        if let Some(host) = host.upgrade() {
            host.select_item(
                item.clone(),
                button_ref.clone(),
                source_view,
                slot.column,
                slot.row,
            );
            host.open_context_menu(
                item.clone(),
                source_view,
                slot.x as f64 + x,
                slot.y as f64 + y,
            );
        }
    });
    button.add_controller(right_click);
}
