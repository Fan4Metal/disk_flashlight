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
cargo run --release -- --bench D:\   # scanner + layout benchmark, prints timings and top-level dirs
cargo run --release -- D:\Projects   # open the GUI scanning a path on start
```

Toolchain: stable MSVC Rust plus VS 2022 Build Tools. In the Bash tool, `cargo` is not on PATH; prefix commands with `export PATH="$HOME/.cargo/bin:$PATH"`. A running `disk_flashlight.exe` locks the release binary, so stop it before rebuilding (`Get-Process disk_flashlight | Stop-Process`).

Heredocs containing Rust code break the Bash tool (lifetime apostrophes); write source files with the Write tool or a Python script.

## Architecture

Data flows in one direction: `scan` → `model` → `layout` → `render` → `ui`.

- **`scan/walk.rs`** walks directories with `std::fs::read_dir` under rayon (on Windows `DirEntry::metadata()` is free, taken from `WIN32_FIND_DATAW`). Symlinks and junctions are skipped. It builds a nested `RawDir` tree and reports progress through atomics in `Progress`. `scan/mod.rs` runs it on a thread and hands the result back over a bounded channel; `scan/win.rs` wraps the few Win32 calls (drive list, cluster size, compressed size). Only `Win32_Foundation` and `Win32_Storage_FileSystem` features of `windows-sys` are enabled; `DRIVE_*` constants are hard-coded to avoid another feature.
- **`model.rs`** packs the raw tree into an arena `Vec<Node>` in DFS order. Every node's children are a contiguous range **sorted by logical size descending**, and names live in one string arena. This ordering is what makes "largest first" free downstream; anything that iterates children may rely on it. For the physical metric the order can differ slightly, so layout and the tree re-sort a copy when `Metric::Physical` is active.
- **`layout.rs`** converts a subtree rooted at any node into rings of `Sector { node, a0, a1 }` (angle 0 = top, clockwise). Ring radii follow a geometric series (`ring_shrink`). It stops at the first child whose arc is shorter than `min_arc_px`, so sector count is bounded by what is visible, not by tree size. `Layout::hit_test` finds the ring by radius and binary-searches the angle; `Layout::index` maps node id → sector for external highlighting.
- **`render.rs`** tessellates all sectors into a single `egui::Mesh` (one draw call) and defines the palette (hue by ring depth, brightness alternating by index, lower saturation for files). Helper shapes for hover outlines and guide circles live here too.
- **`ui/chart.rs`** owns the cached `Layout` + `Arc<Mesh>` keyed on (root, metric, centre, radius) and rebuilds only when the key changes. It draws the chart, handles hover (instant `Tooltip::always_open`, not the delayed `on_hover_ui`), wheel zoom and click-to-navigate, returning a `ChartAction` for `App` to apply.
- **`ui/tree.rs`** is a virtualised directory-only tree (`ScrollArea::show_rows`) with its own expanded set; `reveal()` expands ancestors when the root changes. Expand/collapse arrows are painted as triangles because the default fonts lack ▲/▼ glyphs.
- **`ui/toolbar.rs`** implements toolbar and status bar as `impl App` methods. **`app.rs`** is the eframe `App` (note egui 0.36 API: `fn ui(&mut self, ui, frame)` with `egui::Panel::top/left/bottom(...).show(ui, ...)` and `CentralPanel::no_frame()`, not the older `update(ctx)` / `SidePanel` API). Navigation state is `history::History` (root, back, forward), kept UI-free so it is unit-testable.

## Conventions

- Commits in this repository end with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` (this overrides the global "no Co-Authored-By" rule for this repo only).
- Keep the model and layout allocation-free on hot paths; the chart must stay responsive on ~1M-node trees (benchmarks in `PLAN.md`: C:\ with 917k nodes scans in ~8.5 s cold, layout + mesh ≈ 1 ms).
- Node ids are `u32` indices; `NO_NODE` (`u32::MAX`) is the null parent. Root of a scan is always id 0.
- Palette and layout tunables are grouped in `render::Palette` and `layout::LayoutParams` rather than scattered as literals.
- GUI checks: capture the app's own window with `PrintWindow` (no focus change). Before any simulated mouse/keyboard input, warn the user so they keep their hands off the machine.
