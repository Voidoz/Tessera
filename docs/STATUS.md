# Tessera — project status

Snapshot: **2026-11-24**, version `0.1.0`.

This document is the handoff point for future sessions. It separates what is
actually implemented and verified from what is partial, and lists the known
gaps. `README.md` is the user-facing guide; `AGENTS.md` contains the short
version for coding agents.

## What this is

A Wayland desktop **icon host**, not a desktop environment. It connects to
`zwlr_layer_shell_v1`, covers outputs with transparent bottom-layer surfaces,
renders the contents of `~/Desktop`, and provides automatic placement, pinned
positions, context menus, keyboard actions, Trash, and limited drag-and-drop.
It deliberately does not manage windows, draw wallpaper, provide a panel, or
replace the compositor's own desktop shell.

Current technical shape:

- Rust + GTK 4 (feature `v4_8`) + `gtk4-layer-shell`.
- One **icon surface** per output on the bottom layer with
  `exclusive_zone = -1`, so its origin is exactly the output origin and
  cross-monitor drag coordinates are exact.
- One transparent **probe surface** per output on the bottom layer with
  `exclusive_zone = 0`; its configured size reveals the available area after
  panels. The difference is reserved conservatively on every edge so icons
  never sit under a panel.
- Input regions are restricted to icon cells, so the rest of the desktop stays
  click-through. `drop_target = "full"` opts out of that for whole-desktop
  file drops.
- Positions are grid slots, not pixels. Unpinned items are re-laid out every
  render; pinned items persist in TOML.

## Implemented and verified

"Verified" means exercised manually on niri 26.04 with temporary virtual
pointer/keyboard clients, or covered by unit tests. It does not mean every
compositor or input device was tested.

### Multi-monitor and layout

- A layer surface on every connected output, not just one.
- Shared `~/Desktop` split across outputs: each item appears on exactly one
  output; it is not mirrored.
- Automatic placement: folders first, locale-aware filename collation,
  column-major grid from the configured corner.
- Preferred-output first auto-placement (`output`), then spill to the other
  outputs in layout order.
- Grid compaction down to 32 px when the aggregate capacity is too small.
- Bottom corners grow upward/right; top corners grow downward/right.
- Panel-aware safe area using the probe surface.
- Pinned grid positions in a state file.
- Cross-monitor drag with a live target outline; tested DP-2 → DP-1 through a
  monitor showing a fullscreen game.
- Drag is snap-to-grid; occupied targets choose the nearest free slot.

### Context menu and file operations

- In-surface right-click menu (not an xdg-popup), with:
  - Open
  - Open With…
  - Show in Files
  - Copy Path
  - Rename…
  - Move to Trash
  - Properties
- `Open With…` builds a list from
  `gio::AppInfo::recommended_for_type`, filters hidden apps, and launches with
  the file URI. The chooser has been verified visually; actual third-party app
  launch has not been exercised.
- `Rename…` validates a single path component, preserves the extension when
  selecting the stem, rejects collisions, moves the pinned position with
  `StateStore::rename`, and refreshes. Verified end-to-end.
- `Move to Trash` uses `gio::File::trash_async`; pinned state is removed after
  success. Verified with an isolated `XDG_DATA_HOME`.
- `Delete` and `Backspace` do the same as Move to Trash.
- `Copy Path` puts the absolute path on the clipboard. Verified with
  `wl-paste`.
- `Properties` shows name, type, size, modified time, path, and symlink target.
  Verified visually.
- `Show in Files` opens the parent folder through the default URI handler.
  Implemented but not manually exercised.
- Context menus and dialogs use a full-surface scrim and therefore are verified
  on layer-shell without popup support.

### Keyboard

- Icon surfaces request `KeyboardMode::OnDemand`; clicking an icon presents the
  surface and focuses the button.
- Shortcuts:
  - `Delete` / `Backspace` → Trash
  - `Enter` → open
  - `F2` → rename
  - `Escape` → close overlay / clear selection
  - `Menu` or `Shift+F10` → context menu
- Delete, F2, Enter, and Escape were exercised with a virtual keyboard on
  niri. `Menu` and `Shift+F10` are implemented but not manually exercised.
- Key handling is a capture-phase controller on each output window, so Enter is
  handled before the focused button can activate itself.
- Selecting an icon focuses the window; `clear_selection` is also wired to
  empty-surface clicks when the surface receives them.

### Drag and drop

- **Out:** `Shift`+drag exports a file URI list (`text/uri-list`) via
  `GtkDragSource`. Verified against a normal GTK drop target. Plain drag stays
  inside Tessera and repositions; this modifier split is deliberate.
- **In:** each output has a `GtkDropTarget` for `GdkFileList`. Files and folders
  are copied into the Desktop folder without overwriting; existing names get
  `(1)`, `(2)`, etc. Dropped items are pinned at the nearest free slots to the
  drop point.
- **Incoming drop area:** default `drop_target = "icons"` accepts drops only
  over icon cells, because all other surface area is outside the input region.
  `drop_target = "full"` makes the entire output a drop target and accepts
  drops anywhere, at the cost of click-through.
