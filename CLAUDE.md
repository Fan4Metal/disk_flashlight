# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Disk Flashlight is a Windows-only disk space analyzer (OverDisk-style sunburst chart) written in Rust with egui/eframe 0.36 on wgpu. `PLAN.md` holds the original design plan and status; `README.md` / `README.ru.md` are the user-facing docs and must be kept in sync (both languages, cross-linked, formal tone).

## Commands

```
cargo build --release                # binary at target\release\disk_flashlight.exe
cargo test                           # unit tests (model, layout hit-test, history, format)
cargo test layout::                  # single module's tests
cargo clippy --release               # must stay warning-free
cargo run --release -- --bench D:\   # scanner + layout benchmark, prints method, timings, top-level dirs
cargo run --release -- --bench D:\ --walk   # same, MFT scanner disabled
cargo run --release -- D:\Projects   # open the GUI scanning a path on start
```

GitHub releases: `.github/workflows/release.yml` runs `make_release.py` on `windows-latest` for tags `v*` (tag must equal `v` + Cargo.toml version, otherwise the job fails) and publishes `dist/*.exe`; `workflow_dispatch` builds an artifact only. Creating or pushing a tag publishes a public release, so do it only when the user asks.

Release: `python tools/make_release.py [--no-tests]` runs tests, `cargo build --release`, exports the icon (`disk_flashlight.exe --export-icon target\app.ico`) and compiles `tools/setup.iss` with Inno Setup 6 into `dist\`, passing the version from `Cargo.toml` via `/DMyAppVersion` (the `.iss` is not edited). The installer is per-user (`PrivilegesRequired=lowest`) and registers the context-menu verb under `HKCU\Software\Classes\{Directory,Drive}\shell\DiskFlashlight`. Explorer passes a drive as `"C:\"`, which Windows argument parsing turns into `C:"`; `main.rs::normalize` repairs it, so keep that when touching argument handling. The script fails early if `target\release\disk_flashlight.exe` is running (the file is locked).

Toolchain: stable MSVC Rust plus VS 2022 Build Tools. In the Bash tool, `cargo` is not on PATH; prefix commands with `export PATH="$HOME/.cargo/bin:$PATH"`. A running `disk_flashlight.exe` locks the release binary, so stop it before rebuilding (`Get-Process disk_flashlight | Stop-Process`).

Heredocs containing Rust code break the Bash tool (lifetime apostrophes); write source files with the Write tool or a Python script.

## Architecture

Data flows in one direction: `scan` → `model` → `layout` → `render` → `ui`.

- **`scan/mod.rs::scan`** picks the scanner: volume roots (`C:\`) go to **`scan/mft.rs`** first, which opens `\\.\C:` raw, locates `$MFT` via `FSCTL_GET_NTFS_VOLUME_DATA`, streams the table in 8 MiB blocks on a reader thread and parses 1 KiB FILE records in parallel (fixups, `$FILE_NAME` for name/parent, unnamed `$DATA` for sizes; extension records are merged into their base record; DOS 8.3 names skipped; hard links counted once). It needs administrator rights, so without elevation or on non-NTFS it fails and `scan` falls back to the walk, resetting progress counters. Its parsing is unit-tested on synthetic records built in the test module; testing against a real volume requires an elevated run (`Start-Process -Verb RunAs`, which shows a UAC prompt the user must confirm). The MFT result includes metafiles (`$MFT`, `$LogFile`, `$Extend`) and `System Volume Information`, so totals differ from the walk.
- **`scan/walk.rs`** walks directories with `std::fs::read_dir` under rayon (on Windows `DirEntry::metadata()` is free, taken from `WIN32_FIND_DATAW`). Symlinks and junctions are skipped. It builds a nested `RawDir` tree and reports progress through atomics in `Progress`. `scan/mod.rs` runs it on a thread and hands the result back over a bounded channel; `scan/win.rs` wraps the few Win32 calls (drive list, cluster size, compressed size). `windows-sys` features are kept minimal (`Win32_Foundation`, `Win32_Storage_FileSystem`, `Win32_System_IO`, `Win32_UI_Shell`); `DRIVE_*` constants are hard-coded to avoid another feature. `win::relaunch_elevated` restarts the app via `ShellExecuteW("runas")`; arguments go through `app::quote_arg` because `CommandLineToArgvW` treats `\"` as an escaped quote.
- **`icon.rs`** draws the app icon procedurally with no dependencies; `build.rs` includes it via `#[path]` to write a multi-size `.ico` and embeds it with `winresource`, and `main.rs` uses the same pixels for the window icon.
- **`model.rs`** packs the raw tree into an arena `Vec<Node>` in DFS order. Every node's children are a contiguous range **sorted by logical size descending**, and names live in one string arena. This ordering is what makes "largest first" free downstream; anything that iterates children may rely on it. For the physical metric the order can differ slightly, so layout and the tree re-sort a copy when `Metric::Physical` is active.
- **`layout.rs`** converts a subtree rooted at any node into rings of `Sector { node, a0, a1 }` (angle 0 = top, clockwise). Ring radii follow a geometric series (`ring_shrink`). The first child whose arc is shorter than `merge_arc_px` and all smaller ones become one group sector (`Sector::count > 0`, `node` = the parent, `group_size`; a single leftover is drawn as itself; groups under `min_arc_px` are dropped). `build` also takes the visible rect: `ViewBounds` (polar r range + angular range) culls sectors and whole subtrees off screen, and `render` tessellates only the visible pieces (`ViewBounds::clip`), so work stays bounded by the window even at 200x zoom. `Layout::hit_test` finds the ring by radius and binary-searches the angle; `Layout::index` maps node id → sector for external highlighting (groups are not indexed).
- **`render.rs`** tessellates all sectors into a single `egui::Mesh` (one draw call) and defines the palette. Two modes (`ColorMode`): **Size** (default, OverDisk's size-encoded scheme) takes the hue from `Sector::rel`, the size relative to the largest sibling computed in `layout::emit` (largest = red, small = yellow), and fades saturation towards white linearly with ring depth; **Depth** takes the hue from the ring and alternates brightness. Files get lower saturation in both. Colours go through egui's linear-space `Hsva`, which is what makes them pastel; the user wants to keep that. Helper shapes for hover outlines and guide circles live here too.
- **`ui/chart.rs`** owns the cached `Layout` + `Arc<Mesh>` keyed on (root, metric, centre, radius, view rect) and rebuilds only when the key changes (every frame while panning or zooming, which is cheap thanks to culling). It draws the chart, handles hover (instant `Tooltip::always_open`, not the delayed `on_hover_ui`), wheel zoom towards the cursor (`zoom`, up to 200x), middle-button panning (`pan`, reset on root change or middle double click), click-to-navigate and click-on-group-to-zoom, returning a `ChartAction` for `App` to apply.
- **`ui/tree.rs`** is a virtualised directory-only tree (`ScrollArea::show_rows`) with its own expanded set; `reveal()` expands ancestors when the root changes. Expand/collapse arrows are painted as triangles because the default fonts lack ▲/▼ glyphs.
- **`ui/toolbar.rs`** implements toolbar and status bar as `impl App` methods. **`app.rs`** is the eframe `App` (note egui 0.36 API: `fn ui(&mut self, ui, frame)` with `egui::Panel::top/left/bottom(...).show(ui, ...)` and `CentralPanel::no_frame()`, not the older `update(ctx)` / `SidePanel` API). Navigation state is `history::History` (root, back, forward), kept UI-free so it is unit-testable.

## Conventions

- Commits in this repository end with a `Co-Authored-By:` trailer naming the Claude model doing the work, e.g. `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>` (this overrides the global "no Co-Authored-By" rule for this repo only).
- Keep the model and layout allocation-free on hot paths; the chart must stay responsive on ~1M-node trees (benchmarks in `PLAN.md`: C:\ with 917k nodes scans in ~8.5 s cold, layout + mesh ≈ 1 ms).
- The version comes from `Cargo.toml` via `main::VERSION` and appears in the window title (`Disk Flashlight 0.1.0`), `--version`, the exe version resource and the installer; bump it only in `Cargo.toml`. Match the window by title prefix, not exact title. The release exe is GUI-subsystem, so CLI modes (`--bench`, `--version`, `--help`) call `AttachConsole` to print when run from a console.
- `--mft [PATH]` (`main::elevate_for_mft`) relaunches elevated via `win::relaunch_elevated` with the same path and exits, using the same usefulness rule as the Fast scan button (NTFS volume root, or no path); otherwise, or if UAC is declined, it starts normally. An elevated instance cannot be stopped or captured with PrintWindow from the non-elevated tool shell (UIPI); read its window with `CopyFromScreen` and ask the user to close it.
- Node ids are `u32` indices; `NO_NODE` (`u32::MAX`) is the null parent. Root of a scan is always id 0.
- The chart mesh has no edge feathering; smoothing comes from `NativeOptions::multisampling = 4` (override with env `DF_MSAA`, e.g. `DF_MSAA=1` to compare). The centre disc is drawn as a mesh too (`render::disc_mesh`), because egui circles use a fixed segment count and look polygonal at high zoom.
- Palette and layout tunables are grouped in `render::Palette` and `layout::LayoutParams` rather than scattered as literals.
- GUI checks: capture the app's own window with `PrintWindow` (no focus change). Before any simulated mouse/keyboard input, warn the user so they keep their hands off the machine.
