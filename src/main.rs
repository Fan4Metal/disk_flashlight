#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod format;
mod history;
mod icon;
mod layout;
mod model;
mod render;
mod scan;
mod settings;
mod ui;

use std::path::PathBuf;
use std::time::Instant;

use format::{human_size, thousands};

/// Version from Cargo.toml, shared by the window title, `--version`, the
/// installer and the GitHub release tag.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// eframe app id; also names the settings folder in `%APPDATA%`.
const APP_ID: &str = "Disk Flashlight";

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
        .any(|a| matches!(a.as_str(), "--bench" | "--version" | "-V" | "-h" | "--help"));
    #[cfg(windows)]
    if cli_mode {
        attach_parent_console();
    }
    let mut args = std::env::args().skip(1);
    let mut initial: Option<PathBuf> = None;
    let mut bench_path: Option<PathBuf> = None;
    let mut allow_mft = true;
    let mut want_mft = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--bench" => {
                bench_path = Some(normalize(&args.next().unwrap_or_else(|| "C:".into())));
            }
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
                println!("disk_flashlight [--mft] [PATH] | --bench PATH [--walk] | --export-icon FILE | --version");
                println!("  --mft   restart as administrator (UAC prompt) so that a whole NTFS drive");
                println!("          is scanned through the MFT; ignored where it would not help");
                return Ok(());
            }
            other => initial = Some(normalize(other)),
        }
    }
    if let Some(p) = bench_path {
        return bench(p, allow_mft);
    }
    if want_mft && elevate_for_mft(initial.as_deref()) {
        return Ok(()); // the elevated copy takes over
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("Disk Flashlight {VERSION}"))
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([640.0, 420.0])
            .with_icon(egui::IconData {
                rgba: icon::rgba(64),
                width: 64,
                height: 64,
            }),
        // Centre only on the first run; later runs restore the saved window.
        centered: !eframe::storage_dir(APP_ID).is_some_and(|d| d.join("app.ron").exists()),
        // 4x MSAA: the chart is one big triangle mesh without egui's edge
        // feathering, so thin sectors and arcs would otherwise be jagged.
        // DF_MSAA=1 turns it off (for comparison or a GPU without MSAA).
        multisampling: std::env::var("DF_MSAA")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4),
        ..Default::default()
    };
    eframe::run_native(
        APP_ID,
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, initial)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
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
            scan::win::list_drives().iter().any(|d| {
                d.root.starts_with(letter) && d.fs.eq_ignore_ascii_case("NTFS")
            })
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
fn normalize(arg: &str) -> PathBuf {
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

    println!("top-level (largest first):");
    for c in model.children(0).take(12) {
        let n = model.node(c);
        println!(
            "  {:>10}  {}{}",
            human_size(n.size),
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
    let mesh = render::build_mesh(&model, &l, &render::Palette::default());
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

    // Zoomed in: the 1200x800 view looks at the rings above the centre, so
    // most of the chart is culled. Work must stay bounded by the view.
    for zoom in [10.0f32, 50.0, 200.0] {
        let radius = 380.0 * zoom;
        let c = egui::Pos2::new(600.0, 400.0 + radius * 0.45);
        let t = Instant::now();
        let l = layout::build(&model, 0, model::Metric::Physical, c, radius, view, &params);
        let mesh = render::build_mesh(&model, &l, &render::Palette::default());
        println!(
            "zoom {zoom:>3}: {:.2}ms  ({} sectors, {} vertices)",
            t.elapsed().as_secs_f64() * 1e3,
            thousands(l.sector_count() as u64),
            thousands(mesh.vertices.len() as u64)
        );
    }
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