- Both incoming-drop paths were verified: over an icon in the default mode and
  on empty desktop area in full mode, using a GTK source.
- Copied directories recurse; symlinks are preserved. Remote/non-local URIs are
  logged and skipped.

### Runtime and robustness

- `GFileMonitor` live refresh with debounce when Desktop contents change.
- Monitor add/remove and geometry/scale-change handling (implemented; unplug and
  scale changes not manually tested).
- State file writes are atomic (temp file + rename).
- A stale timeout `SourceId::remove()` panic was found and fixed by clearing the
  stored source when it fires.
- Unit tests cover layout, assignment, config/clamping, Desktop scanning,
  sorting behavior, and state round-tripping/rename.
- Release binary builds with `cargo build --release`.
- No test processes or layer surfaces are left running.

## Partially added / caveats

These are not complete parity features; future work should treat them as
subject to change.

1. **Drag out requires Shift.** It is not the modifier-free drag expected from
   Windows/macOS/Plasma because plain drag is already used for grid
   repositioning. There is no custom drag icon. Only `text/uri-list` is offered.
2. **Drag in is area-limited by default.** Whole-desktop drops require
   `drop_target = "full"`, which disables click-through and blocks clicks from
   reaching other desktop components. There is no automatic "expand input
   region only while a drag is active" mechanism.
3. **Incoming DND interoperability is only lightly tested.** A GTK
   `GdkFileList` source was used. File managers that offer only raw
   `text/uri-list` depend on GTK's conversion and were not tested.
4. **Copies are synchronous.** A large file or directory blocks the UI, with no
   progress, cancel, or conflict dialog.
5. **Incoming drops always copy.** There is no move semantics, and no handling
   of the source's preferred action beyond accepting COPY.
6. **No drag icon/ghost for external export.** Internal grid drags show a
   source dim and target outline, not a floating icon.
7. **Context menu is mouse-oriented.** `Menu`/`Shift+F10` opens it, but arrow
   key navigation inside menus/dialogs is not implemented.
8. **Open With launch is untested.** The chooser and app list are implemented;
   actually opening a third-party app from it was not exercised.
9. **Show in Files is untested.** It opens the parent folder, not the file
   selected in the file manager.
10. **Properties is basic.** No permissions, owner/group, inode/link count,
    checksum, or percentage of disk.
11. **Trash has no confirmation/undo.** Successful trash is silent; failures
    are only logged. There is no Empty Trash UI and no permanent delete.
12. **Single selection only.** There is no Ctrl/Shift-click, rubber band, batch
    delete, batch move, or multi-item drag.
13. **Keyboard focus is compositor-dependent.** Verified on niri with on-demand
    layer-shell keyboard support; other compositors may focus differently.
14. **Panel change handling is reactive.** The probe is polled every 500 ms.
    There can be a brief stale safe area after a panel auto-hides/shows.
15. **Monitor hotplug and scale changes are untested.** The code listens for
    monitor model changes and geometry/scale changes but has not been exercised
    with an actual unplug or scale change.
16. **High-DPI is untested.** Geometry is in logical pixels; no scale testing
    beyond scale 1 was done.
17. **Pinned slots can become invalid.** If a resolution/panel/icon-size change
    reduces capacity, a pinned slot outside the new grid falls back to auto
    placement. The stale entry remains in the state file.
18. **State is keyed by path.** Rename is handled; moving a file outside
    Tessera leaves a stale pinned entry (harmless, but it accumulates).
19. **No error toast.** Most failures go to stderr/log only. Rename is the one
    operation with an inline error message.
20. **No config hot reload or settings UI.** Edit the TOML and restart.
21. **No packaging.** No AUR/Flatpak/deb packaging or XDG autostart installer;
    README shows the manual steps.

## Not implemented

This is the full backlog from the desktop-parity discussion, including items
that are intentionally out of scope for the current design.

### Selection and clipboard

- Multi-selection (Ctrl/Shift-click, rubber-band, select all).
- Copy, cut, paste, duplicate, and move-to-folder operations.
- File clipboard integration (`text/uri-list`,
  `x-special/gnome-copied-files`).
- Empty-desktop **Paste** and an empty-desktop context menu.
- Undo for rename, trash, move, or copy.

### File operations

- Permanent delete (`Shift+Delete`) with confirmation.
- Empty Trash / restore from Trash UI.
- Progress dialogs, cancellation, and overwrite/rename conflict prompts for
  copies.
- Asynchronous copy queue.
- Open in Terminal.
- Permissions/owner/group in Properties.
- Archive creation/extraction.
- “New Folder” / “New Document” on the desktop.
- Batch rename.

### Desktop features

- Separate Desktop folder per monitor (the shared folder is split instead).
- Freeform pixel positioning; positions are grid slots.
- Swap-on-drop when a slot is occupied; currently the nearest free slot is
  chosen.
