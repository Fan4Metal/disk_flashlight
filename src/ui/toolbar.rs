//! Top toolbar and bottom status bar, implemented as methods on `App`.

use std::path::PathBuf;

use egui::{ThemePreference, Ui};

use crate::app::App;
use crate::format::human_size;
use crate::i18n::{count, files, folders};
use crate::layout;
use crate::model::Metric;
use crate::render::ColorMode;

impl App {
    pub fn toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            // Drive picker.
            // A selected drive whose query has not returned yet shows its root.
            let selected_text = match &self.drive {
                Some(root) => self
                    .drives
                    .iter()
                    .find(|d| &d.root == root)
                    .map_or_else(|| root.clone(), |d| d.display()),
                None => "—".into(),
            };
            let mut pick: Option<String> = None;
            let mut pick_folder: Option<String> = None;
            egui::ComboBox::from_id_salt("drive")
                .selected_text(selected_text)
                .width(160.0)
                .show_ui(ui, |ui| {
                    for d in &self.drives {
                        let (free, total) = (human_size(d.free), human_size(d.total));
                        let text = tr!(
                            format!("{}  {free} free of {total}", d.display()),
                            format!("{}  свободно {free} из {total}", d.display())
                        );
                        let selected = self.drive.as_ref() == Some(&d.root);
                        if ui.selectable_label(selected, text).clicked() {
                            pick = Some(d.root.clone());
                        }
                    }
                    if !self.recent.is_empty() {
                        ui.separator();
                        ui.weak(tr!("Recent folders", "Недавние папки"));
                        let current = self.model.as_ref().map(|m| m.root_path.as_str());
                        for path in &self.recent {
                            let selected = current == Some(path.as_str());
                            if ui
                                .selectable_label(selected, elide_middle(path, RECENT_CHARS))
                                .on_hover_text(path)
                                .clicked()
                            {
                                pick_folder = Some(path.clone());
                            }
                        }
                        if ui
                            .selectable_label(false, egui::RichText::new(tr!("Clear recent", "Очистить список")).weak())
                            .on_hover_text(tr!("Forget the recent folders", "Забыть недавние папки"))
                            .clicked()
                        {
                            self.recent.clear();
                        }
                    }
                    ui.separator();
                    if ui
                        .selectable_label(false, tr!("Choose folder…", "Выбрать папку…"))
                        .on_hover_text(tr!("Pick any folder to scan", "Выбрать любую папку для сканирования"))
                        .clicked()
                    {
                        // Opened in `App::ui`, which has the window handle.
                        self.choose_folder = true;
                    }
                });
            if let Some(root) = pick {
                self.start_scan(PathBuf::from(&root));
                self.drive = Some(root);
            }
            // As if typed in the path field: a folder that is gone is
            // reported and the results on screen stay.
            if let Some(path) = pick_folder {
                self.custom_path = path.clone();
                self.start_scan(PathBuf::from(path));
            }

            let scanning = self.scan.is_some();
            if scanning {
                if ui.button(tr!("Cancel", "Отмена")).clicked()
                    && let Some(h) = &self.scan {
                        h.cancel();
                    }
            } else if ui
                .add_enabled(
                    self.model.is_some() || self.drive.is_some(),
                    egui::Button::new(tr!("Rescan", "Обновить")),
                )
                .on_hover_text(tr!(
                    "Scan the current drive or folder again (F5)",
                    "Просканировать текущий диск или папку заново (F5)"
                ))
                .clicked()
            {
                self.rescan();
            }

            if !self.elevated && self.fast_scan_available() {
                let icon = ui.id().with("uac_shield");
                let resp = egui::Button::new((
                    egui::Atom::custom(icon, egui::vec2(11.0, 13.0)),
                    tr!("Fast scan", "Быстрый скан"),
                ))
                .atom_ui(ui);
                if let Some(rect) = resp.rect(icon) {
                    paint_uac_shield(ui.painter(), rect);
                }
                if resp
                    .response
                    .clone()
                    .on_hover_text(tr!(
                        "Restarts as administrator and scans the whole drive \
                         by reading the NTFS MFT directly",
                        "Перезапускает программу от имени администратора и сканирует \
                         весь диск, читая NTFS MFT напрямую"
                    ))
                    .clicked()
                {
                    self.relaunch_as_admin(ui.ctx());
                }
            }

