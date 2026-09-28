//! Application state and the eframe `App` implementation.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use egui::{Key, Modifiers};

use crate::format::human_size;
use crate::history::History;
use crate::model::{Metric, Model, NO_NODE};
use crate::scan::win::{DiskSpace, Drive};
use crate::scan::{self, Method, ScanHandle};
use crate::settings::Settings;
use crate::ui::about::AboutDialog;
use crate::ui::{DANGER, ItemCommand, SideView};
use crate::ui::chart::{ChartAction, ChartView};
use crate::ui::errors::ErrorsView;
use crate::ui::path_field::PathField;
use crate::ui::files::FilesView;
use crate::ui::search::SearchView;
use crate::ui::tree::TreeView;

pub struct App {
    /// Light, dark or as in Windows (the toolbar's theme button).
    pub theme: egui::ThemePreference,
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
    /// "Choose folder…" was picked: open the folder dialog.
    pub choose_folder: bool,
    /// The toolbar's path field.
    pub path_edit: PathField,
    /// Icon of the start screen, rasterised on first show.
    start_icon: Option<egui::TextureHandle>,
    /// Folders scanned recently, newest first, offered in the drive picker.
    pub recent: Vec<String>,
    /// Theme of the last frame, to notice a change.
    shown_theme: Option<egui::Theme>,
    /// Repaint the window caption on this frame (see `follow_theme`).
    refresh_caption: bool,
}

/// Number of folders kept in [`App::recent`].
pub const MAX_RECENT: usize = 8;

