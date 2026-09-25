# Disk Flashlight

*Русская версия: [README.ru.md](README.ru.md)*

Disk Flashlight is a disk space analyzer for Windows in the spirit of OverDisk. The scanned directory is shown as a sunburst chart: the current directory sits in the centre, each ring outward contains the children of the ring before it, and within every ring the entries are ordered by size, largest first, clockwise from twelve o'clock.

The project is written in Rust and uses [egui](https://github.com/emilk/egui) with the wgpu backend. It is a work in progress; the current state corresponds to the MVP described in [PLAN.md](PLAN.md).

## Features

- Scanning of whole NTFS drives by reading the Master File Table directly when the application runs with administrator rights (the `C:` drive with roughly 920 000 entries is scanned in under one second). The MFT scan counts hard links once and includes system areas such as `System Volume Information` and the NTFS metafiles.
- Parallel directory walk for subdirectories, non-NTFS volumes and non-elevated runs (the same `C:` drive takes about 8 seconds on a cold cache and under 3 seconds on a warm one). The **Admin** toolbar button restarts the application elevated.
- Sunburst chart with up to seven rings, rendered as a single cached GPU mesh; sectors thinner than one pixel are dropped, so the chart stays responsive regardless of tree size.
- Tooltip with name, logical and allocated size, and directory and file counts for the sector under the cursor.
- Click on a directory sector to make it the centre; right-click or click the centre to go up; back, forward and up navigation with history.
- Directory tree on the left, synchronised with the chart in both directions.
- Physical (cluster-rounded, compressed and sparse files taken into account) or logical size as the chart metric.
- Status bar with directory and file counts, logical size, allocated size and slack for the current root.

## Building

Requirements: a stable Rust toolchain (MSVC target) and the Visual Studio Build Tools with the C++ workload.

```
cargo build --release
```

The binary is produced at `target\release\disk_flashlight.exe`.

## Usage

```
disk_flashlight.exe              # opens the window; a drive is picked from the toolbar
disk_flashlight.exe D:\Projects  # scans the given path on start-up
disk_flashlight.exe --bench C:\  # command-line benchmark of the scanner and layout
disk_flashlight.exe --bench C:\ --walk  # the same benchmark with the MFT scanner disabled
```

Setting the environment variable `RUST_LOG=disk_flashlight=debug` prints per-phase timings of the MFT scanner.

Keyboard shortcuts: `Backspace` goes up, `Alt+Left` and `Alt+Right` move through history, `F5` rescans. The mouse wheel over the chart changes its zoom.

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

## Roadmap

Planned: saving and loading scan results, and a context menu for opening items in Explorer.

## License

MIT
