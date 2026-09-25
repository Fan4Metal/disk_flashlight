//! Top toolbar and bottom status bar, implemented as methods on `App`.

use std::path::PathBuf;

use egui::Ui;

use crate::app::App;
use crate::format::{human_size, thousands};
use crate::model::Metric;

impl App {
    pub fn toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            // Drive picker.
            let selected_text = self
                .drives
                .get(self.drive_idx)
                .map(|d| d.display())
                .unwrap_or_else(|| "—".into());
            let mut pick: Option<usize> = None;
            egui::ComboBox::from_id_salt("drive")
                .selected_text(selected_text)
                .width(160.0)
                .show_ui(ui, |ui| {
                    for (i, d) in self.drives.iter().enumerate() {
                        let text = format!(
                            "{}  {} free of {}",
                            d.display(),
                            human_size(d.free),
                            human_size(d.total)
                        );
                        if ui.selectable_label(i == self.drive_idx, text).clicked() {
                            pick = Some(i);
                        }
                    }
                });
            if let Some(i) = pick {
                self.drive_idx = i;
                let root = PathBuf::from(&self.drives[i].root);
                self.start_scan(root);
            }

            let scanning = self.scan.is_some();
            if scanning {
                if ui.button("Cancel").clicked()
                    && let Some(h) = &self.scan {
                        h.cancel();
                    }
            } else if ui.button("Rescan").clicked() {
                self.rescan();
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

            ui.separator();

            // Current path (read-only) or a free-form path to scan.
            if let Some(m) = &self.model {
                let path = m.path(self.nav.root);
                ui.add(
                    egui::Label::new(egui::RichText::new(path).monospace()).truncate(),
                );
            } else {
                ui.label("Select a drive to scan, or enter a path:");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.custom_path)
                        .desired_width(260.0)
                        .hint_text(r"D:\Projects"),
                );
                let go = ui.button("Scan").clicked()
                    || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                if go && !self.custom_path.trim().is_empty() {
                    self.start_scan(PathBuf::from(self.custom_path.trim()));
                }
            }
        });
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
                ui.weak(&self.status);
            });
        });
    }
}
