# AGENTS.md — Tessera handoff

**Read `docs/STATUS.md` first.** It is the authoritative implemented/partial/
missing inventory. `README.md` is user-facing.

## Scope

This is a **Wayland desktop icon host**, not a desktop environment. It owns
bottom-layer surfaces, shows `~/Desktop`, and does icon layout, pinned
positions, context menus, keyboard actions, Trash, and limited drag/drop. It
does not manage windows, panels, wallpaper, or other application windows.

## Build and check

```sh
cargo test
cargo clippy --all-targets
cargo build --release
```

Requires GTK 4.8+, gtk4-layer-shell, and Rust 1.92+. The release binary is
`target/release/tessera`.

## Architecture invariants

- One **icon window** and one **probe window** per output.
- Icon window: bottom layer, all anchors, `exclusive_zone = -1`, transparent.
  Its origin is exactly the output origin.
- Probe window: bottom layer, `exclusive_zone = 0`, transparent, empty input
  region. Its configured size gives the panel-free area.
- Input region is restricted to icon cells by default, preserving click-through.
  `drop_target = "full"` opts into full-surface input.
- Menu/dialog overlays are drawn inside the desktop surface, not as xdg-popups.
- Plain drag rearranges icons; `Shift`+drag exports via Wayland DND.
- Incoming drops are **copies** and are pinned near the drop point.
- Pinned state lives in `~/.local/state/tessera/positions.toml` and is keyed by
  path.
- All UI mutation happens on the GTK main thread.

## Safe manual testing

Do not use the user's real Desktop for destructive testing.

```sh
mkdir -p /tmp/tessera-test/Desktop
# populate with test files
./target/release/tessera \
  --desktop-dir /tmp/tessera-test/Desktop \
  --state-file /tmp/tessera-test/positions.toml \
  --output DP-2
```

For Trash tests also isolate the trash directory:

```sh
XDG_DATA_HOME=/tmp/tessera-test/data ./target/release/tessera ...
```

On this machine, use **DP-2** for test surfaces and screenshots. Do not take
over the center/left displays; the user may be gaming or using other windows.
Avoid `pkill -f` patterns that match the test command itself.

Synthetic input used in earlier sessions:

- `zwlr_virtual_pointer_v1` for absolute drag/click sequences;
- `zwp_virtual_keyboard_v1` for focus and key testing (keymap must be sent
  before keys; modifiers must be sent explicitly for Shift/Ctrl).

Those helper C programs were temporary and are not checked in. They are useful
to recreate when verifying keyboard or drag behavior.

## Current known sharp edges

- DND out needs Shift and provides only `text/uri-list`.
- DND in only works over icon cells unless `drop_target = "full"`.
- Copies are synchronous.
- Keyboard focus is verified on niri only.
- No multi-selection, clipboard operations, undo, thumbnails, or keyboard arrow
  navigation.
- Monitor hotplug/high-DPI/panel auto-hide paths are implemented but not
  manually exercised.

## Next likely work

See “Suggested next steps” in `docs/STATUS.md`. The closest-to-desktop-parity
work is multi-selection, copy/cut/paste, undo, and async file operations.

When behavior changes, update `README.md` and `docs/STATUS.md` in the same
session.
