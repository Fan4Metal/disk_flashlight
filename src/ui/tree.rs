//! Left-hand directory tree, virtualised (only visible rows are laid out).

use std::collections::HashSet;

use egui::{Align, Layout, ScrollArea, Ui};

use crate::format::human_size;
use crate::model::{Metric, Model, NO_NODE};

const ROW_HEIGHT: f32 = 20.0;
const INDENT: f32 = 14.0;
const ARROW_WIDTH: f32 = 16.0;

/// Expand/collapse triangle painted directly (the default fonts lack the
/// ▲/▼ glyphs), returning the click response.
fn arrow(ui: &mut Ui, expanded: bool) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ARROW_WIDTH, ROW_HEIGHT), egui::Sense::click());
    let c = rect.center();
    let s = 4.0;
    let pts = if expanded {
        vec![
            c + egui::vec2(-s, -s * 0.6),
            c + egui::vec2(s, -s * 0.6),
            c + egui::vec2(0.0, s * 0.8),
        ]
    } else {
        vec![
            c + egui::vec2(-s * 0.6, -s),
            c + egui::vec2(s * 0.8, 0.0),
            c + egui::vec2(-s * 0.6, s),
        ]
    };
    let color = if resp.hovered() {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    ui.painter()
        .add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
    resp
}

#[derive(Default)]
pub struct TreeAction {
    pub selected: Option<u32>,
    pub hovered: Option<u32>,
}

pub struct TreeView {
    expanded: HashSet<u32>,
    /// (node, depth) for every visible row.
    rows: Vec<(u32, u16)>,
    dirty: bool,
    /// Metric the rows were last sorted by.
    sorted_by: Option<Metric>,
    /// Scroll to the row of this node on the next frame.
    scroll_to: Option<u32>,
}

impl Default for TreeView {
    fn default() -> Self {
        let mut t = Self {
            expanded: HashSet::new(),
            rows: Vec::new(),
            dirty: true,
            sorted_by: None,
            scroll_to: None,
        };
        t.expanded.insert(0);
        t
    }
}

impl TreeView {
    /// Forget expansion state (new model).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Expand all ancestors of `id` so that it is visible, and scroll to it.
    pub fn reveal(&mut self, model: &Model, id: u32) {
        let mut cur = model.node(id).parent;
        while cur != NO_NODE {
            self.expanded.insert(cur);
            cur = model.node(cur).parent;
        }
        self.dirty = true;
        self.scroll_to = Some(id);
    }

    fn rebuild(&mut self, model: &Model, metric: Metric) {
        self.rows.clear();
        self.sorted_by = Some(metric);
        if model.is_empty() {
            return;
        }
        // Iterative DFS; stack holds (node, depth).
        let mut stack: Vec<(u32, u16)> = vec![(0, 0)];
        while let Some((id, depth)) = stack.pop() {
            self.rows.push((id, depth));
            if self.expanded.contains(&id) {
                // Push in reverse so the largest child is popped first.
                let mut dirs: Vec<u32> = model
                    .children(id)
                    .filter(|&c| model.node(c).is_dir)
                    .collect();
                if metric != Metric::Logical {
                    // Stored order is by logical size; re-sort for the shown metric.
                    dirs.sort_by_key(|&c| std::cmp::Reverse(model.node(c).metric(metric)));
                }
                for &c in dirs.iter().rev() {
                    stack.push((c, depth + 1));
                }
            }
        }
        self.dirty = false;
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        model: &Model,
        current_root: u32,
        metric: Metric,
        chart_hovered: Option<u32>,
    ) -> TreeAction {
        if self.dirty || self.sorted_by != Some(metric) {
            self.rebuild(model, metric);
        }
        let mut action = TreeAction::default();
        let mut toggled: Option<u32> = None;
        let scroll_row = self
            .scroll_to
            .take()
            .and_then(|id| self.rows.iter().position(|&(n, _)| n == id));

        let mut area = ScrollArea::both().auto_shrink([false, false]);
        if let Some(row) = scroll_row {
            let y = row as f32 * ROW_HEIGHT;
            area = area.vertical_scroll_offset((y - ROW_HEIGHT * 4.0).max(0.0));
        }
        area.show_rows(ui, ROW_HEIGHT, self.rows.len(), |ui, range| {
            for i in range {
                let (id, depth) = self.rows[i];
                let node = model.node(id);
                let has_subdirs = model.children(id).any(|c| model.node(c).is_dir);
                let is_expanded = self.expanded.contains(&id);
                ui.horizontal(|ui| {
                    ui.set_min_height(ROW_HEIGHT);
                    ui.add_space(depth as f32 * INDENT);
                    if has_subdirs {
                        if arrow(ui, is_expanded).clicked() {
                            toggled = Some(id);
                        }
                    } else {
                        ui.add_space(ARROW_WIDTH);
                    }
                    let selected = id == current_root || Some(id) == chart_hovered;
                    let label = ui.selectable_label(selected, model.name(id));
                    if label.clicked() {
                        action.selected = Some(id);
                    }
                    if label.double_clicked() && has_subdirs {
                        toggled = Some(id);
                    }
                    if label.hovered() {
                        action.hovered = Some(id);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.weak(human_size(node.metric(metric)));
                    });
                });
            }
        });

        if let Some(id) = toggled {
            if !self.expanded.remove(&id) {
                self.expanded.insert(id);
            }
            self.dirty = true;
        }
        action
    }
}