            ui.separator();

            let has_model = self.model.is_some();
            ui.add_enabled_ui(has_model && self.nav.can_back(), |ui| {
                if ui.button("◀").on_hover_text(tr!("Back (Alt+Left)", "Назад (Alt+Влево)")).clicked() {
                    self.go_back();
                }
            });
            ui.add_enabled_ui(has_model && self.nav.can_forward(), |ui| {
                if ui.button("▶").on_hover_text(tr!("Forward (Alt+Right)", "Вперёд (Alt+Вправо)")).clicked() {
                    self.go_forward();
                }
            });
            ui.add_enabled_ui(has_model && self.nav.root != 0, |ui| {
                if ui
                    .button(tr!("Up", "Вверх"))
                    .on_hover_text(tr!("Up (Backspace)", "На уровень выше (Backspace)"))
                    .clicked()
                {
                    self.go_up();
                }
            });

            ui.separator();

            let mut metric = self.metric;
            let metric_label = |m: Metric| match m {
                Metric::Physical => tr!("Physical size", "Размер на диске"),
                Metric::Logical => tr!("Logical size", "Логический размер"),
            };
            egui::ComboBox::from_id_salt("metric")
                .selected_text(metric_label(metric))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut metric, Metric::Physical, metric_label(Metric::Physical))
                        .on_hover_text(tr!(
                            "Space taken on disk, in whole clusters",
                            "Место, занятое на диске, в целых кластерах"
                        ));
                    ui.selectable_value(&mut metric, Metric::Logical, metric_label(Metric::Logical))
                        .on_hover_text(tr!("Size of the data in the files", "Размер данных в файлах"));
                });
            if metric != self.metric {
                self.metric = metric;
                self.chart.invalidate();
            }

            let mut mode = self.chart.palette.mode;
            let label = |m: ColorMode| match m {
                ColorMode::Size => tr!("Colors: by size", "Цвета: по размеру"),
                ColorMode::Depth => tr!("Colors: by level", "Цвета: по уровню"),
                ColorMode::Age => tr!("Colors: by age", "Цвета: по возрасту"),
                ColorMode::Type => tr!("Colors: by type", "Цвета: по типу"),
            };
            egui::ComboBox::from_id_salt("color_mode")
                .selected_text(label(mode))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut mode, ColorMode::Size, label(ColorMode::Size))
                        .on_hover_text(tr!(
                            "Largest item among its siblings in red, smaller ones towards yellow; paler further out",
                            "Самый большой среди соседей элемент красный, меньшие ближе к жёлтому; к краю бледнее"
                        ));
                    ui.selectable_value(&mut mode, ColorMode::Depth, label(ColorMode::Depth))
                        .on_hover_text(tr!(
                            "Colour by ring: red in the centre towards yellow at the rim",
                            "Цвет по кольцу: от красного в центре к жёлтому по краю"
                        ));
                    ui.selectable_value(&mut mode, ColorMode::Type, label(ColorMode::Type))
                        .on_hover_text(tr!(
                            "Colour files by type: the largest types of the scan get a colour each, \
                             the rest one neutral colour; folders are grey",
                            "Цвет файлов по типу: самые объёмные типы скана получают свой цвет, \
                             остальные — один нейтральный; папки серые"
                        ));
                    ui.selectable_value(&mut mode, ColorMode::Age, label(ColorMode::Age))
                        .on_hover_text(tr!(
                            "Colour by the last change: red for recent, through yellow and green, \
                             to blue for ten years or more; a folder by the newest item inside",
                            "Цвет по последнему изменению: от красного для недавних через жёлтый \
                             и зелёный к синему для десяти лет и старше; папка — по самому новому \
                             элементу внутри"
                        ));
                });
            if mode != self.chart.palette.mode {
                self.chart.palette.mode = mode;
                self.chart.invalidate();
            }

            let mut rings = self.chart.params.max_depth;
            egui::ComboBox::from_id_salt("rings")
                .selected_text(tr!(format!("Rings: {rings}"), format!("Колец: {rings}")))
                .show_ui(ui, |ui| {
                    for n in layout::MIN_RINGS..=layout::MAX_RINGS {
                        ui.selectable_value(&mut rings, n, n.to_string());
                    }
                })
                .response
                .on_hover_text(tr!(
                    "How many levels of folders the chart shows around the centre (+ and − keys)",
                    "Сколько уровней папок диаграмма показывает вокруг центра (клавиши + и −)"
                ));
            self.set_rings(rings);

            ui.checkbox(&mut self.tree.follow_hover, tr!("Follow in tree", "Следить в дереве"))
                .on_hover_text(tr!(
                    "Expand the directory tree to the item under the mouse in the chart",
                    "Раскрывать дерево папок до элемента под курсором на диаграмме"
                ));

            ui.separator();

            // About at the right end; the path takes the space left over.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(tr!("About", "О программе"))
                    .on_hover_text(tr!("About Disk Flashlight (F1)", "О программе Disk Flashlight (F1)"))
                    .clicked()
                {
                    self.about.open = true;
                }
                if theme_button(ui, self.theme).clicked() {
                    self.theme = match self.theme {
                        ThemePreference::System => ThemePreference::Light,
                        ThemePreference::Light => ThemePreference::Dark,
                        ThemePreference::Dark => ThemePreference::System,
                    };
                    ui.ctx().set_theme(self.theme);
                }
                ui.separator();
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    self.path_field(ui);
                });
            });
        });
    }

    /// The current path, or a path to type and scan (`ui::path_field`).
    fn path_field(&mut self, ui: &mut Ui) {
        let current = self.model.as_ref().map(|m| m.path(self.nav.root));
        let drives = &self.drives;
        let go = self.path_edit.show(ui, &mut self.custom_path, current, drives, |p| {
            crate::app::is_remote(drives, p)
        });
        if let Some(path) = go {
            self.start_scan(path);
        }
    }

    pub fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if let Some(h) = &self.scan {
                let (n_files, n_dirs, bytes, errors) = h.progress.snapshot();
                ui.add(egui::Spinner::new().size(14.0));
                let secs = h.started.elapsed().as_secs_f32();
                let found = format!("{}, {}, {}", files(n_files), folders(n_dirs), human_size(bytes));
                let text = tr!(
                    format!("Scanning {}  —  {found}  ({secs:.1} s)", h.path.display()),
                    format!(
                        "Сканирование {}  —  {found}  ({} с)",
                        h.path.display(),
                        format!("{secs:.1}").replace('.', ",")
                    )
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if errors > 0 {
                        ui.weak(count(
                            errors,
                            ["access error", "access errors"],
                            ["ошибка доступа", "ошибки доступа", "ошибок доступа"],
                        ));
                    }
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(text).truncate());
                    });
                });
                return;
            }
            let Some(model) = &self.model else {
                ui.add(egui::Label::new(&self.status).truncate());
                return;
            };
            let root = model.node(self.nav.root);
            let (dirs, files) = (
                crate::format::thousands(root.dirs as u64),
                crate::format::thousands(root.files as u64),
            );
            let (size, alloc) = (human_size(root.size), human_size(root.alloc));
            let waste = human_size(root.alloc.saturating_sub(root.size));
            ui.label(tr!(
                format!("Dirs: {dirs}   Files: {files}   Size: {size}   Alloc: {alloc}   Waste: {waste}"),
                format!("Папок: {dirs}   Файлов: {files}   Размер: {size}   На диске: {alloc}   Потери: {waste}")
            ));
            let hovered = self.chart.hovered.or(self.tree_hovered);
            // The errors link stays at the right end; the hovered item and
            // then the status message take what is left, cut short with an
            // ellipsis rather than running over the counts.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.errors.link(ui);
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    if let Some(h) = hovered {
                        let n = model.node(h);
                        ui.separator();
                        ui.add(
                            egui::Label::new(format!(
                                "{}  —  {} ({})",
                                model.name(h),
                                human_size(n.size),
                                human_size(n.alloc)
                            ))
                            .truncate(),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(&self.status).weak()).truncate());
                    });
                });
            });
        });
    }
}

