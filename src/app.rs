//! Application state and the eframe `App` implementation.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use egui::{Key, Modifiers};

use crate::history::History;
use crate::model::{Metric, Model, NO_NODE};
use crate::scan::win::{DiskSpace, Drive};
use crate::scan::{self, Method, ScanHandle};
use crate::settings::Settings;
use crate::ui::about::AboutDialog;
use crate::ui::SideView;
use crate::ui::chart::{ChartAction, ChartView};
use crate::ui::files::FilesView;
use crate::ui::search::SearchView;
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
    pub files: FilesView,
    pub search: SearchView,
    pub side: SideView,
    pub tree_hovered: Option<u32>,
    pub about: AboutDialog,
    pub status: String,
    /// Process has administrator rights (enables the MFT scanner).
    pub elevated: bool,
    /// Capacity and free space when the scan covers a whole volume (or
    /// network share), for the free-space sector.
    pub disk: Option<DiskSpace>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, initial: Option<PathBuf>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::light());
        let settings = cc.storage.map(Settings::load).unwrap_or_default();
        let drives = scan::win::list_drives();
        // The last scanned path is offered, not scanned: in the path field,
        // and in the drive picker (so Rescan scans it) if it is a drive.
        let last_path = settings.last_path.unwrap_or_default();
        let drive_idx = drives
            .iter()
            .position(|d| d.root.eq_ignore_ascii_case(&last_path))
            .unwrap_or(usize::MAX);
        let mut app = Self {
            model: None,
            scan: None,
            drives,
            drive_idx,
            custom_path: last_path,
            nav: History::default(),
            metric: settings.metric,
            chart: ChartView::default(),
            tree: TreeView::default(),
            files: FilesView::default(),
            search: SearchView::default(),
            side: settings.side_view,
            tree_hovered: None,
            about: AboutDialog::default(),
            status: "Ready".into(),
            elevated: scan::win::is_elevated(),
            disk: None,
        };
        app.chart.palette.mode = settings.color_mode;
        app.tree.follow_hover = settings.follow_in_tree;
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
                // Reflect the scanned volume in the drive picker; a folder
                // scan shows no drive there.
                self.drive_idx = self
                    .drives
                    .iter()
                    .position(|d| d.root.eq_ignore_ascii_case(&model.root_path))
                    .unwrap_or(usize::MAX);
                // A rescan of the same path keeps the current folder (or its
                // closest surviving ancestor) and the history; anything else
                // starts at the root.
                match self.model.take() {
                    Some(old) if old.root_path.eq_ignore_ascii_case(&model.root_path) => {
                        self.nav.remap(|id| model.find_dir(&old.rel_path(id)));
                    }
                    _ => self.nav.reset(),
                }
                self.tree.reset();
                self.files.reset();
                self.search.reset();
                self.tree.reveal(&model, self.nav.root);
                let root = std::path::Path::new(&model.root_path);
                self.disk = if root.parent().is_none() {
                    scan::win::disk_space(root)
                } else {
                    None
                };
                self.model = Some(Arc::new(model));
                self.chart.invalidate();
                self.scan = None;
            }
            Some(Err(e)) => {
                self.status = format!("Scan failed: {e}");
                self.scan = None;
            }
        }
    }

    /// Path being scanned or last scanned: a running scan wins over the
    /// loaded result, which wins over the drive picker.
    fn scan_target(&self) -> Option<String> {
        self.scan
            .as_ref()
            .map(|h| h.path.display().to_string())
            .or_else(|| self.model.as_ref().map(|m| m.root_path.clone()))
            .or_else(|| self.drives.get(self.drive_idx).map(|d| d.root.clone()))
    }

    /// Whether restarting elevated would switch to the MFT scanner: when the
    /// target is on a local NTFS drive.
    pub fn fast_scan_available(&self) -> bool {
        let Some(target) = self.scan_target() else {
            return false;
        };
        let Some(letter) = scan::mft::drive_letter(std::path::Path::new(&target)) else {
            return false;
        };
        self.drives.iter().any(|d| {
            d.root.starts_with(letter) && d.fs.eq_ignore_ascii_case("NTFS")
        })
    }

    /// Restart elevated (UAC prompt) so the MFT scanner can be used, keeping
    /// the current scan target. Closes this window on success.
    pub fn relaunch_as_admin(&mut self, ctx: &egui::Context) {
        let target = self.scan_target().unwrap_or_default();
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
        let (back, fwd, up, rescan, about, find) = ctx.input(|i| {
            (
                i.modifiers.matches_exact(Modifiers::ALT) && i.key_pressed(Key::ArrowLeft),
                i.modifiers.matches_exact(Modifiers::ALT) && i.key_pressed(Key::ArrowRight),
                i.key_pressed(Key::Backspace),
                i.key_pressed(Key::F5),
                i.key_pressed(Key::F1),
                i.modifiers.matches_exact(Modifiers::COMMAND) && i.key_pressed(Key::F),
            )
        });
        if about {
            self.about.open = true;
        }
        if find {
            self.side = SideView::Search;
            self.search.focus = true;
        }
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
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let last_path = match &self.model {
            Some(m) => Some(m.root_path.clone()),
            None => Some(self.custom_path.trim().to_string()),
        };
        Settings {
            metric: self.metric,
            color_mode: self.chart.palette.mode,
            follow_in_tree: self.tree.follow_hover,
            side_view: self.side,
            last_path,
        }
        .save(storage);
    }

    fn ui(&mut self, root_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root_ui.ctx().clone();
        self.poll_scan(&ctx);
        if !self.about.open {
            self.handle_keys(&ctx);
        }
        self.about.show(&ctx);

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
                        ui.heading("Select a drive or enter a path to begin");
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
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(&mut self.side, SideView::Folders, "Folders");
                    ui.selectable_value(&mut self.side, SideView::LargestFiles, "Largest files")
                        .on_hover_text("The 100 largest files under the centre of the chart");
                    ui.selectable_value(&mut self.side, SideView::Search, "Search")
                        .on_hover_text("Find files and folders by name (Ctrl+F)");
                });
                ui.separator();
                let (root, metric, hovered) = (self.nav.root, self.metric, self.chart.hovered);
                tree_action = Some(match self.side {
                    SideView::Folders => self.tree.show(ui, &model, root, metric, hovered),
                    SideView::LargestFiles => self.files.show(ui, &model, root, metric, hovered),
                    SideView::Search => self.search.show(ui, &model, metric, hovered),
                });
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
                .show(
                    ui,
                    &model,
                    self.nav.root,
                    self.metric,
                    self.tree_hovered,
                    self.disk.filter(|_| self.nav.root == 0),
                );
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
pub fn quote_arg(s: &str) -> String {
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
