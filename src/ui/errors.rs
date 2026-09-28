//! "Access errors" window: the folders the last scan could not read, opened
//! from the link in the status bar.

use egui::{Align, Layout, ScrollArea, Ui};

use crate::format::thousands;
use crate::scan::walk::{MAX_KEPT_ERRORS, ScanError};

const ROW_HEIGHT: f32 = 20.0;

#[derive(Default)]
pub struct ErrorsView {
    pub open: bool,
    /// Set when the window opens: place it in the middle of the app window
    /// instead of where egui remembers it.
    center: bool,
    /// All errors of the scan; `list` holds at most `MAX_KEPT_ERRORS`.
    count: u64,
    /// Sorted by path.
    list: Vec<ScanError>,
}

impl ErrorsView {
    /// Errors of a finished scan (a new scan closes the window).
    pub fn set(&mut self, count: u64, mut list: Vec<ScanError>) {
        list.sort_by(|a, b| a.path.cmp(&b.path));
        *self = Self {
            open: false,
            center: false,
            count,
            list,
        };
    }

    /// Link in the status bar that opens the window, if there were errors.
    pub fn link(&mut self, ui: &mut Ui) {
        let text = match self.count {
            0 => return,
            1 => "1 access error".to_string(),
            n => format!("{} access errors", thousands(n)),
        };
        if ui
            .link(text)
            .on_hover_text("Folders that could not be read")
            .clicked()
        {
            self.open = true;
            self.center = true;
        }
    }

    /// The window; `elevated` says whether a hint about Fast scan helps.
    pub fn show(&mut self, ctx: &egui::Context, elevated: bool) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        let center = ctx.content_rect().center();
        // The title carries the count; a fixed id keeps it one window.
        let mut window = egui::Window::new(format!("Access errors ({})", thousands(self.count)))
            .id(egui::Id::new("access_errors"))
            .open(&mut open)
            .default_size([640.0, 420.0])
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(center)
            .collapsible(false);
        // egui keeps the position of a closed window (across restarts too),
        // so every opening moves it back to the middle; it stays movable.
        if std::mem::take(&mut self.center) {
            window = window.current_pos(center);
        }
        window
            .show(ctx, |ui| {
                ui.label(match self.count {
                    1 => "1 folder could not be read, so its contents are missing from the chart."
                        .to_string(),
                    n => format!(
                        "{} folders could not be read, so their contents are missing from the chart.",
                        thousands(n)
                    ),
                });
                if !elevated {
                    ui.weak("A Fast scan (as administrator, through the MFT) reads most of them.");
                }
                ui.horizontal(|ui| {
                    if self.count > self.list.len() as u64 {
                        ui.weak(format!(
                            "Only the first {} are listed.",
                            thousands(MAX_KEPT_ERRORS as u64)
                        ));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Copy list").clicked() {
                            let text: String = self
                                .list
                                .iter()
                                .map(|e| format!("{}\t{}\n", e.path, e.message))
                                .collect();
                            ui.ctx().copy_text(text);
                        }
                    });
                });
                ui.separator();
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show_rows(ui, ROW_HEIGHT, self.list.len(), |ui, range| {
                        for e in &self.list[range] {
                            row(ui, e);
                        }
                    });
            });
        self.open = open;
    }
}

/// One error: the folder's path, and the reason on the right.
fn row(ui: &mut Ui, e: &ScanError) {
    ui.horizontal(|ui| {
        ui.set_min_height(ROW_HEIGHT);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.weak(&e.message);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                // A truncated label shows the full path on hover by itself.
                let path = ui.add(egui::Label::new(&e.path).truncate().sense(egui::Sense::click()));
                path.context_menu(|ui| {
                    if ui.button("Show in Explorer").clicked() {
                        crate::scan::win::open_in_explorer(&e.path, false);
                        ui.close();
                    }
                    if ui.button("Copy path").clicked() {
                        ui.ctx().copy_text(e.path.clone());
                        ui.close();
                    }
                });
            });
        });
    });
}