- Auto-arrange toggle, sorting modes, and “clean up” command.
- Special desktop icons: Home, Trash, mounted volumes, network locations.
- Image/video/document thumbnails; themed icons only.
- File emblems/badges (symlink, read-only, executable, cloud status).
- Tooltips for truncated names.
- Drag-to-edge auto-scroll and multi-item drag.
- Multiple virtual desktops with distinct pinned layouts; layer surfaces span
  workspaces.

### Keyboard and accessibility

- Arrow-key navigation between icons.
- Keyboard navigation inside context menus and dialogs.
- Tab/accessibility tree review, screen-reader labels/roles, and focus
  indicators.
- Full keyboard-only operation without a pointer.
- IME/text input in rename beyond basic GtkEntry behavior.

### Robustness and ecosystem

- Automated GUI/integration tests for drag/drop, keyboard, menus, monitor
  hotplug, and panels.
- Tests against Hyprland, sway, river, labwc, Wayfire, KDE, and non-GTK file
  managers.
- Async file operations and structured error surface.
- Config hot reload and a settings UI.
- Localization and RTL layout.
- Packaging and install script; migration tool for renames.
- Security review of `.desktop` launching, trust, and untrusted drop handling.
- Remote URI support for incoming drops.
- Wayland DND modifier-free export and an actual drag icon.
- Detection of compositor support for on-demand keyboard focus with a graceful
  fallback.

## Architecture map

| File | Responsibility |
| --- | --- |
| `src/main.rs` | CLI, config/state path selection, GTK application, monitors |
| `src/ui.rs` | Layer surfaces, probes, icon widgets, drag/drop, context menus, dialogs, keyboard, focus |
| `src/layout.rs` | Pure grid geometry, panel insets, compaction, slots |
| `src/assignment.rs` | Pinned-first, auto-fill placement across outputs |
| `src/state.rs` | TOML position store, atomic save, rename/remove |
| `src/desktop.rs` | Desktop scan, `.desktop` parsing, MIME/icon resolution, launch |
| `src/config.rs` | TOML config, CLI parsing, drop-target mode |
| `contrib/tessera.desktop` | Optional XDG autostart entry |

Important invariants:

- Icon surface: bottom layer, `exclusive_zone = -1`, all anchors, transparent.
- Probe surface: bottom layer, `exclusive_zone = 0`, transparent, input region
  empty.
- Icon cells are the only input by default; overlays temporarily expand input
  to the full surface.
- Global drag coordinates are output geometry origin + surface-local pointer.
- Plain drag = internal re-grid; Shift+drag = external file DND.
- Incoming drops are copies, never moves.

## State and configuration

Default config:

```toml
# ~/.config/tessera/config.toml
output = "DP-2"        # preferred auto-placement output
corner = "top-left"
icon_size = 64
font_size = 13
gap = 8
margin = 32
single_click = false
drop_target = "icons"  # "icons" | "full"
```

Default position store:

```toml
# ~/.local/state/tessera/positions.toml
version = 1

[[items]]
path = "/home/me/Desktop/Notes.txt"
output = "DP-2"
column = 2
row = 3
```

Overrides:

- `--output`, `--desktop-dir`, `--config`, `--state-file`
- `TESSERA_OUTPUT`, `TESSERA_DESKTOP_DIR`, `TESSERA_STATE_FILE`

## Test commands

```sh
cargo test
cargo clippy --all-targets
cargo build --release
```

The current test suite has 25 passing unit tests. It does not cover UI
interaction. Manual testing used temporary wlr virtual-pointer and
virtual-keyboard clients; those helpers are not checked in.

## Suggested next steps, in order

1. **Multi-selection + batch trash.** Biggest desktop-feel gap; enables batch
   operations without a clipboard.
2. **Copy/cut/paste and duplicate**, including file clipboard and an
   empty-desktop Paste menu item. This also makes incoming/outgoing DND feel
   less isolated.
3. **Undo** for trash/rename/move (a bounded action log).
4. **Async file operations** with progress and conflict prompts; remove the
   synchronous copy freeze.
5. **Incoming DND usability**: consider a small always-input drop hotspot or a
   well-explained full mode; investigate whether a temporary input-region
   expansion can be triggered by the compositor.
6. **Keyboard navigation** (arrows, menu arrows, focus ring) and an
   accessibility pass.
7. **Thumbnails** and file emblems.
8. **Integration tests** for layer surfaces and input regions, ideally with a
   nested compositor such as `niri --headless`/`sway --headless` or a custom
   wayland test harness.
9. **Packaging and autostart installer**, then test on Hyprland/sway.

## Session handoff notes

- Do not test destructive operations against the real `~/Desktop`. Use a
  fixture directory and `--state-file`.
- Use `XDG_DATA_HOME` pointing at a temporary directory when testing Trash, so
  test files do not land in the user's real Trash.
- On this machine, test on **DP-2** and avoid disturbing the center/left
  displays; the user may be gaming or using them. `niri msg layers`, `grim -o
  DP-2`, and synthetic wlr input helpers were used.
- Keep `README.md` and this file in sync when behavior changes.
