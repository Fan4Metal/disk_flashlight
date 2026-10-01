<div align="center">

<img src="images/icon.png" width="112" alt="Disk Flashlight icon">

<h1>Disk Flashlight</h1>

<p><b>A disk space analyzer for Windows with a sunburst chart, in the spirit of OverDisk</b></p>

[![Release](https://img.shields.io/github/v/release/Fan4Metal/disk_flashlight?label=release)](https://github.com/Fan4Metal/disk_flashlight/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/Fan4Metal/disk_flashlight/total)](https://github.com/Fan4Metal/disk_flashlight/releases)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-0078D6)](#download)
[![Rust](https://img.shields.io/badge/Rust-2024%20edition-B7410E?logo=rust)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/license-MIT-green)](LICENSE)

**English** | [Русский](README.ru.md)

[Download](#download) · [Features](#features) · [Usage](#usage) · [Keyboard and mouse](#keyboard-and-mouse) · [Command line](#command-line) · [Building](#building)

</div>

---

Disk Flashlight shows the scanned directory as a sunburst chart: the current directory sits in the centre, each ring outward contains the children of the ring before it, and within every ring the entries are ordered by size, largest first, clockwise from twelve o'clock.

![Disk Flashlight showing drive C: as a sunburst chart, with the directory tree on the left](images/screenshot.png)

The project is written in Rust and uses [egui](https://github.com/emilk/egui) with the wgpu backend.

## Highlights

- **Fast scanning.** With administrator rights, NTFS volumes are read straight from the Master File Table: the `C:` drive with roughly 920 000 entries takes under one second.
- **Responsive chart.** The chart is a single cached GPU mesh, so it stays smooth on trees of a million entries and at 200x zoom.
- **Search, file types and the largest files** next to the chart, all linked to it: matches and types are coloured on the chart, and every list leads to the item's folder.
- **Cleanup in place.** Items are moved to the Recycle Bin from the chart or any list, and the view is updated without a rescan.
- **CSV export** of the tree, the lists and the search results, from the window or from the command line.
- **English and Russian** interface, light and dark themes, a portable mode, and no network access beyond the scanned network paths and the optional update check.

## Download

Ready-made builds are published on the [Releases](https://github.com/Fan4Metal/disk_flashlight/releases/latest) page:

| File | Description |
|---|---|
| `Disk_Flashlight_<version>_Setup.exe` | Per-user installer. Administrator rights are not required: the program is placed in `%LOCALAPPDATA%\Programs\Disk Flashlight`. An optional task adds **Analyze with Disk Flashlight** to the Explorer context menu of folders and drives; the entry is removed on uninstallation. |
| `Disk_Flashlight_<version>_portable.zip` | The program and an empty `disk_flashlight.ron` in a `Disk Flashlight` folder. It runs without installation and keeps its settings in that file. |

The executables are not code-signed, so Windows SmartScreen may warn about an unknown publisher on first launch.

## Features

### Scanning

- **MFT scan.** When the application runs with administrator rights, NTFS drives and folders are scanned by reading the Master File Table directly (the `C:` drive with roughly 920 000 entries is scanned in under one second). A folder is taken out of the table of the whole volume, so even a small folder takes about as long as its drive. The MFT scan counts hard links once, under the folder of the file's first name, and includes system areas such as `System Volume Information` and the NTFS metafiles.
- **Directory walk.** Non-NTFS volumes, network paths and non-elevated runs are scanned by a parallel directory walk (the same `C:` drive takes about 8 seconds on a cold cache and about 1.3 seconds on a warm one). Network drives and volumes with other file systems do not support the MFT and are always scanned this way.
- **Fast scan.** Without administrator rights, the toolbar always shows the **Fast scan** button (marked with the UAC shield): it restarts the application elevated, after which local NTFS drives are scanned through the MFT. A drive or folder on such a volume that is already chosen is rescanned at once.
- **Access errors.** Folders that the directory walk cannot read (usually for lack of rights) are counted in the status bar; the count is a link to the **Access errors** window, which lists their paths with the reason, can copy the list or save it as CSV (**Export…**, columns `path` and `error`), and opens a folder's location in Explorer from its context menu.
- **Size metric.** The chart measures either the physical size (cluster-rounded, compressed and sparse files taken into account; small files that NTFS keeps inside their MFT record take no space) or the logical size.

### Chart

- **Rings.** Seven rings by default, rendered as a single cached GPU mesh; sectors thinner than one pixel are dropped, so the chart stays responsive regardless of tree size. The number of rings, from 3 to 12, is chosen in the **Rings** list on the toolbar or with the `+` and `-` keys and is kept between runs; the rings thin out towards the rim, so the outermost one is about a quarter as thick as the first whatever their number.
- **Files and folders.** As in OverDisk, files are drawn with a gap to their folder, so they are told apart from directories at a glance. Edges are smoothed with 4x multisampling.
- **Small items.** Items too small to be told apart (under about four points of arc) are merged into one neutral **N smaller items** sector per directory; zooming in splits the group as items become wide enough, and a click on the group zooms in.
- **Free space.** When a whole drive (or network share) is scanned and its root is in the centre, the free space is shown as a pale grey-blue sector after the contents in the first ring, with its size and share of the capacity in the tooltip.
- **Tooltip.** The sector under the cursor shows its name, the logical and allocated size, the share of the parent folder and of the centre of the chart (`12% of Users, 3.4% of C:`), the directory and file counts, and the date of the last change, both as a date and as an age (`2024-03-15 (6 months ago)`). The date of a file is its last write time; the date of a folder is the newest last write time of anything inside it, so that a folder of old files shows as old even when an entry was added to it recently.

#### Colour schemes

The scheme is selected in the toolbar.

| Scheme | Colour |
|---|---|
| **By size** | The largest item among its siblings is red and smaller ones shift towards yellow, with colours becoming paler towards the rim (as in OverDisk). |
| **By level** | The colour depends on the ring. |
| **By age** | The colour depends on the date of the last change, from red for items changed just before the scan through yellow and green to blue for items ten years old or older, on a logarithmic scale explained by a legend in the corner of the chart. |
| **By type** | Each of the twelve largest file types of the scan has a colour of its own, all other files one neutral colour, and folders are grey; the colours do not change while moving through the folders, and a legend in the corner lists them. |

### Navigation and actions

- **Navigation.** A click on a directory sector makes it the centre, and on a file moves to its folder; a click on the centre goes up. Back, forward and up navigation keeps a history.
- **Zoom and pan.** The mouse wheel zooms the chart towards the cursor, up to 200x; dragging with the middle mouse button pans it (see [Keyboard and mouse](#keyboard-and-mouse)).
- **Context menu.** A right-click offers **Open in Explorer**, **Properties**, **Copy path** and **Delete…** for the sector under the cursor, or for the current directory when the centre is right-clicked; the same menu is available in the directory tree and in the file lists. `Ctrl+C` copies the full path of the item under the mouse in the chart or in a list, or of the centre of the chart when the mouse is elsewhere.
- **Deletion.** **Delete…** asks for confirmation and moves the item to the Recycle Bin (Windows warns before deleting permanently an item that cannot be recycled); the chart, the tree and the lists are then updated without a rescan. The scanned folder itself cannot be deleted.
- **Rescanning.** A rescan keeps the current folder and the navigation history, and the previous results stay visible until it finishes; if the folder no longer exists, the view moves to its closest remaining parent. When another drive or folder is chosen, the previous results are cleared at once and a spinner with the scan progress is shown instead.

### Side panel

The panel on the left has four tabs: **Folders**, **Largest files**, **Search** and **Types**.

- **Folders.** The directory tree is synchronised with the chart in both directions. When the centre of the chart changes, only the folders on the path to it stay expanded. While the mouse moves over the chart, the tree is temporarily expanded to the hovered item and folds back when the mouse rests on the chart outside the sectors (moving the mouse from the chart into the tree keeps it expanded); the **Follow in tree** toolbar option (enabled by default) turns this off. **Collapse all** above the tree folds every folder but the scanned one; a click on a file of the current centre expands the tree to it again.
- **Largest files.** The 100 largest files under the centre of the chart, with their folders. Hovering a row highlights the file on the chart, and a click moves the chart into the file's folder, where the file stays selected in the list. The list can be limited to files not modified for more than one, two, five or ten years before the scan (**Older than**), which finds large files that have not been used for a long time.
- **Search** (`Ctrl+F`). Files and folders of the whole scan whose names contain the typed text, ignoring case; the 200 largest matches are listed with the total count and the total size (files inside a matching folder are counted once), and the matching parts of each name are highlighted. While the tab is open, the chart keeps the colour of matches and fades everything else; a folder holding some matches is coloured by the share of its size they take, and its tooltip shows that share. A click on a folder makes it the centre of the chart, a click on a file moves into its folder. The query syntax is described [below](#search-syntax).
- **Types.** The file types (extensions, ignoring case) under the centre of the chart with the description Windows gives them, their share of the centre and their total size, largest first; the tooltip of a row adds the number of files and both sizes. A click on a type colours its files on the chart and fades everything else, a second click clears it; a double click lists the files of the type in the **Search** tab (`*.mp4`). The arrow at the left of a type unfolds its 20 largest files under the centre, with their folders; as in the other lists, hovering one outlines it on the chart, a click moves the chart into its folder, and the context menu applies to it. When a type has more files, a link after them opens all of them in the **Search** tab.

The **Largest files** and **Search** lists can be ordered by size (**Largest first**), by the date of the last change with the oldest first (**Oldest first**) or with the newest first (**Newest first**); when they are ordered by date or filtered by age, a column with the date is shown. The tooltip of a row gives the full path and the date.

#### Search syntax

| Query | Finds |
|---|---|
| `report 2024` | Names containing every word separated by spaces, in any order. |
| `mp4\|mkv`, `*.mp4;*.mkv` | Alternatives: words joined by `;` or `\|` bind tighter than a space. |
| `"big fish"` | A phrase kept together by quotes. |
| `*.py` | A mask for the whole name, as in Explorer: `*` is any run of characters, `?` is one character. |
| `*.mp4;*.mkv 2010` | MP4 and MKV files with 2010 in the name. |

The button with a page and a folder cycles between files and folders, files only and folders only. The **ab** button in the field restricts plain-text matches to whole words (so `.py` finds `script.py` but not `module.pyd`); both buttons are remembered between runs. The **×** button or `Esc` clears the field.

### Export to CSV

**Export…** in the **Folders**, **Largest files** and **Search** tabs saves the list as a CSV file:

| Tab | Contents | Columns |
|---|---|---|
| **Folders** | The folders of the tree as far as they are expanded, so a report of the wanted depth is made by expanding the tree first. | `path`, `level` (the depth below the scanned folder), `size`, `allocated`, `files` and `folders` (the numbers of files and folders inside), `modified` |
| **Largest files** | The listed files, in the order chosen. | `path`, `type` (`file` or `folder`), `size` and `allocated` (in bytes), `modified` (local time, `2024-03-15 14:02:11`) |
| **Search** | All matches, not only the 200 listed, in the order chosen. | the same as **Largest files** |

The file is UTF-8 with a byte order mark, so that Excel reads non-ASCII names. Fields are separated by the list separator of the Windows region settings, which Excel expects when a CSV file is opened (`;` where the decimal separator is a comma). The same lists are available without the window through [`--export`](#command-line).

### Interface

- **Languages.** The interface is available in English and Russian. The language follows that of Windows (Russian when Windows is shown in Russian, English otherwise) and can be chosen in the **About** window; sizes, dates and numbers follow the language (`4.6 GB` and `2024-03-15` in English, `4,6 ГБ` and `15.03.2024` in Russian). The command-line modes print in English.
- **Themes.** Light and dark themes are switched by the button with a sun, a moon or a half-filled circle next to **About**: a click moves from following the Windows setting (the default) to the light theme, then to the dark one and back. The chart has a palette of its own for each theme; on the dark one the colours are deeper and fade towards the background further out.
- **Status bar.** The status bar shows the directory and file counts, the logical size, the allocated size and the slack for the current root.

### Settings and portable mode

The following settings are kept between runs: the language, the theme, the update check option, the chart metric, the colour scheme, the number of rings, the **Follow in tree** option, the tab shown in the left panel, the order of both lists and the age filter of the **Largest files** tab, the window size and position, the tree width, and the last scanned path, which is offered in the path field without being scanned. Nothing is selected in the drive list at start-up, so a slow or network drive is not scanned unasked.

Settings are stored in `%APPDATA%\Disk Flashlight\data\app.ron`. A copy becomes portable when a `disk_flashlight.ron` file (an empty one is enough) lies next to `disk_flashlight.exe`: the settings are then kept in that file, and nothing is written to the user profile. If the file cannot be written, for example on a read-only medium, the profile is used instead. The **About** window shows where the settings are kept. If the program stops because of an internal error, it says so in a message box and appends the details to `crash.log` in the same folder.

### Update check and privacy

The update check is optional and off by default. With **Check for updates at start-up (once a day)** enabled in the **About** window, the application asks the GitHub Releases API (`api.github.com`) for the latest release at most once a day; **Check now** in the same window asks at once. The request carries nothing but the program version in its `User-Agent` header, and nothing is downloaded or installed: when a newer release exists, a button with its version appears to the right of **About** and opens the release page in the browser. Otherwise the application accesses the network only for the network drives and paths it scans.

## Usage

### Choosing what to scan

- **Drive list.** Besides the drives, the drive list offers the eight folders scanned most recently (drive roots are not repeated there; **Clear recent** empties the list, which is kept between runs) and **Choose folder…**, which opens the standard Windows folder dialog and scans the chosen folder.
- **Drag and drop.** A folder or a drive can be dragged from Explorer onto the window; a dropped file scans the folder it is in. While the application runs as administrator (after **Fast scan**), Windows does not pass drops from the non-elevated Explorer to it.
- **Path field.** A path can be typed into the field at the right of the toolbar, which shows the current path and becomes editable on a click, as the address bar of Explorer: while typing, the matching folders (or drives) are offered below it, the arrow keys pick one, `Tab` or `Enter` takes it, and `Enter` scans the typed path; `Esc` cancels. A path that does not exist or is not a folder is reported in the status bar, and the results on screen stay.
- **Explorer.** With the installer's optional task, **Analyze with Disk Flashlight** in the context menu of a folder or a drive opens it in the program.

The version is shown in the **About** window.

### Keyboard and mouse

| Keys | Action |
|---|---|
| `Backspace` | Up one level (with a selected sector, the folder left stays selected) |
| `Alt+Left`, `Alt+Right` | Back and forward through history |
| `F5` | Rescan |
| `Ctrl+F` | Open the search |
| `Ctrl+C` | Copy the path of the item under the mouse (or of the centre) |
| `+`, `-` | Add or remove a ring |
| `F1` | **About** window (also available from the toolbar) |
| `Right`, `Left` | Next or previous sector of the same ring (clockwise and back) |
| `Down` | The largest item inside the selected folder, one ring further out |
| `Up` | The parent, one ring further in |
| `Enter` | Acts as a click: a folder becomes the centre of the chart, with its largest item selected, and a file takes the chart to its folder |
| `Delete` | Move the selected item (or the one under the mouse, in the chart or a list) to the Recycle Bin after a confirmation |
| `Ctrl+E` | Open the selected item in Explorer (the centre when nothing is selected) |
| `Esc` | Clear the keyboard selection |

The first arrow starts from the item under the mouse, or from the largest item. The selected sector is outlined and shows its tooltip, as under the mouse, until the mouse moves or `Esc` is pressed.

| Mouse | Action |
|---|---|
| Left click on a sector | A folder becomes the centre; a file moves the chart to its folder; a group zooms in |
| Left click on the centre | Up one level |
| Right click | Context menu |
| Wheel | Zoom towards the cursor |
| Middle-button drag | Pan |
| Middle-button double click | Restore the initial view, as does the small button with the zoom factor in the top right corner of the chart (shown while zoomed or panned) |

### Command line

```
disk_flashlight.exe              # opens the window; a drive is picked from the toolbar
disk_flashlight.exe D:\Projects  # scans the given path on start-up
disk_flashlight.exe --bench C:\  # command-line benchmark of the scanner and layout
disk_flashlight.exe --bench C:\ --walk  # the same benchmark with the MFT scanner disabled
disk_flashlight.exe --version    # prints the version
disk_flashlight.exe --mft C:\    # restarts as administrator (UAC prompt) and scans C: through the MFT
disk_flashlight.exe --export D:\ --out largest.csv                  # the 100 largest files as CSV
disk_flashlight.exe --export D:\ --largest 500 --older-than 2 --out old.csv
disk_flashlight.exe --export C:\ --search "*.iso;*.vhdx" --sep ";" --out images.csv
```

The `--mft` option requests administrator rights only where they speed up the scan: for a drive or folder on a local NTFS volume, or when no path is given. For another file system or a network path, and when the UAC prompt is declined, the program starts normally and walks the directories.

The `--export` option scans the path as `--bench` does and writes a list in the format of **Export…**, without opening the window. Without `--out` the CSV goes to the standard output; a one-line summary goes to the standard error. `--help` lists the options.

| Option | Meaning |
|---|---|
| `--largest N` | The N largest files (100 by default) |
| `--older-than Y` | Only files not changed for Y years (with `--largest`) |
| `--search QUERY` | Every match of a query in the syntax of the **Search** tab |
| `--kind files`, `--kind folders` | Limits the search to files or to folders |
| `--whole-word` | Matches whole words |
| `--sort size\|oldest\|newest` | Orders the rows |
| `--logical` | Measures sizes by length rather than by space on disk |
| `--sep SEP` | Field separator (a comma by default, `tab` for a tab) |
| `--out FILE` | Output file |
| `--walk` | Disables the MFT scanner |

Setting the environment variable `RUST_LOG=disk_flashlight=debug` prints per-phase timings of the MFT scanner.

## Building

Requirements: a stable Rust toolchain (MSVC target) and the Visual Studio Build Tools with the C++ workload.

```
cargo build --release
```

The binary is produced at `target\release\disk_flashlight.exe`.

<details>
<summary><b>Setting up the environment</b></summary>

The project builds on Windows 10 or 11 (x64). The following was used; the commands given install everything through `winget`:

- **Visual Studio Build Tools 2022** with the "Desktop development with C++" workload (`Microsoft.VisualStudio.Workload.VCTools`): the MSVC v143 compiler and linker and the Windows 11 SDK (10.0.26100). Rust needs the linker for the MSVC target, and the build script, which embeds the icon and the version information into the executable (`winresource`), needs `rc.exe` from the SDK. The Build Tools are installed without Visual Studio itself:

  ```
  winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
  ```

  When the Build Tools are missing, `rustup-init.exe` also offers to install them.

- **Rust** through `rustup`, with the `stable-x86_64-pc-windows-msvc` toolchain (tested with 1.98). The project uses the 2024 edition and `if let` chains, so Rust 1.88 or later is required.

  ```
  winget install Rustlang.Rustup
  rustup default stable-x86_64-pc-windows-msvc
  ```

- For the installer and releases only: **Python 3** (tested with 3.14; the script needs nothing beyond the standard library) and **Inno Setup 6**, whose `ISCC.exe` is looked for in the standard installation folders and on `PATH`.

  ```
  winget install Python.Python.3.14
  winget install JRSoftware.InnoSetup
  ```

The other dependencies (egui, wgpu, windows-sys and the rest) are downloaded by `cargo` on the first build.

</details>

### Installer and release

The release script builds the executable and a per-user installer (Inno Setup 6 is required):

```
python tools/make_release.py              # tests, release build, installer
python tools/make_release.py --no-tests   # the same without cargo test
```

The installer is written to `dist\Disk_Flashlight_<version>_Setup.exe`, together with the portable archive `dist\Disk_Flashlight_<version>_portable.zip` (see [Download](#download)); the version is taken from `Cargo.toml`.

Releases on GitHub are built by the **Release** workflow (`.github/workflows/release.yml`). After the version in `Cargo.toml` is updated and committed, pushing a matching tag publishes a release with the installer and the portable archive:

```
git tag v0.1.0
git push origin v0.1.0
```

The workflow stops if the tag does not match the version in `Cargo.toml`. A manual run from the Actions tab builds the same files as a workflow artifact without publishing a release.

## Project layout

| Path | Purpose |
|---|---|
| `src/model.rs` | Arena tree packed in depth-first order with children sorted by size; largest files and name search |
| `src/types.rs` | File types (extensions): totals under a folder and colour slots |
| `src/scan/mft.rs` | NTFS Master File Table reader and parser |
| `src/scan/walk.rs` | Parallel directory walk (rayon over directory listings read in 64 KiB batches) |
| `src/scan/win.rs` | Win32 helpers: drives, directory listing, cluster size, compressed sizes, Recycle Bin, Explorer |
| `src/layout.rs` | Sunburst layout and hit testing |
| `src/render.rs` | Tessellation of sectors into an `egui::Mesh`, palettes for both themes |
| `src/ui/` | Chart widget, directory tree, Largest files, Search and Types tabs, path field, toolbar and status bar, About and Access errors windows |
| `src/history.rs` | Back/forward navigation history |
| `src/settings.rs` | Settings kept between runs, portable mode |
| `src/i18n.rs`, `src/format.rs` | English and Russian interface; sizes, numbers and dates in the interface language |
| `tools/` | Installer script (`setup.iss`) and release script (`make_release.py`) |

## License

MIT, see [LICENSE](LICENSE).