/// Longest recent folder shown in full in the drive picker, in characters.
const RECENT_CHARS: usize = 60;

/// `s` with its middle replaced by `…` if it is longer than `max`
/// characters, so that both the drive and the folder name stay visible.
fn elide_middle(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let head = (max - 1) / 2;
    let tail = max - 1 - head;
    let start: String = s.chars().take(head).collect();
    let end: String = s.chars().skip(n - tail).collect();
    format!("{start}…{end}")
}

/// The theme switch: a half-filled circle while following Windows, a sun
/// for the light theme, a moon for the dark one; a click moves on to the
/// next of the three.
fn theme_button(ui: &mut Ui, theme: ThemePreference) -> egui::Response {
    use egui::{Shape, Stroke, pos2, vec2};
    let icon = ui.id().with("theme_icon");
    let resp = egui::Button::new(egui::Atom::custom(icon, vec2(14.0, 14.0))).atom_ui(ui);
    let tip = match theme {
        ThemePreference::System => tr!(
            "Theme: as in Windows (click: light)",
            "Тема: как в Windows (щелчок: светлая)"
        ),
        ThemePreference::Light => tr!("Theme: light (click: dark)", "Тема: светлая (щелчок: тёмная)"),
        ThemePreference::Dark => tr!(
            "Theme: dark (click: as in Windows)",
            "Тема: тёмная (щелчок: как в Windows)"
        ),
    };
    let response = resp.response.clone().on_hover_text(tip);
    let Some(rect) = resp.rect(icon) else { return response };
    let visuals = ui.style().interact(&response);
    let (color, fill) = (visuals.text_color(), visuals.weak_bg_fill);
    let painter = ui.painter();
    let c = rect.center();
    match theme {
        ThemePreference::System => {
            let r = 5.5;
            painter.circle_stroke(c, r, Stroke::new(1.3, color));
            // Left half filled: a convex half disc.
            let half: Vec<_> = (0..=16)
                .map(|i| {
                    let a = std::f32::consts::PI * (0.5 + i as f32 / 16.0);
                    c + vec2(r * a.cos(), -r * a.sin())
                })
                .collect();
            painter.add(Shape::convex_polygon(half, color, Stroke::NONE));
        }
        ThemePreference::Light => {
            painter.circle_filled(c, 3.2, color);
            for i in 0..8 {
                let a = std::f32::consts::TAU * i as f32 / 8.0;
                let d = vec2(a.cos(), a.sin());
                painter.line_segment([c + d * 5.0, c + d * 6.8], Stroke::new(1.3, color));
            }
        }
        ThemePreference::Dark => {
            // A crescent: a disc with another in the button's colour over
            // its upper right.
            painter.circle_filled(c, 6.0, color);
            painter.circle_filled(pos2(c.x + 3.0, c.y - 2.2), 5.0, fill);
        }
    }
    response
}

