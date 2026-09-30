#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
mod i18n;
mod app;
mod export;
mod format;
mod history;
mod icon;
mod layout;
mod model;
mod render;
mod scan;
mod settings;
mod types;
mod ui;
mod update;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::Context;
use format::{human_size, thousands};

/// Version from Cargo.toml, shared by the About window, `--version`, the
/// installer and the GitHub release tag.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// eframe app id; also names the settings folder in `%APPDATA%` (unless the
/// copy is portable, see `settings::location`).
const APP_ID: &str = "Disk Flashlight";

/// The saved window was maximized: it is created normal and maximized once
/// shown (see `main`), and `App` reads this to do so.
pub static MAXIMIZE_WHEN_SHOWN: AtomicBool = AtomicBool::new(false);

/// The release build is a GUI-subsystem program with no console of its own,
/// so text printed by the command-line modes would be lost when started from
/// cmd or PowerShell. Attach to the parent's console in that case; output
/// that is already redirected (a pipe or a file) is left untouched.
#[cfg(windows)]
fn attach_parent_console() {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    if std::io::stdout().as_raw_handle().is_null() {
        unsafe {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let cli_mode = std::env::args()
        .skip(1)
        .any(|a| matches!(a.as_str(), "--bench" | "--export" | "--version" | "-V" | "-h" | "--help"));
    #[cfg(windows)]
    if cli_mode {
        attach_parent_console();
    }
    let mut args = std::env::args().skip(1);
    let mut initial: Option<PathBuf> = None;
    let mut bench_path: Option<PathBuf> = None;
    let mut allow_mft = true;
    let mut want_mft = false;
    let mut export_path: Option<PathBuf> = None;
    let mut ex = ExportArgs::default();
    while let Some(a) = args.next() {
        let mut value = |what: &str| args.next().ok_or_else(|| anyhow::anyhow!("{what} needs a value"));
        match a.as_str() {
            "--bench" => {
                bench_path = Some(normalize(&value("--bench").unwrap_or_else(|_| "C:".into())));
            }
            "--export" => export_path = Some(normalize(&value("--export")?)),
            "--out" => ex.out = Some(value("--out")?),
            "--largest" => ex.largest = Some(value("--largest")?.parse().context("--largest takes a number")?),
            "--search" => ex.search = Some(value("--search")?),
            "--sep" => {
                let s = value("--sep")?;
                let mut chars = s.chars();
                ex.sep = match (s.as_str(), chars.next(), chars.next()) {
                    ("tab", _, _) => '\t',
                    (_, Some(c), None) if c != '"' => c,
                    _ => anyhow::bail!("--sep takes one character (or tab)"),
                };
            }
            "--kind" => {
                ex.kind = match value("--kind")?.as_str() {
                    "all" => model::ItemKind::All,
                    "files" => model::ItemKind::Files,
                    "folders" => model::ItemKind::Folders,
                    k => anyhow::bail!("--kind takes all, files or folders, not {k}"),
                };
            }
            "--whole-word" => ex.whole_word = true,
            "--older-than" => {
                ex.older_than = value("--older-than")?.parse().context("--older-than takes a number of years")?;
            }
            "--sort" => {
                ex.sort = match value("--sort")?.as_str() {
                    "size" => ui::files::ListSort::Size,
                    "oldest" => ui::files::ListSort::Oldest,
                    "newest" => ui::files::ListSort::Newest,
                    s => anyhow::bail!("--sort takes size, oldest or newest, not {s}"),
                };
            }
            "--logical" => ex.metric = model::Metric::Logical,
            "--walk" => allow_mft = false,
            "--mft" => want_mft = true,
            "--export-icon" => {
                // Used by tools/make_release.py for the installer's icon.
                let out = args.next().unwrap_or_else(|| "app.ico".into());
                std::fs::write(&out, icon::ico(&[16, 20, 24, 32, 40, 48, 64, 256]))?;
                return Ok(());
            }
            "-V" | "--version" => {
                println!("Disk Flashlight {VERSION}");
                return Ok(());
            }
            "-h" | "--help" => {
                println!("Disk Flashlight {VERSION}");
                println!("disk_flashlight [--mft] [PATH] | --bench PATH [--walk] | --export PATH [options]");
                println!("                | --export-icon FILE | --version");
                println!("  --mft   restart as administrator (UAC prompt) so that a whole NTFS drive");
                println!("          is scanned through the MFT; ignored where it would not help");
                println!("  --export PATH   scan PATH and write a list as CSV (UTF-8 with BOM):");
                println!("    --largest N        the N largest files (the default, 100)");
                println!("    --older-than Y     with --largest: only files not changed for Y years");
                println!("    --search QUERY     all matches of QUERY, as in the Search tab");
                println!("    --kind K           with --search: all, files or folders");
                println!("    --whole-word       with --search: match whole words only");
                println!("    --sort S           size (default), oldest or newest");
                println!("    --logical          sizes by length, not by space on disk");
                println!("    --sep C            field separator: one character or tab (default ,)");
                println!("    --out FILE         write to FILE instead of the console (- for stdout)");
                println!("    --walk             do not use the MFT scanner");
                return Ok(());
            }
            other => initial = Some(normalize(other)),
        }
    }
    if let Some(p) = bench_path {
        return bench(p, allow_mft);
    }
    if let Some(p) = export_path {
        return export(p, ex, allow_mft);
    }
    if want_mft && elevate_for_mft(initial.as_deref()) {
        return Ok(()); // the elevated copy takes over
    }

    let stored = settings::location();
    install_panic_hook(stored.file.as_ref().and_then(|f| f.parent()).map(|d| d.join(CRASH_LOG)));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Disk Flashlight")
            .with_inner_size([1200.0, 800.0])
            // Widened to what the toolbar needs by `App::fit_min_width`.
            .with_min_inner_size(app::MIN_WINDOW)
            .with_icon(egui::IconData {
                rgba: icon::rgba(64),
                width: 64,
                height: 64,
            }),
        // Centre only on the first run; later runs restore the saved window.
        centered: !stored.has_saved(),
        // A portable copy keeps its settings next to the exe; otherwise
        // eframe's own place in %APPDATA%.
        persistence_path: if stored.portable { stored.file.clone() } else { None },
        // 4x MSAA: the chart is one big triangle mesh without egui's edge
        // feathering, so thin sectors and arcs would otherwise be jagged.
        // DF_MSAA=1 turns it off (for comparison or a GPU without MSAA).
        multisampling: std::env::var("DF_MSAA")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4),
        // eframe creates the window hidden and shows it after the first
        // frame, but winit shows a window created maximized at once
        // (ShowWindow(SW_MAXIMIZE)), which flashes an empty white window.
        // So a restored maximized window is created normal, at the same
        // size, and maximized by `App` once it is visible.
        window_builder: Some(Box::new(|mut builder| {
            if builder.maximized == Some(true) {
                builder.maximized = Some(false);
                MAXIMIZE_WHEN_SHOWN.store(true, Ordering::Relaxed);
            }
            builder
        })),
        ..Default::default()
    };
    eframe::run_native(
        APP_ID,
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, initial)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Written next to the settings file when the app fails.
const CRASH_LOG: &str = "crash.log";

/// The release build aborts on a panic and has no console, so the window
/// would vanish without a word: say what happened in a message box and
/// append it to `log`, so that it can be reported. The default hook still
/// prints it (seen in a debug build or with a console).
fn install_panic_hook(log: Option<PathBuf>) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        let thread = std::thread::current();
        let when = model::unix_now();
        let report = format!(
            "Disk Flashlight {VERSION}, Unix time {when}, thread {}\n{info}\n\n",
            thread.name().unwrap_or("unnamed")
        );
        let saved = log.as_ref().filter(|path| {
            use std::io::Write;
            let _ = path.parent().map(std::fs::create_dir_all);
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut f| f.write_all(report.as_bytes()))
                .is_ok()
        });
        let text = match saved {
            Some(path) => tr!(
                format!(
                    "Disk Flashlight stopped because of an internal error.\n\n{info}\n\n\
                     The details were saved to {}.",
                    path.display()
                ),
                format!(
                    "Disk Flashlight остановлена из-за внутренней ошибки.\n\n{info}\n\n\
                     Подробности сохранены в {}.",
                    path.display()
                )
            ),
            None => tr!(
                format!("Disk Flashlight stopped because of an internal error.\n\n{info}"),
                format!("Disk Flashlight остановлена из-за внутренней ошибки.\n\n{info}")
            ),
        };
        scan::win::error_box("Disk Flashlight", &text);
    }));
}

