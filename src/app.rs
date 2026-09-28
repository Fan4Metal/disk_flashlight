//! Application state and the eframe `App` implementation.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use egui::{Key, Modifiers};

use crate::format::{human_size, thousands};
use crate::history::History;
use crate::model::{Metric, Model, NO_NODE};
use crate::scan::win::{DiskSpace, Drive};
use crate::scan::{self, Method, ScanHandle};
use crate::settings::Settings;
use crate::ui::about::AboutDialog;
use crate::ui::{DANGER, ItemCommand, SideView};
use crate::ui::chart::{ChartAction, ChartView};
use crate::ui::errors::ErrorsView;
use crate::ui::files::FilesView;
use crate::ui::search::SearchView;
use crate::ui::tree::TreeView;

pub struct App {
    pub model: Option<Arc<Model>>,
    pub scan: Option<ScanHandle>,
    /// Drives for the picker, by letter; each is added once its (possibly
    /// slow, for a network drive) query on a background thread returns.
    pub drives: Vec<Drive>,
    drive_rx: crossbeam_channel::Receiver<Option<Drive>>,
    /// Root of the drive selected in the picker (`C:\`).
    pub drive: Option<String>,
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
    /// Folders the last scan could not read.
    pub errors: ErrorsView,
    pub status: String,
    /// Process has administrator rights (enables the MFT scanner).
    pub elevated: bool,
    /// Capacity and free space when the scan covers a whole volume (or
    /// network share), for the free-space sector.
    pub disk: Option<DiskSpace>,
    /// Item waiting for the user to confirm its deletion, in that model.
    confirm_delete: Option<(Arc<Model>, u32)>,
    /// Deletion running on a background thread.
    deleting: Option<Deleting>,
}

