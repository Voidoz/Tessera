//! The GTK style sheet used by the desktop surfaces.

use gtk::gdk;

const CSS: &str = r#"
window.tessera-window {
    background-color: transparent;
    color: white;
}

button.tessera-icon {
    background-color: transparent;
    background-image: none;
    border: 1px solid transparent;
    border-radius: 16px;
    box-shadow: none;
    padding: 4px;
    min-width: 0;
    min-height: 0;
    transition: background-color 120ms ease, border-color 120ms ease, opacity 100ms ease;
}

button.tessera-icon:hover {
    background-color: rgba(255, 255, 255, 0.11);
    border-color: rgba(255, 255, 255, 0.10);
}

button.tessera-icon:active {
    background-color: rgba(255, 255, 255, 0.18);
}

button.tessera-icon.selected {
    background-color: rgba(255, 255, 255, 0.20);
    border-color: rgba(255, 255, 255, 0.36);
}

button.tessera-icon.dragging {
    opacity: 0.32;
}

button.tessera-icon:focus {
    outline: none;
}

button.tessera-icon label {
    color: #ffffff;
    font-size: 13px;
    font-weight: 500;
    text-shadow: 0 1px 3px rgba(0, 0, 0, 0.95),
                 0 0 7px rgba(0, 0, 0, 0.55);
}

.tessera-drop {
    background-color: rgba(120, 170, 255, 0.20);
    border: 2px solid rgba(190, 215, 255, 0.80);
    border-radius: 16px;
}

.tessera-scrim {
    background-color: rgba(0, 0, 0, 0.16);
}

.tessera-menu,
.tessera-dialog {
    background-color: rgba(27, 29, 35, 0.97);
    border: 1px solid rgba(255, 255, 255, 0.16);
    border-radius: 12px;
    padding: 7px;
    box-shadow: 0 10px 32px rgba(0, 0, 0, 0.55);
    color: #ffffff;
}

.tessera-menu-title {
    color: rgba(255, 255, 255, 0.62);
    font-size: 11px;
    font-weight: 600;
    padding: 5px 10px 4px;
}

.tessera-dialog-title {
    font-size: 15px;
    font-weight: 600;
    padding: 4px 4px 8px;
}

button.tessera-menu-item {
    background-color: transparent;
    background-image: none;
    border: none;
    box-shadow: none;
    border-radius: 8px;
    padding: 7px 10px;
    min-height: 0;
}

button.tessera-menu-item:hover {
    background-color: rgba(255, 255, 255, 0.12);
}

button.tessera-menu-item label {
    color: #ffffff;
}

button.tessera-primary {
    background-color: rgba(110, 160, 255, 0.72);
    border: none;
    border-radius: 8px;
    padding: 6px 14px;
    color: #ffffff;
}

button.tessera-primary:hover {
    background-color: rgba(130, 176, 255, 0.88);
}

entry.tessera-entry {
    background-color: rgba(255, 255, 255, 0.10);
    border: 1px solid rgba(255, 255, 255, 0.20);
    border-radius: 8px;
    padding: 6px 8px;
    color: #ffffff;
}

label.tessera-error {
    color: #ffb4b4;
    font-size: 12px;
}

label.tessera-property-key {
    color: rgba(255, 255, 255, 0.55);
    font-size: 12px;
}

label.tessera-property-value {
    color: #ffffff;
    font-size: 12px;
}
"#;

/// Install the style sheet for the default display.
pub(super) fn install() {
    let display = gdk::Display::default().expect("a GDK display is required");
    let provider = gtk::CssProvider::new();
    provider.load_from_data(CSS);
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}