/// Put `path` at the front of the recent folders `list`, dropping an older
/// entry for the same folder and anything past [`MAX_RECENT`]. Drive roots
/// are left out: the drive picker lists them anyway.
pub fn push_recent(list: &mut Vec<String>, path: &str) {
    if path.is_empty() || drive_root_of(path).is_some() {
        return;
    }
    list.retain(|p| !same_path(p, path));
    list.insert(0, path.to_string());
    list.truncate(MAX_RECENT);
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
        // Solid scroll bars take their own width; the default floating ones
        // are drawn over the right edge of list rows (sizes).
        cc.egui_ctx
            .all_styles_mut(|s| s.spacing.scroll = egui::style::ScrollStyle::solid());
        let settings = cc.storage.map(Settings::load).unwrap_or_default();
        // Before any text is made.
        crate::i18n::set_lang(settings.language.resolve());
        cc.egui_ctx.set_theme(settings.theme);
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
        // The last scanned path is only offered in the path field: nothing is
        // selected in the drive picker and nothing is scanned (a slow or
        // network drive would start working unasked).
        let last_path = settings.last_path.unwrap_or_default();
        let mut app = Self {
            theme: settings.theme,
            model: None,
            scan: None,
            drives: Vec::new(),
            drive_rx,
            drive: None,
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
            status: tr!("Ready", "Готово").into(),
            elevated: scan::win::is_elevated(),
            disk: None,
            confirm_delete: None,
            deleting: None,
            choose_folder: false,
            path_edit: PathField::default(),
            start_icon: None,
            recent: settings.recent,
            shown_theme: None,
            refresh_caption: false,
        };
        app.chart.palette.mode = settings.color_mode;
        app.about.lang = settings.language;
        app.tree.follow_hover = settings.follow_in_tree;
        app.search.whole_word = settings.search_whole_word;
        app.search.kind = settings.search_kind;
        app.search.sort = settings.search_sort;
        app.files.sort = settings.files_sort;
        app.files.older_than = settings.files_older_than;
        if let Some(p) = initial {
            app.start_scan(p);
        }
        app
    }

    pub fn start_scan(&mut self, path: PathBuf) {
        // A mistyped path must not cost the results on screen. Network
        // paths are left to the scan thread: a server that is away can
        // take long to answer.
        if !is_remote(&self.drives, &path) {
            let problem = match std::fs::metadata(&path) {
                Ok(m) if m.is_dir() => None,
                Ok(_) => Some(tr!("not a folder", "это не папка").to_string()),
                Err(e) => Some(e.to_string()),
            };
            if let Some(problem) = problem {
                self.status = tr!(
                    format!("Cannot scan {}: {problem}", path.display()),
                    format!("Не удаётся просканировать {}: {problem}", path.display())
                );
                return;
            }
        }
        if let Some(h) = &self.scan {
            h.cancel();
        }
        // Another drive or folder: its old results go at once, and the
        // spinner shows until the new ones arrive. A rescan of the same
        // path keeps them, and then the current folder and the history.
        let path_str = path.to_string_lossy();
        if self.model.as_ref().is_some_and(|m| !same_path(&m.root_path, &path_str)) {
            self.clear_model();
        }
        self.status = tr!(format!("Scanning {}", path.display()), format!("Сканирование {}", path.display()));
        self.scan = Some(scan::start(path));
    }

    /// The system's folder dialog, modal to the window, starting at the
    /// current scan; the chosen folder is scanned.
    fn pick_folder(&mut self, parent: &eframe::Frame) {
        let start = self
            .model
            .as_ref()
            .map(|m| m.root_path.clone())
            .or_else(|| self.drive.clone())
            .unwrap_or_else(|| self.custom_path.trim().to_string());
        let mut dialog = rfd::FileDialog::new()
            .set_title(tr!("Choose a folder to scan", "Выберите папку для сканирования"))
            .set_parent(parent);
        if !start.is_empty() {
            dialog = dialog.set_directory(&start);
        }
        if let Some(dir) = dialog.pick_folder() {
            let path = dir.to_string_lossy().into_owned();
            self.drive = drive_root_of(&path);
            self.custom_path = path;
            self.start_scan(dir);
        }
    }

    /// Show no scan result: the chart, the tree and the lists go.
    fn clear_model(&mut self) {
        self.model = None;
        self.nav.reset();
        self.tree.reset();
        self.files.reset();
        self.search.reset();
        self.chart.invalidate();
        self.tree_hovered = None;
        self.disk = None;
        self.errors = ErrorsView::default();
        self.confirm_delete = None;
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
                    Method::Walk => tr!("directory walk", "обход папок"),
                };
                // Errors are shown by a link next to the status.
                let (files, dirs) = (root.files as u64, root.dirs as u64);
                self.status = tr!(
                    format!(
                        "Scanned {} / {} in {secs:.1} s via {method}",
                        crate::i18n::files(files),
                        crate::i18n::folders(dirs)
                    ),
                    format!(
                        "Просканировано: {}, {} за {} с ({method})",
                        crate::i18n::files(files),
                        crate::i18n::folders(dirs),
                        format!("{secs:.1}").replace('.', ",")
                    )
                );
                self.errors.set(errors, h.progress.take_failed());
                if let Some(reason) = info.fallback_reason {
                    log::info!("MFT fallback: {reason}");
                }
                // Reflect the scanned volume in the drive picker; a folder
                // scan shows no drive there, but joins the recent folders.
                self.drive = drive_root_of(&model.root_path);
                push_recent(&mut self.recent, &model.root_path);
                self.set_model(model);
                self.scan = None;
            }
            Some(Err(e)) => {
                self.status = tr!(format!("Scan failed: {e}"), format!("Сканирование не удалось: {e}"));
                self.scan = None;
            }
        }
    }

    /// Show `model`, a new scan or an edited copy of the current one. The
    /// same path keeps the current folder (or its closest surviving
    /// ancestor) and the history; anything else starts at the root.
    fn set_model(&mut self, model: Model) {
        match self.model.take() {
            Some(old) if same_path(&old.root_path, &model.root_path) => {
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
    fn run_command(&mut self, command: ItemCommand, ctx: &egui::Context) {
        let Some(model) = self.model.clone() else { return };
        match command {
            ItemCommand::OpenInExplorer(id) => {
                scan::win::open_in_explorer(&model.path(id), model.node(id).is_dir);
            }
            ItemCommand::Properties(id) => {
                let path = model.path(id);
                if !scan::win::show_properties(&path) {
                    self.status = tr!(
                        format!("No properties available for {path}"),
                        format!("Свойства недоступны: {path}")
                    );
                }
            }
            ItemCommand::CopyPath(id) => {
                let path = model.path(id);
                ctx.copy_text(path.clone());
                self.status = tr!(format!("Copied {path}"), format!("Скопирован путь {path}"));
            }
            ItemCommand::Delete(id) if id != 0 => {
                if self.deleting.is_some() {
                    self.status = tr!("Another deletion is still running", "Предыдущее удаление ещё не закончено").into();
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
            ui.heading(tr!("Move to the Recycle Bin?", "Переместить в корзину?"));
            ui.add_space(6.0);
            ui.strong(format!("{}{}", model.name(id), if n.is_dir { "\\" } else { "" }));
            ui.label(if n.is_dir {
                let (files, dirs) = (crate::i18n::files(n.files as u64), crate::i18n::folders(n.dirs as u64));
                tr!(
                    format!("{} in {files} and {dirs}", human_size(n.size)),
                    format!("{}: {files}, {dirs}", human_size(n.size))
                )
            } else {
                human_size(n.size)
            });
            ui.add(egui::Label::new(egui::RichText::new(model.path(id)).weak()).wrap());
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let text = |s: &str| egui::RichText::new(s).size(16.0);
                let delete = egui::Button::new(
                    text(tr!("Move to Recycle Bin", "В корзину")).color(egui::Color32::WHITE),
                )
                    .fill(DANGER)
                    .min_size(egui::vec2(180.0, 34.0));
                confirm = ui.add(delete).clicked();
                cancel = ui
                    .add(egui::Button::new(text(tr!("Cancel", "Отмена"))).min_size(egui::vec2(100.0, 34.0)))
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
            self.status = tr!(format!("Could not start deleting: {e}"), format!("Не удалось начать удаление: {e}"));
            return;
        }
        self.status = tr!(
            format!("Moving {path} to the Recycle Bin…"),
            format!("Перемещение {path} в корзину…")
        );
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
            (true, _) => tr!(
                format!("Moved {} ({size}) to the Recycle Bin", d.path),
                format!("Перемещено в корзину: {} ({size})", d.path)
            ),
            (false, Err(e)) => tr!(
                format!("{} was not deleted: {e}", d.path),
                format!("Не удалось удалить {}: {e}", d.path)
            ),
            (false, Ok(())) => tr!(
                format!("{} was not deleted completely; rescan to update", d.path),
                format!("Удалено не полностью: {}; для обновления просканируйте заново", d.path)
            ),
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
            self.status = tr!("Elevation was cancelled", "Запуск с правами администратора отменён").into();
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

    /// Bring the window caption in line with the theme. egui passes a new
    /// theme to the window at the end of the frame it changed in, and
    /// Windows applies it to the caption only on the next repaint of the
    /// frame, so the frame after the change asks for one.
    fn follow_theme(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if std::mem::take(&mut self.refresh_caption)
            && let Ok(handle) = frame.window_handle()
            && let RawWindowHandle::Win32(w) = handle.as_raw()
        {
            scan::win::refresh_caption(w.hwnd.get());
        }
        let theme = ctx.theme();
        if self.shown_theme.is_some_and(|t| t != theme) {
            self.refresh_caption = true;
            ctx.request_repaint();
        }
        self.shown_theme = Some(theme);
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        // Do not steal keys from a focused text field.
        if ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        // Ctrl+C arrives as a Copy event rather than as a key press.
        let copy = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)));
        if copy && let Some(m) = &self.model {
            // The item under the mouse in the chart or a list, else the
            // centre of the chart.
            let id = self.chart.hovered.or(self.tree_hovered).unwrap_or(self.nav.root);
            if (id as usize) < m.len() {
                self.run_command(ItemCommand::CopyPath(id), ctx);
            }
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
            theme: self.theme,
            language: self.about.lang,
            metric: self.metric,
            color_mode: self.chart.palette.mode,
            follow_in_tree: self.tree.follow_hover,
            side_view: self.side,
            search_whole_word: self.search.whole_word,
            search_kind: self.search.kind,
            search_sort: self.search.sort,
            files_sort: self.files.sort,
            files_older_than: self.files.older_than,
            last_path,
            recent: self.recent.clone(),
        }
        .save(storage);
    }

    fn ui(&mut self, root_ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = root_ui.ctx().clone();
        // eframe shows the window after the first frame; maximize it from
        // the second one (see `main::MAXIMIZE_WHEN_SHOWN`).
        if ctx.cumulative_frame_nr() >= 1
            && crate::MAXIMIZE_WHEN_SHOWN.swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }
        self.follow_theme(&ctx, frame);
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
        if std::mem::take(&mut self.choose_folder) {
            self.pick_folder(frame);
        }

        let Some(model) = self.model.clone() else {
            egui::CentralPanel::default().show(root_ui, |ui| match &self.scan {
                Some(h) => {
                    let (files, dirs, _, _) = h.progress.snapshot();
                    ui.vertical_centered(|ui| {
                        ui.add_space((ui.available_height() * 0.5 - 60.0).max(0.0));
                        ui.add(egui::Spinner::new().size(48.0));
                        ui.add_space(12.0);
                        ui.heading(tr!(
                            format!("Scanning {}", h.path.display()),
                            format!("Сканирование {}", h.path.display())
                        ));
                        ui.weak(format!(
                            "{}, {}",
                            crate::i18n::files(files),
                            crate::i18n::folders(dirs)
                        ));
                    });
                }
                None => {
                    // Start screen: the icon, the name and what to do.
                    let icon = crate::ui::app_icon(ui.ctx(), START_ICON, &mut self.start_icon);
                    ui.vertical_centered(|ui| {
                        ui.add_space((ui.available_height() * 0.5 - START_ICON).max(0.0));
                        ui.image((icon.id(), egui::vec2(START_ICON, START_ICON)));
                        ui.add_space(14.0);
                        ui.label(egui::RichText::new("Disk Flashlight").size(30.0).strong());
                        ui.weak(tr!(
                            format!("Version {}", crate::VERSION),
                            format!("Версия {}", crate::VERSION)
                        ));
                        ui.add_space(18.0);
                        ui.label(tr!(
                            "Select a drive, choose a folder or enter a path to begin",
                            "Выберите диск или папку либо введите путь"
                        ));
                    });
                }
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
                    ui.selectable_value(&mut self.side, SideView::Folders, tr!("Folders", "Папки"));
                    ui.selectable_value(
                        &mut self.side,
                        SideView::LargestFiles,
                        tr!("Largest files", "Крупные файлы"),
                    )
                    .on_hover_text(tr!(
                        "The 100 largest files under the centre of the chart",
                        "100 самых больших файлов в центре диаграммы"
                    ));
                    ui.selectable_value(&mut self.side, SideView::Search, tr!("Search", "Поиск"))
                        .on_hover_text(tr!(
                            "Find files and folders by name (Ctrl+F)",
                            "Поиск файлов и папок по имени (Ctrl+F)"
                        ));
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
                self.run_command(c, &ctx);
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
            ChartAction::Command(c) => self.run_command(c, &ctx),
        }
    }
}

/// Whether `path` is on a network share or on one of the network `drives`.
pub fn is_remote(drives: &[Drive], path: &std::path::Path) -> bool {
    if path.to_string_lossy().starts_with(r"\\") {
        return true;
    }
    scan::mft::drive_letter(path).is_some_and(|letter| {
        drives.iter().any(|d| {
            d.root.starts_with(letter.to_ascii_uppercase()) && d.kind == scan::win::DriveKind::Remote
        })
    })
}

/// Size of the icon on the start screen, in points.
const START_ICON: f32 = 112.0;

/// Whether two paths name the same folder, ignoring case and a trailing
/// backslash (`D:\` and `d:`).
fn same_path(a: &str, b: &str) -> bool {
    a.trim_end_matches('\\').eq_ignore_ascii_case(b.trim_end_matches('\\'))
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
    use super::{MAX_RECENT, drive_root_of, push_recent, quote_arg, same_path};

    #[test]
    fn recent_folders() {
        let mut list = Vec::new();
        push_recent(&mut list, r"D:\Projects");
        push_recent(&mut list, r"D:\Films");
        // A drive root is not kept, the same folder moves to the front.
        push_recent(&mut list, r"C:\");
        push_recent(&mut list, r"d:\projects\");
        assert_eq!(list, [r"d:\projects\", r"D:\Films"]);
        for i in 0..20 {
            push_recent(&mut list, &format!(r"E:\f{i}"));
        }
        assert_eq!(list.len(), MAX_RECENT);
        assert_eq!(list[0], r"E:\f19");
    }

    #[test]
    fn drive_roots() {
        assert_eq!(drive_root_of(r"C:\").as_deref(), Some(r"C:\"));
        assert_eq!(drive_root_of("d:").as_deref(), Some(r"D:\"));
        assert_eq!(drive_root_of(r"D:\Projects"), None);
        assert_eq!(drive_root_of(r"\\server\share"), None);
        assert_eq!(drive_root_of(""), None);
    }

    #[test]
    fn same_paths() {
        assert!(same_path(r"D:\", "d:"));
        assert!(same_path(r"D:\Projects", r"d:\projects\"));
        assert!(!same_path(r"D:\", r"C:\"));
        assert!(!same_path(r"D:\Projects", r"D:\"));
    }

    #[test]
    fn quoting() {
        assert_eq!(quote_arg(r"C:\"), r"C:\");
        assert_eq!(quote_arg(r"D:\My Files"), r#""D:\My Files""#);
        assert_eq!(quote_arg(r"D:\My Files\"), r#""D:\My Files\\""#);
    }
}
