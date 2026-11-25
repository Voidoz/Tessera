//! User-visible error reporting through the desktop notification service.
//!
//! Tessera does not draw its own error UI.  Failures are sent as
//! `GNotification`s, so the session's notification daemon (dunst, mako,
//! GNOME, KDE, …) presents them with the user's own styling and history.
//!
//! All errors share one notification id: a burst of failures updates the
//! existing message instead of stacking a new window each time.

use gtk::gio;
use gtk::prelude::*;
use log::debug;

const ERROR_ID: &str = "tessera-error";

/// Show `message` as an error notification.
///
/// The application must already be registered on the session bus, which is
/// true from `activate` onwards.
pub fn error(app: &impl IsA<gio::Application>, message: &str) {
    debug!("notification: {message}");
    let notification = gio::Notification::new("Tessera");
    notification.set_body(Some(message));
    notification.set_icon(&gio::ThemedIcon::new("dialog-error"));
    app.upcast_ref::<gio::Application>()
        .send_notification(Some(ERROR_ID), &notification);
}
