# Telorgon File Explorer

A native Telorgon file manager with integrated open/save/folder dialogs and a download queue. It uses the sibling `../telorgon/crates/telorgon` library and runs on the Telorgon desktop's Wayland session.

```sh
cargo run --release
cargo run --release -- /home/aku/Downloads
cargo run --release -- --pick
cargo run --release -- --folder
cargo run --release -- --save --name report.txt
cargo run --release -- --download https://example.org/report.pdf
```

The interface follows Windows 11 File Explorer in dark mode, with full-width tabs, address/search boxes, a compact command bar, pinned navigation, sortable details columns, and large icons. Open/save dialogs use the same browser and destination controls.

The app provides folder navigation, search, sorting, list/grid views, multiple selection, file previews and properties, new folders, rename, clipboard operations, trash/restore, bookmarks, and downloads. File operations and downloads run on worker threads. Destructive actions request confirmation in the app.

## Telorgon app integration

Applications can launch the same executable with `--request-stdin`, write one JSON request to stdin and close stdin. The process writes one JSON response to stdout. Exit status is `0` for selection, `1` for cancellation, and `2` for an error; diagnostics go to stderr. JSON input and output are limited to 256 KiB.

```sh
printf '%s\n' '{"mode":"save","title":"Save report","app_id":"dev.telorgon.notes","current_folder":"/home/aku/Documents","current_name":"report.txt","filters":[{"name":"Text","rules":[{"kind":0,"pattern":"*.txt"}]}]}' \
  | target/release/telorgon-file-explorer --request-stdin
```

Modes are `open`, `folder`, `save`, `save_files`, and `download`. `download` accepts `source_url` and a suggested `current_name`; it asks for the destination, runs the transfer, and returns the destination after the user accepts the completed download. Clients can instead use `save` to choose a destination and perform the transfer themselves. `multiple` enables several items for open/folder selection. `accept_label`, `title`, `app_id`, `current_folder`, `current_file`, `current_name`, `filters`, `current_filter`, and `choices` carry dialog preferences. `parent_window` and `modal` are preserved in the protocol; the current Telorgon managed window API does not attach dialogs to external parent windows or enforce modal parenting.

Filters use `{"name":"Images","rules":[{"kind":0,"pattern":"*.png"},{"kind":1,"pattern":"image/*"}]}`. Kind `0` supports filename globs including `*`, `?`, and bracket ranges. Kind `1` uses common extension/MIME mappings; this is not a content sniffing database. Choices have `id`, `label`, `options` as `[id,label]` pairs, and `selected`; an empty options list represents a boolean choice.

The response includes `cancelled`, `uris`, `paths`, `writable`, `current_filter`, and `choices`. **Use `uris` as the authoritative selected destinations.** Percent-encoded `file://` URIs preserve arbitrary Unix filename bytes; `paths` are display strings. `current_folder_bytes`, `current_file_bytes`, and `files_bytes` carry raw bytes when UTF-8 strings cannot represent a path. Raw JSON byte arrays omit the terminating NUL used by D-Bus. `save_files` takes `files` or `files_bytes`, chooses a destination folder, and returns destinations in order with numbered filename suggestions for existing collisions. The dialog does not create empty destination files.

## Desktop integration

```sh
cargo build --release
target/release/telorgon-file-explorer --portal
```

`--portal` serves `org.freedesktop.impl.portal.FileChooser` version 4 (`OpenFile`, `SaveFile`, `SaveFiles`) at `/org/freedesktop/portal/desktop`, on bus name `org.freedesktop.impl.portal.desktop.telorgon.FileExplorer`. It checks that backend calls come from the owner of `org.freedesktop.portal.Desktop`. The existing Telorgon ScreenCast backend retains `org.freedesktop.impl.portal.desktop.telorgon`.

Each request launches a separate Telorgon dialog. `org.freedesktop.impl.portal.Request.Close` cancels and reaps that request's process. The request owner is checked; abandoned requests close when the portal frontend disconnects. Up to 16 dialogs can run concurrently. The backend also exposes `org.freedesktop.FileManager1` at `/org/freedesktop/FileManager1`, with `ShowItems`, `ShowFolders`, and `ShowItemProperties`. An already-running file manager can retain that standard bus name without preventing FileChooser service startup.

Install assets explicitly with `bash packaging/install.sh`. It copies the binary, desktop entry, D-Bus activation services, `.portal` registration, and an example configuration under `~/.local`. For a package, use `--prefix /usr --destdir /absolute/staging/root`. The script does not select a default file manager, overwrite portal preferences, or restart services.

Portal discovery depends on the frontend's installed data directory; a local `.portal` file alone may not be discovered. Install `packaging/telorgon-file-explorer.portal` into the frontend's `DATADIR/xdg-desktop-portal/portals` (commonly `/usr/share/xdg-desktop-portal/portals`) when packaging the full Telorgon system. Merge this preference into the session's existing `xdg-desktop-portal/*-portals.conf`:

```ini
[preferred]
org.freedesktop.impl.portal.FileChooser=telorgon-file-explorer
org.freedesktop.impl.portal.ScreenCast=telorgon
```

Preserve any existing defaults and interfaces. The desktop session must propagate its Wayland display and desktop variables to the D-Bus activation environment; restarted frontend/backend services then use the configured Telorgon chooser. The sibling `test-shell/portal-startup.sh` currently explicitly selects/checks GTK FileChooser, so its session bootstrap must also be updated when adopting this backend in that shell.

The implementation follows the [FileChooser backend contract](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.FileChooser.html), [Request cancellation contract](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.Request.html), and [backend registration instructions](https://flatpak.github.io/xdg-desktop-portal/docs/writing-a-new-backend.html).

## Verification

```sh
cargo test --offline
```

Tests cover filesystem operations, request validation, percent-encoded raw-byte paths, filter patterns, portal option decoding, cancellation ownership, bounded subprocesses, and download behavior. Visual interaction requires an available Wayland session. Parent-window attachment, network mounts, and content-based MIME detection remain future integration work.
