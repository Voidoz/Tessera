# Tessera

A small Wayland **desktop icon host** whose only job is to show the contents of
`~/Desktop` across all of your monitors.

Tessera puts a transparent layer-shell surface over every output and lays out
desktop items automatically. Items can be dragged to a new grid slot, including
onto another monitor, and those positions are remembered. It also has a real
file menu, keyboard shortcuts, Trash support, and drag-and-drop in both
directions.

> **Project status:** [`docs/STATUS.md`](docs/STATUS.md) is the authoritative
> list of implemented, partial, and missing work.

## Two kinds of positions

**Automatic.** Items you have never dragged are arranged by Tessera:

- folders come first, then everything else, using locale-aware filename
  collation;
- the grid starts in the configured corner (top-left by default);
- columns fill from top to bottom before a new column starts to the right;
- new items go to the preferred output first, then spill onto the other
  outputs in layout order when it is full;
- labels get two lines and long names are ellipsized;
- if the combined capacity of all outputs is too small, icons compact in steps
  down to 32 px so that everything still fits.

**Pinned.** Drag any item to a slot and it stays there. Dragging to another
monitor moves it to that monitor. Positions are stored in:

```text
~/.local/state/tessera/positions.toml
```

Pinned items keep their slot when icons are resized or the folder changes.
Unpinned items reflow around them. To reset everything to automatic placement,
delete that file and restart Tessera.

Every item appears on exactly one output; the folder is not mirrored. If you
want different icons on different screens, drag them there.

## Interaction

### Pointer

- **Drag** an icon to another free grid slot. The nearest free slot is used,
  with a target outline shown on whichever monitor the pointer is over.
- Drag past the edge of a monitor to move the item to another screen. Cross-
  monitor dragging works even when the pointer passes over normal windows.
- **Double-click** an item to open it with the desktop's default handler.
- **Right-click** an item for its context menu:
  - Open
  - Open With…
  - Show in Files
  - Copy Path
  - Rename…
  - Move to Trash
  - Properties
- **Shift+drag** an icon out of Tessera to another application using Wayland
  drag-and-drop. Plain dragging repositions the icon; Shift specifically
  exports the file.
- **Drag files in** from another application. By default, drops are accepted
  over icon cells so the rest of the desktop can remain click-through. Set
  `drop_target = "full"` to accept drops anywhere on the desktop (see the
  trade-off below). Incoming files and folders are copied into `~/Desktop`
  without overwriting existing names, and are pinned near the drop point.

### Keyboard

Click an icon to focus Tessera, then:

| Key | Action |
| --- | --- |
| `Delete` / `Backspace` | Move the selected item to Trash |
| `Enter` | Open the selected item |
| `F2` | Rename the selected item |
| `Menu` / `Shift+F10` | Open the context menu |
| `Escape` | Close the menu/dialog, or clear the selection |

Layer-shell keyboard focus is requested on demand and is established by
clicking an icon. Keyboard actions work on niri/Hyprland-style compositors that
support on-demand keyboard interactivity.

### Click-through

The empty desktop can stay click-through: by default each surface's Wayland
input region is restricted to icon cells, so right-clicks and clicks on bare
wallpaper reach your compositor or another desktop shell. This is why incoming
drops on bare wallpaper only work in `drop_target = "full"` mode.

Context menus and dialogs are drawn inside the desktop surface, so they work
without xdg-popup support.

## Configuration

```toml
# ~/.config/tessera/config.toml
output = "DP-2"          # preferred output for unpinned items
corner = "top-left"      # top-left | top-right | bottom-left | bottom-right
icon_size = 64
font_size = 13
gap = 8
margin = 32
single_click = false
drop_target = "icons"    # "icons" (click-through) or "full" (drop anywhere)
```

`output` does not restrict Tessera to one monitor. Every output gets a surface;
it only decides where new, unpinned items appear first. Without it, Tessera
prefers the output whose logical position is `(0,0)`, then the top-leftmost
output.

With `drop_target = "full"` the entire desktop accepts pointer and drop events.
This enables dropping files anywhere, but empty-desktop clicks no longer pass
through to other desktop components. With `"icons"` (the default), drops are
accepted only over icon cells and bare-wallpaper clicks remain click-through.