/// The Windows UAC shield (blue and yellow quarters), drawn into `rect` to
/// mark an action that asks for administrator rights.
fn paint_uac_shield(painter: &egui::Painter, rect: egui::Rect) {
    use egui::{Color32, Shape, Stroke, pos2};
    let (l, r, t, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
    let c = rect.center();
    // Convex pentagon: flat top, straight sides, pointed bottom.
    let shield = vec![
        pos2(l, t),
        pos2(r, t),
        pos2(r, t + rect.height() * 0.55),
        pos2(c.x, b),
        pos2(l, t + rect.height() * 0.55),
    ];
    let blue = Color32::from_rgb(38, 98, 196);
    let yellow = Color32::from_rgb(246, 196, 44);
    painter.add(Shape::convex_polygon(shield.clone(), blue, Stroke::NONE));
    // Yellow top-right and bottom-left quarters, clipped from the same shape.
    for quarter in [
        egui::Rect::from_min_max(pos2(c.x, t), pos2(r, c.y)),
        egui::Rect::from_min_max(pos2(l, c.y), pos2(c.x, b)),
    ] {
        painter
            .with_clip_rect(quarter)
            .add(Shape::convex_polygon(shield.clone(), yellow, Stroke::NONE));
    }
    painter.add(Shape::closed_line(
        shield,
        Stroke::new(1.0, Color32::from_rgb(20, 50, 110)),
    ));
}

#[cfg(test)]
mod tests {
    use super::elide_middle;

    #[test]
    fn elides_the_middle() {
        assert_eq!(elide_middle(r"D:\Projects", 20), r"D:\Projects");
        let long = r"D:\Projects\video_converter\samples\клипы";
        let short = elide_middle(long, 21);
        assert_eq!(short.chars().count(), 21);
        assert_eq!(short, r"D:\Project…ples\клипы");
    }
}
