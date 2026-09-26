//! "Largest files" list: the biggest files under the chart's current root.

use egui::{Align, Layout, RichText, ScrollArea, Ui};

use crate::format::human_size;
use crate::model::{Metric, Model, NO_NODE};
use crate::ui::tree::TreeAction;

/// Number of files listed.
const LIMIT: usize = 100;
const ROW_HEIGHT: f32 = 20.0;

#[derive(Default)]
pub struct FilesView {
    /// `(root, metric)` the rows were built for.
    key: Option<(u32, Metric)>,
    /// Largest first: file id and its folder relative to the root.
    rows: Vec<(u32, String)>,
    /// File last clicked; stays highlighted after the chart moves into its
    /// folder (it is still in the list there).
    selected: Option<u32>,
    scroll_to_selected: bool,
}

impl FilesView {
    /// Forget everything (new model).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// A click selects the file and asks to navigate to its folder.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        model: &Model,
        root: u32,
        metric: Metric,
        chart_hovered: Option<u32>,
    ) -> TreeAction {
        if self.key != Some((root, metric)) {
            self.key = Some((root, metric));
            self.rows = model
                .largest_files(root, metric, LIMIT)
                .into_iter()
                .map(|id| (id, folder_under(model, root, id)))
                .collect();
            self.scroll_to_selected = self.selected.is_some();
        }

        let mut action = TreeAction::default();
        if self.rows.is_empty() {
            ui.weak("No files here");
            return action;
        }
        let pitch = ROW_HEIGHT + ui.spacing().item_spacing.y;
        let mut area = ScrollArea::vertical().auto_shrink([false, false]);
        if std::mem::take(&mut self.scroll_to_selected)
            && let Some(row) = self.rows.iter().position(|&(id, _)| Some(id) == self.selected)
        {
            area = area.vertical_scroll_offset((row as f32 * pitch - pitch * 4.0).max(0.0));
        }
        area.show_rows(ui, ROW_HEIGHT, self.rows.len(), |ui, range| {
            for (id, folder) in &self.rows[range] {
                let id = *id;
                ui.horizontal(|ui| {
                    ui.set_min_height(ROW_HEIGHT);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.weak(human_size(model.node(id).metric(metric)));
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            let mut text = egui::text::LayoutJob::default();
                            let style = ui.style();
                            RichText::new(model.name(id)).append_to(
                                &mut text,
                                style,
                                egui::FontSelection::Default,
                                Align::Center,
                            );
                            if !folder.is_empty() {
                                RichText::new(format!("   {folder}")).weak().append_to(
                                    &mut text,
                                    style,
                                    egui::FontSelection::Default,
                                    Align::Center,
                                );
                            }
                            let highlighted = Some(id) == self.selected || Some(id) == chart_hovered;
                            let label = ui
                                .add(egui::Button::selectable(highlighted, text).truncate())
                                .on_hover_ui(|ui| {
                                    ui.label(model.path(id));
                                });
                            if label.clicked() {
                                self.selected = Some(id);
                                action.selected = Some(model.node(id).parent);
                            }
                            if label.hovered() {
                                action.hovered = Some(id);
                            }
                        });
                    });
                });
            }
        });
        action
    }
}

/// Folder of `file` relative to `root` (`""` directly under the root).
fn folder_under(model: &Model, root: u32, file: u32) -> String {
    let mut parts = Vec::new();
    let mut cur = model.node(file).parent;
    while cur != root && cur != NO_NODE {
        parts.push(model.name(cur));
        cur = model.node(cur).parent;
    }
    parts.reverse();
    parts.join("\\")
}