/// `--mft`: restart elevated when that enables the MFT scanner, i.e. when
/// not elevated yet and the target is on a local NTFS drive (or no target
/// was given, so the drive is picked later). Returns `true` if the
/// elevated copy was started; a declined UAC prompt continues without it.
fn elevate_for_mft(target: Option<&std::path::Path>) -> bool {
    if scan::win::is_elevated() {
        return false;
    }
    let useful = match target {
        None => true,
        Some(p) => scan::mft::drive_letter(p).is_some_and(|letter| {
            scan::win::drive_info(&format!("{letter}:\\"))
                .is_some_and(|d| d.fs.eq_ignore_ascii_case("NTFS"))
        }),
    };
    if !useful {
        log::info!("--mft ignored: target is not on a local NTFS drive");
        return false;
    }
    let mut args = String::from("--mft");
    if let Some(p) = target {
        args.push(' ');
        args.push_str(&app::quote_arg(&p.to_string_lossy()));
    }
    scan::win::relaunch_elevated(&args)
}

/// `C:` means "current directory on C" to Windows; a bare drive letter given
/// by the user always means the volume root.
///
/// Explorer's context menu passes a drive as `"C:\"`; Windows argument
/// parsing reads `\"` as an escaped quote, so the program receives `C:"`.
/// A trailing quote is therefore turned back into a backslash.
pub fn normalize(arg: &str) -> PathBuf {
    let arg = match arg.strip_suffix('"') {
        Some(stripped) => format!("{}\\", stripped.trim_end_matches('\\')),
        None => arg.to_string(),
    };
    let b = arg.as_bytes();
    if b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        PathBuf::from(format!("{arg}\\"))
    } else {
        PathBuf::from(arg)
    }
}

