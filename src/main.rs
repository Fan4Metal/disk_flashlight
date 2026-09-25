#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod format;
mod history;
mod icon;
mod layout;
mod model;
mod render;
mod scan;
mod ui;

use std::path::PathBuf;
use std::time::Instant;

use format::{human_size, thousands};

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let mut args = std::env::args().skip(1);
    let mut initial: Option<PathBuf> = None;
    let mut bench_path: Option<PathBuf> = None;
    let mut allow_mft = true;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--bench" => {
                bench_path = Some(normalize(&args.next().unwrap_or_else(|| "C:".into())));
            }
            "--walk" => allow_mft = false,
            "-h" | "--help" => {
                println!("disk_flashlight [PATH] | --bench PATH [--walk]");
                return Ok(());
            }
            other => initial = Some(normalize(other)),
        }
    }
    if let Some(p) = bench_path {
        return bench(p, allow_mft);
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Disk Flashlight")
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([640.0, 420.0])
            .with_icon(egui::IconData {
                rgba: icon::rgba(64),
                width: 64,
                height: 64,
            }),
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "Disk Flashlight",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, initial)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

/// `C:` means "current directory on C" to Windows; a bare drive letter given
/// by the user always means the volume root.
fn normalize(arg: &str) -> PathBuf {
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
    let l = layout::build(&model, 0, model::Metric::Physical, center, 380.0, &params);
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
    Ok(())
}