A missing file means defaults. A malformed file prints a warning and falls back
to defaults, so a typo never leaves you without a desktop.

## How it compares to a full desktop

This is intentionally a focused desktop icon host, but it now covers the core
desktop file interactions:

**Implemented:** per-monitor surfaces, automatic and persisted positions,
cross-monitor drag, double-click open, context menu, Open With, rename, move to
Trash, Copy Path, properties, Delete/Enter/F2/Escape shortcuts, live folder
watching, drag files in (copy) and out (Shift+drag), panel-aware layout,
click-through wallpaper.

**Still missing compared with Windows/macOS/Plasma:**

- multi-selection (Ctrl/Shift-click and rubber-band selection);
- copy/cut/paste of files between folders, and duplicate;
- undo for rename/delete/move;
- permanent delete (`Shift+Delete`) and an empty-Trash UI;
- in-surface progress/error toasts (errors currently go to the log);
- image/video thumbnails rather than themed icons;
- keyboard arrow-key navigation and full accessibility;
- freeform pixel positioning (positions snap to grid slots);
- separate Desktop folders per monitor (the shared folder is split instead);
- empty-desktop context menu and file paste;
- file-operation details such as permissions/owner, “Open in Terminal”, and
  archive extraction.

## Requirements

- A Wayland compositor implementing `zwlr_layer_shell_v1`: **niri**,
  **Hyprland**, **sway**, **river**, **labwc**, **Wayfire**, etc.
- Rust 1.92 or newer at build time.
- GTK 4.8 or newer and gtk4-layer-shell at build and run time.
  On Arch: `sudo pacman -S gtk4 gtk4-layer-shell`.
- The `gio` command from GLib when opening `.desktop` entries.
- Compositors without layer-shell (notably GNOME/Mutter) are not supported by
  this design.

## Build and install

```sh
cargo build --release
install -Dm755 target/release/tessera ~/.local/bin/tessera
```

Make sure `~/.local/bin` is on the `PATH` your compositor sees, then start it
once per graphical session:

**niri** (`~/.config/niri/config.kdl`):

```kdl
spawn-at-startup "tessera"
```

**Hyprland** (`~/.config/hypr/hyprland.conf`):

```conf
exec-once = tessera
```

**sway** (`~/.config/sway/config`):

```conf
exec tessera
```

If your compositor supports XDG autostart, `contrib/tessera.desktop` can be
copied to `~/.config/autostart/` instead.

## Command line

```
tessera [--output CONNECTOR] [--desktop-dir PATH] [--config PATH] [--state-file PATH]
tessera --help
tessera --version
```

`TESSERA_OUTPUT`, `TESSERA_DESKTOP_DIR` and `TESSERA_STATE_FILE` are accepted
as convenience environment overrides, mainly for development and testing. An
explicit flag always wins over the matching environment variable.

Inspect connector names with `niri msg outputs`, `hyprctl monitors`, or
`swaymsg -t get_outputs`.

## How it works

Each monitor gets:

- a transparent full-output surface on the **bottom** layer with
  `exclusive_zone = -1`. It draws icons and tracks drags in exact global
  coordinates. The bottom layer sits above wallpaper surfaces (including
  DMS/Quickshell backgrounds) and below normal windows.
- a nearly invisible probe surface with `exclusive_zone = 0`. Its configured
  size is the output's available area. Tessera uses the difference to keep
  icons clear of panels; the difference is reserved conservatively on every
  edge, so it works no matter where a bar is.

The folder is watched with `GFileMonitor`, and the grid is rebuilt after a
short debounce when items are added, removed or renamed.

Positions are snap-to-grid slots, not raw pixels, so they survive icon-size
changes. The persistent store is a small TOML file written atomically after
each drop, rename or delete.

Incoming drops and file copies are synchronous. A very large folder drop can
block the UI while it copies; a background copy queue is a future improvement.

## Development

```sh
cargo test
cargo run -- --desktop-dir ./some/test/folder --output DP-2
```

The layout, assignment and state logic is covered by unit tests and does not
need a compositor. Placement is computed by `scene`, `layout` and `assignment`
without any GTK widgets; `ui::host` owns the long-lived state and the other
`ui` modules implement one feature each. The full
implemented/partial/missing inventory and the next-work list live in
[`docs/STATUS.md`](docs/STATUS.md).

## License

MIT