/// Options of `--export`.
struct ExportArgs {
    out: Option<String>,
    largest: Option<usize>,
    search: Option<String>,
    sep: char,
    kind: model::ItemKind,
    whole_word: bool,
    older_than: u32,
    sort: ui::files::ListSort,
    metric: model::Metric,
}

impl Default for ExportArgs {
    fn default() -> Self {
        Self {
            out: None,
            largest: None,
            search: None,
            sep: ',',
            kind: model::ItemKind::All,
            whole_word: false,
            older_than: 0,
            sort: ui::files::ListSort::Size,
            metric: model::Metric::Physical,
        }
    }
}

/// `--export PATH`: scan, pick the largest files or the matches of a
/// search as the tabs do, and write them as CSV; a summary goes to stderr.
fn export(path: PathBuf, ex: ExportArgs, allow_mft: bool) -> anyhow::Result<()> {
    if ex.largest.is_some() && ex.search.is_some() {
        anyhow::bail!("--largest and --search cannot be combined");
    }
    if ex.search.is_some() && ex.older_than > 0 {
        anyhow::bail!("--older-than goes with --largest, not --search");
    }
    let progress = scan::Progress::default();
    let t = Instant::now();
    let (model, info) = scan::scan(&path, &progress, allow_mft)?;
    let scan_time = t.elapsed();
    let metric = ex.metric;
    let mut ids = match ex.search.as_deref().map(str::trim) {
        Some(query) => {
            // Every match: the first pass counts them.
            let count = model.search(0, query, metric, 1, ex.whole_word, ex.kind).count;
            model.search(0, query, metric, count.max(1), ex.whole_word, ex.kind).ids
        }
        None => {
            let before = (ex.older_than > 0)
                .then(|| model.scanned_at.saturating_sub(ex.older_than.saturating_mul(ui::files::YEAR_SECS)));
            model.largest_files(0, metric, ex.largest.unwrap_or(100), before)
        }
    };
    ex.sort.apply(&model, metric, &mut ids, |&i| i);
    match ex.out.as_deref() {
        Some(file) if file != "-" => {
            export::save(std::path::Path::new(file), |w, offset| {
                export::write_items(w, &model, &ids, ex.sep, offset)
            })
            .with_context(|| format!("writing {file}"))?
        }
        _ => {
            use std::io::Write;
            let mut w = std::io::BufWriter::new(std::io::stdout().lock());
            export::write_items(&mut w, &model, &ids, ex.sep, format::local_offset())?;
            w.flush()?;
        }
    }
    eprintln!(
        "{} rows from {} ({:?}, scanned in {:.1}s)",
        thousands(ids.len() as u64),
        model.root_path,
        info.method,
        scan_time.as_secs_f64()
    );
    Ok(())
}