/// A move to the Recycle Bin in progress.
struct Deleting {
    /// Model the item belongs to; if another replaced it meanwhile (a
    /// rescan), it is not edited.
    model: Arc<Model>,
    id: u32,
    path: String,
    rx: crossbeam_channel::Receiver<Result<(), String>>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, initial: Option<PathBuf>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::light());
        // Solid scroll bars take their own width; the default floating ones
        // are drawn over the right edge of list rows (sizes).
        cc.egui_ctx
            .all_styles_mut(|s| s.spacing.scroll = egui::style::ScrollStyle::solid());
        let settings = cc.storage.map(Settings::load).unwrap_or_default();
        let (tx, drive_rx) = crossbeam_channel::unbounded();
        for root in scan::win::drive_roots() {
            let (tx, ctx) = (tx.clone(), cc.egui_ctx.clone());
            let spawned = std::thread::Builder::new()
                .name(format!("drive {root}"))
                .spawn(move || {
                    let _ = tx.send(scan::win::drive_info(&root));
                    ctx.request_repaint();
                });
            if let Err(e) = spawned {
                log::warn!("drive query thread: {e}");
            }
        }
        // The last scanned path is offered, not scanned: in the path field,
        // and in the drive picker (so Rescan scans it) if it is a drive.
        let last_path = settings.last_path.unwrap_or_default();
        let mut app = Self {
            model: None,
            scan: None,
            drives: Vec::new(),
            drive_rx,
            drive: drive_root_of(&last_path),
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
            errors: ErrorsView::default(),
            status: "Ready".into(),
            elevated: scan::win::is_elevated(),
            disk: None,
            confirm_delete: None,
            deleting: None,
        };
        app.chart.palette.mode = settings.color_mode;
        app.tree.follow_hover = settings.follow_in_tree;
        app.search.whole_word = settings.search_whole_word;
        app.search.kind = settings.search_kind;
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
            .or_else(|| self.drive.as_ref().map(PathBuf::from));
        if let Some(p) = path {
            self.start_scan(p);
        }
    }

    /// Add drives whose background query has returned.
    fn poll_drives(&mut self) {
        while let Ok(info) = self.drive_rx.try_recv() {
            if let Some(d) = info {
                let at = self.drives.partition_point(|x| x.root < d.root);
                self.drives.insert(at, d);
            }
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
                // Errors are shown by a link next to the status.
                self.status = format!(
                    "Scanned {} files / {} dirs in {secs:.1}s via {method}",
                    crate::format::thousands(root.files as u64),
                    crate::format::thousands(root.dirs as u64),
                );
                self.errors.set(errors, h.progress.take_failed());
                if let Some(reason) = info.fallback_reason {
                    log::info!("MFT fallback: {reason}");
                }
                // Reflect the scanned volume in the drive picker; a folder
                // scan shows no drive there.
                self.drive = drive_root_of(&model.root_path);
                self.set_model(model);
                self.scan = None;
            }
            Some(Err(e)) => {
                self.status = format!("Scan failed: {e}");
                self.scan = None;
            }
        }
    }

    /// Show `model`, a new scan or an edited copy of the current one. The
    /// same path keeps the current folder (or its closest surviving
    /// ancestor) and the history; anything else starts at the root.
    fn set_model(&mut self, model: Model) {
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
        self.tree_hovered = None;
        let root = std::path::Path::new(&model.root_path);
        self.disk = if root.parent().is_none() {
            scan::win::disk_space(root)
        } else {
            None
        };
        self.model = Some(Arc::new(model));
        self.chart.invalidate();
    }

    /// Carry out a context-menu command on an item of the current model.
    fn run_command(&mut self, command: ItemCommand) {
        let Some(model) = self.model.clone() else { return };
        match command {
            ItemCommand::OpenInExplorer(id) => {
                scan::win::open_in_explorer(&model.path(id), model.node(id).is_dir);
            }
            ItemCommand::Properties(id) => {
                let path = model.path(id);
                if !scan::win::show_properties(&path) {
                    self.status = format!("No properties available for {path}");
                }
            }
            ItemCommand::Delete(id) if id != 0 => {
                if self.deleting.is_some() {
                    self.status = "Another deletion is still running".into();
                } else {
                    self.confirm_delete = Some((model, id));
                }
            }
            ItemCommand::Delete(_) => {}
        }
    }

    /// The confirmation dialog for `confirm_delete`; on "Move to Recycle
    /// Bin" the deletion starts on a background thread.
    fn show_confirm_delete(&mut self, ctx: &egui::Context) {
        let Some((model, id)) = self.confirm_delete.clone() else { return };
        let n = model.node(id);
        let (mut confirm, mut cancel) = (false, false);
        let modal = egui::Modal::new(egui::Id::new("confirm_delete")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading("Move to the Recycle Bin?");
            ui.add_space(6.0);
            ui.strong(format!("{}{}", model.name(id), if n.is_dir { "\\" } else { "" }));
            ui.label(if n.is_dir {
                format!(
                    "{} in {} files and {} folders",
                    human_size(n.size),
                    thousands(n.files as u64),
                    thousands(n.dirs as u64)
                )
            } else {
                human_size(n.size)
            });
            ui.add(egui::Label::new(egui::RichText::new(model.path(id)).weak()).wrap());
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let text = |s: &str| egui::RichText::new(s).size(16.0);
                let delete = egui::Button::new(text("Move to Recycle Bin").color(egui::Color32::WHITE))
                    .fill(DANGER)
                    .min_size(egui::vec2(180.0, 34.0));
                confirm = ui.add(delete).clicked();
                cancel = ui
                    .add(egui::Button::new(text("Cancel")).min_size(egui::vec2(100.0, 34.0)))
                    .clicked();
            });
        });
        if cancel || modal.should_close() {
            self.confirm_delete = None;
        } else if confirm {
            self.confirm_delete = None;
            // A rescan may have replaced the model while the dialog was open.
            if self.model.as_ref().is_some_and(|m| Arc::ptr_eq(m, &model)) {
                self.start_delete(model, id, ctx);
            }
        }
    }

    fn start_delete(&mut self, model: Arc<Model>, id: u32, ctx: &egui::Context) {
        let path = model.path(id);
        let (tx, rx) = crossbeam_channel::bounded(1);
        let (p, ctx) = (path.clone(), ctx.clone());
        let spawned = std::thread::Builder::new().name("recycle".into()).spawn(move || {
            let _ = tx.send(scan::win::recycle(&p));
            ctx.request_repaint();
        });
        if let Err(e) = spawned {
            self.status = format!("Could not start deleting: {e}");
            return;
        }
        self.status = format!("Moving {path} to the Recycle Bin…");
        self.deleting = Some(Deleting { model, id, path, rx });
    }

    /// Finish a deletion: what is gone from disk leaves the model too.
    fn poll_delete(&mut self) {
        let Some(d) = &self.deleting else { return };
        let Ok(result) = d.rx.try_recv() else { return };
        let Some(d) = self.deleting.take() else { return };
        let gone = std::fs::symlink_metadata(&d.path).is_err();
        let size = human_size(d.model.node(d.id).metric(self.metric));
        self.status = match (gone, result) {
            (true, _) => format!("Moved {} ({size}) to the Recycle Bin", d.path),
            (false, Err(e)) => format!("{} was not deleted: {e}", d.path),
            (false, Ok(())) => format!("{} was not deleted completely; rescan to update", d.path),
        };
        if gone && self.model.as_ref().is_some_and(|m| Arc::ptr_eq(m, &d.model)) {
            self.set_model(d.model.without(d.id));
        }
    }

    /// Path being scanned or last scanned: a running scan wins over the
    /// loaded result, which wins over the drive picker.
    fn scan_target(&self) -> Option<String> {
        self.scan
            .as_ref()
            .map(|h| h.path.display().to_string())
            .or_else(|| self.model.as_ref().map(|m| m.root_path.clone()))
            .or_else(|| self.drive.clone())
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
            search_whole_word: self.search.whole_word,
            search_kind: self.search.kind,
            last_path,
        }
        .save(storage);
    }

    fn ui(&mut self, root_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root_ui.ctx().clone();
        self.poll_drives();
        self.poll_scan(&ctx);
        self.poll_delete();
        self.show_confirm_delete(&ctx);
        if !self.about.open && self.confirm_delete.is_none() {
            self.handle_keys(&ctx);
        }
        self.about.show(&ctx);
        self.errors.show(&ctx, self.elevated);

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
                    SideView::Folders => {
                        self.tree
                            .show(ui, &model, root, metric, hovered, self.chart.pointer_inside)
                    }
                    SideView::LargestFiles => self.files.show(ui, &model, root, metric, hovered),
                    SideView::Search => self.search.show(ui, &model, metric, hovered),
                });
            });
        if let Some(a) = tree_action {
            self.tree_hovered = a.hovered;
            if let Some(id) = a.selected {
                self.navigate(id);
            }
            if let Some(c) = a.command {
                self.run_command(c);
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
                    self.search.highlight().filter(|_| self.side == SideView::Search),
                );
        });
        match chart_action {
            ChartAction::None => {}
            ChartAction::Navigate(id) => self.navigate(id),
            ChartAction::Up => self.go_up(),
            ChartAction::Command(c) => self.run_command(c),
        }
    }
}

/// `C:\` (also `c:`) -> `Some("C:\\")`; any other path -> `None`.
fn drive_root_of(path: &str) -> Option<String> {
    let p = std::path::Path::new(path);
    let letter = scan::mft::drive_letter(p)?;
    p.parent().is_none().then(|| format!("{letter}:\\"))
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
    use super::{drive_root_of, quote_arg};

    #[test]
    fn drive_roots() {
        assert_eq!(drive_root_of(r"C:\").as_deref(), Some(r"C:\"));
        assert_eq!(drive_root_of("d:").as_deref(), Some(r"D:\"));
        assert_eq!(drive_root_of(r"D:\Projects"), None);
        assert_eq!(drive_root_of(r"\\server\share"), None);
        assert_eq!(drive_root_of(""), None);
    }

    #[test]
    fn quoting() {
        assert_eq!(quote_arg(r"C:\"), r"C:\");
        assert_eq!(quote_arg(r"D:\My Files"), r#""D:\My Files""#);
        assert_eq!(quote_arg(r"D:\My Files\"), r#""D:\My Files\\""#);
    }
}
