# Disk Flashlight

**English** | [Русский](README.ru.md)

Disk Flashlight is a disk space analyzer for Windows in the spirit of OverDisk. The scanned directory is shown as a sunburst chart: the current directory sits in the centre, each ring outward contains the children of the ring before it, and within every ring the entries are ordered by size, largest first, clockwise from twelve o'clock.

![Disk Flashlight showing drive C: as a sunburst chart, with the directory tree on the left](images/screenshot.png)

The project is written in Rust and uses [egui](https://github.com/emilk/egui) with the wgpu backend. It is a work in progress; the current state corresponds to the MVP described in [PLAN.md](PLAN.md).

## Features

- Scanning of whole NTFS drives by reading the Master File Table directly when the application runs with administrator rights (the `C:` drive with roughly 920 000 entries is scanned in under one second). The MFT scan counts hard links once and includes system areas such as `System Volume Information` and the NTFS metafiles.
- Parallel directory walk for subdirectories, non-NTFS volumes and non-elevated runs (the same `C:` drive takes about 8 seconds on a cold cache and under 3 seconds on a warm one). When a whole NTFS drive is scanned without administrator rights, the **Fast scan** toolbar button (marked with the UAC shield) restarts the application elevated and rescans the drive through the MFT.
- Sunburst chart with up to seven rings, rendered as a single cached GPU mesh; sectors thinner than one pixel are dropped, so the chart stays responsive regardless of tree size.
- Items too small to be told apart (under about four points of arc) are merged into one neutral **N smaller items** sector per directory; zooming in splits the group as items become wide enough, and a click on the group zooms in. Edges are smoothed with 4x multisampling.
- Tooltip with name, logical and allocated size, and directory and file counts for the sector under the cursor.
- Click on a directory sector to make it the centre; a click on the centre goes up; back, forward and up navigation with history.
- Context menu on right-click with **Open in Explorer** and **Properties** for the sector under the cursor, or for the current directory when the centre is right-clicked.
- Directory tree on the left, synchronised with the chart in both directions. While the mouse moves over the chart, the tree is temporarily expanded to the hovered item; the **Follow in tree** toolbar option (enabled by default) turns this off.
- Two colour schemes selectable in the toolbar: **by size**, where the largest item among its siblings is red and smaller ones shift towards yellow, with colours becoming paler towards the rim (as in OverDisk), and **by level**, where the colour depends on the ring.
- Physical (cluster-rounded, compressed and sparse files taken into account) or logical size as the chart metric.
- Status bar with directory and file counts, logical size, allocated size and slack for the current root.

## Building

Requirements: a stable Rust toolchain (MSVC target) and the Visual Studio Build Tools with the C++ workload.

```
cargo build --release
```

The binary is produced at `target\release\disk_flashlight.exe`.

## Installer and release

The release script builds the executable and a per-user installer (Inno Setup 6 is required):

```
python tools/make_release.py              # tests, release build, installer
python tools/make_release.py --no-tests   # the same without cargo test
```

The installer is written to `dist\Disk_Flashlight_<version>_Setup.exe`, together with a portable archive, `dist\Disk_Flashlight_<version>_portable.zip`, which contains only the program in a `Disk Flashlight` folder and runs without installation; the version is taken from `Cargo.toml`. Installation does not require administrator rights: the program is placed in `%LOCALAPPDATA%\Programs\Disk Flashlight`. An optional task adds the **Analyze with Disk Flashlight** item to the Explorer context menu of folders and drives; the entry is removed on uninstallation.

Releases on GitHub are built by the **Release** workflow (`.github/workflows/release.yml`). After the version in `Cargo.toml` is updated and committed, pushing a matching tag publishes a release with the installer and the portable archive:

```
git tag v0.1.0
git push origin v0.1.0
```

The workflow stops if the tag does not match the version in `Cargo.toml`. A manual run from the Actions tab builds the same files as a workflow artifact without publishing a release. The executables are not code-signed, so Windows SmartScreen may warn about an unknown publisher on first launch.

## Usage

```
disk_flashlight.exe              # opens the window; a drive is picked from the toolbar
disk_flashlight.exe D:\Projects  # scans the given path on start-up
disk_flashlight.exe --bench C:\  # command-line benchmark of the scanner and layout
disk_flashlight.exe --bench C:\ --walk  # the same benchmark with the MFT scanner disabled
disk_flashlight.exe --version    # prints the version
disk_flashlight.exe --mft C:\    # restarts as administrator (UAC prompt) and scans C: through the MFT
```

Setting the environment variable `RUST_LOG=disk_flashlight=debug` prints per-phase timings of the MFT scanner.

The `--mft` option requests administrator rights only where they speed up the scan: for the root of an NTFS drive, or when no path is given. For a subdirectory or another file system, and when the UAC prompt is declined, the program starts normally and walks the directories.

The version is shown in the window title.

Keyboard shortcuts: `Backspace` goes up, `Alt+Left` and `Alt+Right` move through history, `F5` rescans. A rescan keeps the current folder and the navigation history; if the folder no longer exists, the view moves to its closest remaining parent. The mouse wheel zooms the chart towards the cursor; dragging with the middle mouse button pans it, and a middle-button double click restores the initial view.

## Project layout

| Path | Purpose |
|---|---|
| `src/model.rs` | Arena tree packed in depth-first order with children sorted by size |
| `src/scan/mft.rs` | NTFS Master File Table reader and parser |
| `src/scan/walk.rs` | Parallel directory walk (rayon over `read_dir`) |
| `src/scan/win.rs` | Win32 helpers: drive enumeration, cluster size, compressed sizes |
| `src/layout.rs` | Sunburst layout and hit testing |
| `src/render.rs` | Tessellation of sectors into an `egui::Mesh`, palette |
| `src/ui/` | Chart widget, directory tree, toolbar and status bar |
| `src/history.rs` | Back/forward navigation history |
| `tools/` | Installer script (`setup.iss`) and release script (`make_release.py`) |

## License

MIT, see [LICENSE](LICENSE).
