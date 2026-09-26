//! Application state and the eframe `App` implementation.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use egui::{Key, Modifiers};

use crate::history::History;
use crate::model::{Metric, Model, NO_NODE};
use crate::scan::{self, Method, ScanHandle, win::Drive};
use crate::ui::chart::{ChartAction, ChartView};
use crate::ui::tree::TreeView;

pub struct App {
    pub model: Option<Arc<Model>>,
    pub scan: Option<ScanHandle>,
    pub drives: Vec<Drive>,
    pub drive_idx: usize,
    pub custom_path: String,
    pub nav: History,
    pub metric: Metric,
    pub chart: ChartView,
    pub tree: TreeView,
    pub tree_hovered: Option<u32>,
    pub status: String,
    /// Process has administrator rights (enables the MFT scanner).
    pub elevated: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, initial: Option<PathBuf>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::light());
        let drives = scan::win::list_drives();
        let mut app = Self {
            model: None,
            scan: None,
            drives,
            drive_idx: usize::MAX,
            custom_path: String::new(),
            nav: History::default(),
            metric: Metric::Physical,
            chart: ChartView::default(),
            tree: TreeView::default(),
            tree_hovered: None,
            status: "Ready".into(),
            elevated: scan::win::is_elevated(),
        };
        if let Some(p) = initial {
            app.start_scan(p);
        }
        app
    }

    pub fn start_scan(&mut self, path: PathBuf) {
        if let Some(h) = &self.scan {
            h.cancel();
        }
        self.status = format!("Scanning {}", path.display());
        self.scan = Some(scan::start(path));
    }

    pub fn rescan(&mut self) {
        let path = self
            .model
            .as_ref()
            .map(|m| PathBuf::from(&m.root_path))
            .or_else(|| self.drives.get(self.drive_idx).map(|d| PathBuf::from(&d.root)));
        if let Some(p) = path {
            self.start_scan(p);
        }
    }

    fn poll_scan(&mut self, ctx: &egui::Context) {
        let Some(h) = &self.scan else { return };
        match h.try_result() {
            None => {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            Some(Ok((model, info))) => {
                let secs = h.started.elapsed().as_secs_f32();
                let (_, _, _, errors) = h.progress.snapshot();
                let root = model.node(0);
                let method = match info.method {
                    Method::Mft => "MFT",
                    Method::Walk => "directory walk",
                };
                self.status = format!(
                    "Scanned {} files / {} dirs in {secs:.1}s via {method}{}",
                    crate::format::thousands(root.files as u64),
                    crate::format::thousands(root.dirs as u64),
                    if errors > 0 {
                        format!(", {errors} access errors")
                    } else {
                        String::new()
                    }
                );
                if let Some(reason) = info.fallback_reason {
                    log::info!("MFT fallback: {reason}");
                }
                // Reflect the scanned volume in the drive picker.
                if let Some(i) = self
                    .drives
                    .iter()
                    .position(|d| d.root.eq_ignore_ascii_case(&model.root_path))
                {
                    self.drive_idx = i;
                }
                self.model = Some(Arc::new(model));
                self.nav.reset();
                self.tree.reset();
                self.chart.invalidate();
                self.scan = None;
            }
            Some(Err(e)) => {
                self.status = format!("Scan failed: {e}");
                self.scan = None;
            }
        }
    }

    /// Restart elevated (UAC prompt) so the MFT scanner can be used, keeping
    /// the current scan target. Closes this window on success.
    pub fn relaunch_as_admin(&mut self, ctx: &egui::Context) {
        let target = self
            .model
            .as_ref()
            .map(|m| m.root_path.clone())
            .or_else(|| self.scan.as_ref().map(|h| h.path.display().to_string()))
            .or_else(|| self.drives.get(self.drive_idx).map(|d| d.root.clone()))
            .unwrap_or_default();
        if scan::win::relaunch_elevated(&quote_arg(&target)) {
            if let Some(h) = &self.scan {
                h.cancel();
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            self.status = "Elevation was cancelled".into();
        }
    }

    pub fn navigate(&mut self, id: u32) {
        if self.nav.navigate(id) {
            self.after_root_change();
        }
    }

    fn after_root_change(&mut self) {
        if let Some(m) = &self.model {
            self.tree.reveal(m, self.nav.root);
        }
    }

    pub fn go_back(&mut self) {
        if self.nav.back() {
            self.after_root_change();
        }
    }

    pub fn go_forward(&mut self) {
        if self.nav.forward() {
            self.after_root_change();
        }
    }

    pub fn go_up(&mut self) {
        let Some(m) = &self.model else { return };
        let parent = m.node(self.nav.root).parent;
        if parent != NO_NODE {
            self.navigate(parent);
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        // Do not steal keys from a focused text field.
        if ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let (back, fwd, up, rescan) = ctx.input(|i| {
            (
                i.modifiers.matches_exact(Modifiers::ALT) && i.key_pressed(Key::ArrowLeft),
                i.modifiers.matches_exact(Modifiers::ALT) && i.key_pressed(Key::ArrowRight),
                i.key_pressed(Key::Backspace),
                i.key_pressed(Key::F5),
            )
        });
        if back {
            self.go_back();
        }
        if fwd {
            self.go_forward();
        }
        if up {
            self.go_up();
        }
        if rescan {
            self.rescan();
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, root_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root_ui.ctx().clone();
        self.poll_scan(&ctx);
        self.handle_keys(&ctx);

        egui::Panel::top("toolbar").show(root_ui, |ui| {
            ui.add_space(2.0);
            self.toolbar(ui);
            ui.add_space(2.0);
        });
        egui::Panel::bottom("status").show(root_ui, |ui| {
            self.status_bar(ui);
        });

        let Some(model) = self.model.clone() else {
            egui::CentralPanel::default().show(root_ui, |ui| {
                ui.centered_and_justified(|ui| {
                    if self.scan.is_some() {
                        ui.add(egui::Spinner::new().size(48.0));
                    } else {
                        ui.heading("Select a drive to begin");
                    }
                });
            });
            return;
        };

        let mut tree_action = None;
        egui::Panel::left("tree")
            .resizable(true)
            .default_size(300.0)
            .min_size(160.0)
            .show(root_ui, |ui| {
                tree_action = Some(self.tree.show(
                    ui,
                    &model,
                    self.nav.root,
                    self.metric,
                    self.chart.hovered,
                ));
            });
        if let Some(a) = tree_action {
            self.tree_hovered = a.hovered;
            if let Some(id) = a.selected {
                self.navigate(id);
            }
        }

        let mut chart_action = ChartAction::None;
        egui::CentralPanel::no_frame().show(root_ui, |ui| {
                chart_action = self
                .chart
                .show(ui, &model, self.nav.root, self.metric, self.tree_hovered);
        });
        match chart_action {
            ChartAction::None => {}
            ChartAction::Navigate(id) => self.navigate(id),
            ChartAction::Up => self.go_up(),
            ChartAction::OpenInExplorer(id) => {
                let path = model.path(id);
                scan::win::open_in_explorer(&path, model.node(id).is_dir);
            }
            ChartAction::Properties(id) => {
                let path = model.path(id);
                if !scan::win::show_properties(&path) {
                    self.status = format!("No properties available for {path}");
                }
            }
        }
    }
}

/// Quote a single command-line argument for `CommandLineToArgvW`, which
/// treats backslashes before a closing quote as escapes: `"D:\My Dir\"`
/// would swallow its closing quote, so trailing backslashes are doubled.
fn quote_arg(s: &str) -> String {
    if !s.contains([' ', '\t', '"']) {
        return s.to_string();
    }
    let trailing = s.len() - s.trim_end_matches('\\').len();
    format!("\"{}{}\"", s.replace('"', "\\\""), "\\".repeat(trailing))
}

#[cfg(test)]
mod tests {
    use super::quote_arg;

    #[test]
    fn quoting() {
        assert_eq!(quote_arg(r"C:\"), r"C:\");
        assert_eq!(quote_arg(r"D:\My Files"), r#""D:\My Files""#);
        assert_eq!(quote_arg(r"D:\My Files\"), r#""D:\My Files\\""#);
    }
}
