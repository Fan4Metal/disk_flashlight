#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod format;
mod history;
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
    while let Some(a) = args.next() {
        match a.as_str() {
            "--bench" => {
                let path = args.next().unwrap_or_else(|| "C:\\".into());
                return bench(PathBuf::from(path));
            }
            "-h" | "--help" => {
                println!("disk_flashlight [PATH] | --bench PATH");
                return Ok(());
            }
            other => initial = Some(PathBuf::from(other)),
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Disk Flashlight")
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([640.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Disk Flashlight",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, initial)))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

/// `--bench PATH`: scan, pack, lay out and tessellate, printing timings.
fn bench(path: PathBuf) -> anyhow::Result<()> {
    let progress = scan::Progress::default();
    let t = Instant::now();
    let model = scan::walk::scan(&path, &progress)?;
    let scan_time = t.elapsed();
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