/// `--bench PATH`: scan, pack, lay out and tessellate, printing timings.
fn bench(path: PathBuf, allow_mft: bool) -> anyhow::Result<()> {
    let progress = scan::Progress::default();
    // Start the rayon pool first so its start-up is not billed to the scanner.
    rayon::broadcast(|_| ());
    let t = Instant::now();
    let (model, info) = scan::scan(&path, &progress, allow_mft)?;
    let scan_time = t.elapsed();
    println!(
        "method:    {:?}{}",
        info.method,
        info.fallback_reason
            .map(|r| format!("  (MFT unavailable: {r})"))
            .unwrap_or_default()
    );
    let (_, _, _, errors) = progress.snapshot();
    let root = model.node(0);
    println!("path:      {}", model.root_path);
    println!("cluster:   {} B", model.cluster_size);
    println!(
        "scan:      {:.2}s  ({} nodes, {} files, {} dirs, {} errors)",
        scan_time.as_secs_f64(),
        thousands(model.len() as u64),
        thousands(root.files as u64),
        thousands(root.dirs as u64),
        errors
    );
    println!(
        "size:      {} logical, {} allocated",
        human_size(root.size),
        human_size(root.alloc)
    );

    println!("top-level (largest first, with the last change):");
    for c in model.children(0).take(12) {
        let n = model.node(c);
        println!(
            "  {:>10}  {:<10}  {}{}",
            human_size(n.size),
            format::date(n.modified),
            model.name(c),
            if n.is_dir { "\\" } else { "" }
        );
    }

    let params = layout::LayoutParams::default();
    let center = egui::Pos2::new(600.0, 400.0);
    let t = Instant::now();
    let view = egui::Rect::from_center_size(center, egui::vec2(1200.0, 800.0));
    let l = layout::build(&model, 0, model::Metric::Physical, center, 380.0, view, &params);
    let layout_time = t.elapsed();
    let t = Instant::now();
    let mesh = render::build_mesh(&model, &l, &render::Palette::default(), None);
    let mesh_time = t.elapsed();
    println!(
        "layout:    {:.2}ms  ({} sectors)",
        layout_time.as_secs_f64() * 1e3,
        thousands(l.sector_count() as u64)
    );
    println!(
        "mesh:      {:.2}ms  ({} vertices, {} triangles)",
        mesh_time.as_secs_f64() * 1e3,
        thousands(mesh.vertices.len() as u64),
        thousands((mesh.indices.len() / 3) as u64)
    );

    let deep = layout::LayoutParams { max_depth: layout::MAX_RINGS, ..params };
    let t = Instant::now();
    let l = layout::build(&model, 0, model::Metric::Physical, center, 380.0, view, &deep);
    let mesh = render::build_mesh(&model, &l, &render::Palette::default(), None);
    println!(
        "{} rings: {:.2}ms  ({} sectors, {} vertices)",
        layout::MAX_RINGS,
        t.elapsed().as_secs_f64() * 1e3,
        thousands(l.sector_count() as u64),
        thousands(mesh.vertices.len() as u64)
    );

    // Zoomed in: the 1200x800 view looks at the rings above the centre, so
    // most of the chart is culled. Work must stay bounded by the view.
    for zoom in [10.0f32, 50.0, 200.0] {
        let radius = 380.0 * zoom;
        let c = egui::Pos2::new(600.0, 400.0 + radius * 0.45);
        let t = Instant::now();
        let l = layout::build(&model, 0, model::Metric::Physical, c, radius, view, &params);
        let mesh = render::build_mesh(&model, &l, &render::Palette::default(), None);
        println!(
            "zoom {zoom:>3}: {:.2}ms  ({} sectors, {} vertices)",
            t.elapsed().as_secs_f64() * 1e3,
            thousands(l.sector_count() as u64),
            thousands(mesh.vertices.len() as u64)
        );
    }

    // "Largest files" tab for the whole scan, the worst case.
    let t = Instant::now();
    let top = model.largest_files(0, model::Metric::Physical, 100, None);
    println!(
        "top files: {:.2}ms  ({} listed)",
        t.elapsed().as_secs_f64() * 1e3,
        top.len()
    );

    // File types: built with the model; the totals for the whole scan and
    // the colouring of one type are what the Types tab does.
    let t = Instant::now();
    let rebuilt = types::FileTypes::build(&model);
    let t_build = t.elapsed();
    let t = Instant::now();
    let stats = model.types.stats(&model, 0, model::Metric::Physical);
    let t_stats = t.elapsed();
    let t = Instant::now();
    let hits = stats.first().map(|s| model.types.hits(&model, s.ty, model::Metric::Physical));
    println!(
        "types:     build {:.2}ms, totals {:.2}ms, colour one {:.2}ms  ({} types; {})",
        t_build.as_secs_f64() * 1e3,
        t_stats.as_secs_f64() * 1e3,
        t.elapsed().as_secs_f64() * 1e3,
        stats.len(),
        rebuilt
            .coloured()
            .iter()
            .map(|&ty| format!(".{}", rebuilt.name(ty)))
            .collect::<Vec<_>>()
            .join(" ")
    );
    drop(hits);

    // The model rebuilt without a deleted item (the largest file).
    if let Some(&victim) = top.first() {
        let t = Instant::now();
        let rest = model.without(victim);
        println!(
            "delete:    {:.2}ms  ({} nodes left)",
            t.elapsed().as_secs_f64() * 1e3,
            thousands(rest.len() as u64)
        );
    }

    // Drive picker: each drive is queried on its own thread at start-up.
    for root in scan::win::drive_roots() {
        let t = Instant::now();
        let info = scan::win::drive_info(&root);
        println!(
            "drive {root}:  {:.2}ms  ({})",
            t.elapsed().as_secs_f64() * 1e3,
            info.map_or("not ready".into(), |d| format!("{:?}, {}", d.kind, d.fs))
        );
    }

    // Name search over the whole scan, run on every keystroke in the UI.
    for query in ["d", "dll", "setup", "а", "*.dll", "*a*b*", "a b c", "*.mp4;*.mkv;*.avi"] {
        let t = Instant::now();
        let count = model.search(0, query, model::Metric::Physical, 200, false, model::ItemKind::All).count;
        println!(
            "search {query:>5}: {:.2}ms  ({} matches)",
            t.elapsed().as_secs_f64() * 1e3,
            thousands(count as u64)
        );
    }
    // The chart with the matches of a search coloured in.
    let found = model.search(0, "d", model::Metric::Physical, 200, false, model::ItemKind::All);
    let t = Instant::now();
    let mesh = render::build_mesh(&model, &l, &render::Palette::default(), Some(&found.hits));
    println!(
        "mesh with matches: {:.2}ms  ({} vertices)",
        t.elapsed().as_secs_f64() * 1e3,
        thousands(mesh.vertices.len() as u64)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::normalize;
    use std::path::PathBuf;

    #[test]
    fn normalizes_explorer_arguments() {
        assert_eq!(normalize("C:"), PathBuf::from(r"C:\"));
        assert_eq!(normalize(r"C:\"), PathBuf::from(r"C:\"));
        assert_eq!(normalize("C:\""), PathBuf::from(r"C:\"));
        assert_eq!(normalize(r"D:\Projects"), PathBuf::from(r"D:\Projects"));
        assert_eq!(normalize(r#"D:\My Dir\""#), PathBuf::from(r"D:\My Dir\"));
    }
}
