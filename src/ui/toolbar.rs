//! Top toolbar and bottom status bar, implemented as methods on `App`.

use std::path::PathBuf;

use egui::{ThemePreference, Ui};

use crate::app::App;
use crate::format::{human_size, thousands};
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
                        let text = format!(
                            "{}  {} free of {}",
                            d.display(),
                            human_size(d.free),
                            human_size(d.total)
                        );
                        let selected = self.drive.as_ref() == Some(&d.root);
                        if ui.selectable_label(selected, text).clicked() {
                            pick = Some(d.root.clone());
                        }
                    }
                    if !self.recent.is_empty() {
                        ui.separator();
                        ui.weak("Recent folders");
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
                            .selectable_label(false, egui::RichText::new("Clear recent").weak())
                            .on_hover_text("Forget the recent folders")
                            .clicked()
                        {
                            self.recent.clear();
                        }
                    }
                    ui.separator();
                    if ui
                        .selectable_label(false, "Choose folder…")
                        .on_hover_text("Pick any folder to scan")
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
                if ui.button("Cancel").clicked()
                    && let Some(h) = &self.scan {
                        h.cancel();
                    }
            } else if ui
                .add_enabled(
                    self.model.is_some() || self.drive.is_some(),
                    egui::Button::new("Rescan"),
                )
                .on_hover_text("Scan the current drive or folder again (F5)")
                .clicked()
            {
                self.rescan();
            }

            if !self.elevated && self.fast_scan_available() {
                let icon = ui.id().with("uac_shield");
                let resp = egui::Button::new((
                    egui::Atom::custom(icon, egui::vec2(11.0, 13.0)),
                    "Fast scan",
                ))
                .atom_ui(ui);
                if let Some(rect) = resp.rect(icon) {
                    paint_uac_shield(ui.painter(), rect);
                }
                if resp
                    .response
                    .clone()
                    .on_hover_text(
                        "Restarts as administrator and scans the whole drive \
                         by reading the NTFS MFT directly",
                    )
                    .clicked()
                {
                    self.relaunch_as_admin(ui.ctx());
                }
            }

            ui.separator();

            let has_model = self.model.is_some();
            ui.add_enabled_ui(has_model && self.nav.can_back(), |ui| {
                if ui.button("◀").on_hover_text("Back (Alt+Left)").clicked() {
                    self.go_back();
                }
            });
            ui.add_enabled_ui(has_model && self.nav.can_forward(), |ui| {
                if ui.button("▶").on_hover_text("Forward (Alt+Right)").clicked() {
                    self.go_forward();
                }
            });
            ui.add_enabled_ui(has_model && self.nav.root != 0, |ui| {
                if ui.button("Up").on_hover_text("Up (Backspace)").clicked() {
                    self.go_up();
                }
            });

            ui.separator();

            let mut metric = self.metric;
            egui::ComboBox::from_id_salt("metric")
                .selected_text(match metric {
                    Metric::Physical => "Physical size",
                    Metric::Logical => "Logical size",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut metric, Metric::Physical, "Physical size");
                    ui.selectable_value(&mut metric, Metric::Logical, "Logical size");
                });
            if metric != self.metric {
                self.metric = metric;
                self.chart.invalidate();
            }

            let mut mode = self.chart.palette.mode;
            let label = |m: ColorMode| match m {
                ColorMode::Size => "Colors: by size",
                ColorMode::Depth => "Colors: by level",
                ColorMode::Age => "Colors: by age",
            };
            egui::ComboBox::from_id_salt("color_mode")
                .selected_text(label(mode))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut mode, ColorMode::Size, label(ColorMode::Size))
                        .on_hover_text("Largest item among its siblings in red, smaller ones towards yellow; paler further out");
                    ui.selectable_value(&mut mode, ColorMode::Depth, label(ColorMode::Depth))
                        .on_hover_text("Colour by ring: red in the centre towards yellow at the rim");
                    ui.selectable_value(&mut mode, ColorMode::Age, label(ColorMode::Age))
                        .on_hover_text(
                            "Colour by the last change: red for recent, through yellow and green, \
                             to blue for ten years or more; a folder by the newest item inside",
                        );
                });
            if mode != self.chart.palette.mode {
                self.chart.palette.mode = mode;
                self.chart.invalidate();
            }

            ui.checkbox(&mut self.tree.follow_hover, "Follow in tree")
                .on_hover_text("Expand the directory tree to the item under the mouse in the chart");

            ui.separator();

            // About at the right end; the path takes the space left over.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("About").on_hover_text("About Disk Flashlight (F1)").clicked() {
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
                let (files, dirs, bytes, errors) = h.progress.snapshot();
                ui.add(egui::Spinner::new().size(14.0));
                ui.label(format!(
                    "Scanning {}  —  {} files, {} dirs, {}  ({:.1}s)",
                    h.path.display(),
                    thousands(files),
                    thousands(dirs),
                    human_size(bytes),
                    h.started.elapsed().as_secs_f32()
                ));
                if errors > 0 {
                    ui.weak(format!("{} access errors", thousands(errors)));
                }
                return;
            }
            let Some(model) = &self.model else {
                ui.label(&self.status);
                return;
            };
            let root = model.node(self.nav.root);
            ui.label(format!(
                "Dirs: {}   Files: {}   Size: {}   Alloc: {}   Waste: {}",
                thousands(root.dirs as u64),
                thousands(root.files as u64),
                human_size(root.size),
                human_size(root.alloc),
                human_size(root.alloc.saturating_sub(root.size)),
            ));
            if let Some(h) = self.chart.hovered.or(self.tree_hovered) {
                let n = model.node(h);
                ui.separator();
                ui.label(format!(
                    "{}  —  {} ({})",
                    model.name(h),
                    human_size(n.size),
                    human_size(n.alloc)
                ));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.errors.link(ui);
                ui.weak(&self.status);
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
        ThemePreference::System => "Theme: as in Windows (click: light)",
        ThemePreference::Light => "Theme: light (click: dark)",
        ThemePreference::Dark => "Theme: dark (click: as in Windows)",
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
