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
    /// Expand the tree to the item hovered in the chart.
    pub follow_hover: bool,
    expanded: HashSet<u32>,
    /// (node, depth) for every visible row.
    rows: Vec<(u32, u16)>,
    dirty: bool,
    /// Metric the rows were last sorted by.
    sorted_by: Option<Metric>,
    /// Scroll to the row of this node on the next frame.
    scroll_to: Option<u32>,
    /// Scroll the row of this node into view on the next frame, only if it
    /// is outside the visible part.
    ensure_visible: Option<u32>,
    /// Directory last hovered in the chart; its collapsed ancestors are in
    /// `peek`, expanded temporarily until the next hovered directory.
    peek_target: Option<u32>,
    peek: Vec<u32>,
    /// Scroll offset and height of the list during the last frame.
    viewport: (f32, f32),
}

impl Default for TreeView {
    fn default() -> Self {
        let mut t = Self {
            follow_hover: true,
            expanded: HashSet::new(),
            rows: Vec::new(),
            dirty: true,
            sorted_by: None,
            scroll_to: None,
            ensure_visible: None,
            peek_target: None,
            peek: Vec::new(),
            viewport: (0.0, 0.0),
        };
        t.expanded.insert(0);
        t
    }
}

impl TreeView {
    /// Forget expansion state (new model).
    pub fn reset(&mut self) {
        *self = Self {
            follow_hover: self.follow_hover,
            ..Self::default()
        };
    }

    /// Show `id`, the new centre of the chart: only it and its ancestors stay
    /// expanded, everything else collapses (so expansions do not pile up while
    /// navigating), and the tree scrolls to it.
    pub fn reveal(&mut self, model: &Model, id: u32) {
        self.expanded.clear();
        let mut cur = id;
        while cur != NO_NODE {
            self.expanded.insert(cur);
            cur = model.node(cur).parent;
        }
        self.peek.clear();
        self.peek_target = None;
        self.dirty = true;
        self.scroll_to = Some(id);
    }

    /// Temporarily expand the ancestors of `id` (replacing the previous
    /// peek) and scroll it into view.
    fn peek_at(&mut self, model: &Model, id: u32) {
        self.peek_target = Some(id);
        self.peek.clear();
        let mut cur = model.node(id).parent;
        while cur != NO_NODE {
            if !self.expanded.contains(&cur) {
                self.peek.push(cur);
            }
            cur = model.node(cur).parent;
        }
        self.dirty = true;
        self.ensure_visible = Some(id);
    }

    fn is_expanded(&self, id: u32) -> bool {
        self.expanded.contains(&id) || self.peek.contains(&id)
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
            if self.is_expanded(id) {
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
        // The tree lists directories only: a hovered file shows its folder.
        let hovered_dir = chart_hovered.map(|id| {
            let n = model.node(id);
            if n.is_dir { id } else { n.parent }
        });
        if !self.follow_hover {
            if self.peek_target.take().is_some() {
                self.peek.clear();
                self.dirty = true;
            }
        } else if let Some(d) = hovered_dir
            && self.peek_target != Some(d)
        {
            self.peek_at(model, d);
        }
        if self.dirty || self.sorted_by != Some(metric) {
            self.rebuild(model, metric);
        }
        let mut action = TreeAction::default();
        let mut toggled: Option<u32> = None;
        // show_rows places rows one row height plus item spacing apart.
        let pitch = ROW_HEIGHT + ui.spacing().item_spacing.y;
        let row_y = |id: u32| {
            self.rows
                .iter()
                .position(|&(n, _)| n == id)
                .map(|row| row as f32 * pitch)
        };
        let (top, height) = self.viewport;
        let offset = if let Some(y) = self.scroll_to.take().and_then(row_y) {
            Some((y - pitch * 4.0).max(0.0))
        } else if let Some(y) = self.ensure_visible.take().and_then(row_y) {
            if y < top {
                Some(y)
            } else if y + pitch > top + height {
                Some(y + pitch - height)
            } else {
                None
            }
        } else {
            None
        };

        let mut area = ScrollArea::both().auto_shrink([false, false]);
        if let Some(y) = offset {
            area = area.vertical_scroll_offset(y);
        }
        let out = area.show_rows(ui, ROW_HEIGHT, self.rows.len(), |ui, range| {
            for i in range {
                let (id, depth) = self.rows[i];
                let node = model.node(id);
                let has_subdirs = model.children(id).any(|c| model.node(c).is_dir);
                let is_expanded = self.is_expanded(id);
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
                    let selected = id == current_root || Some(id) == hovered_dir;
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
        self.viewport = (out.state.offset.y, out.inner_rect.height());

        if let Some(id) = toggled {
            if let Some(i) = self.peek.iter().position(|&p| p == id) {
                self.peek.swap_remove(i);
            } else if !self.expanded.remove(&id) {
                self.expanded.insert(id);
            }
            self.dirty = true;
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RawDir;

    fn dir(name: &str, size: u64, subdirs: Vec<RawDir>) -> RawDir {
        // Sizes set the order: a directory's own `size` is recomputed from
        // files, so give each one a file of that size.
        RawDir {
            name: name.into(),
            files: vec![crate::model::RawFile { name: "f".into(), size, alloc: size }],
            subdirs,
            ..Default::default()
        }
    }

    fn rows(t: &mut TreeView, m: &Model) -> Vec<String> {
        t.rebuild(m, Metric::Logical);
        t.rows.iter().map(|&(id, _)| m.name(id).to_string()).collect()
    }

    #[test]
    fn navigation_collapses_everything_off_the_path() {
        let raw = dir(
            "root",
            1,
            vec![
                dir("a", 30, vec![dir("a1", 20, vec![dir("deep", 1, vec![])]), dir("a2", 10, vec![])]),
                dir("b", 5, vec![dir("b1", 1, vec![])]),
            ],
        );
        let m = Model::from_raw(raw, "X:\\".into(), 1);
        let id = |path: &[&str]| m.find_dir(path);
        let mut t = TreeView::default();
        assert_eq!(rows(&mut t, &m), ["root", "a", "b"]);

        // b is expanded by hand, then the chart moves into a1 (inside a).
        t.expanded.insert(id(&["b"]));
        assert_eq!(rows(&mut t, &m), ["root", "a", "b", "b1"]);
        t.reveal(&m, id(&["a", "a1"]));
        assert_eq!(rows(&mut t, &m), ["root", "a", "a1", "deep", "a2", "b"]);

        // Up to a: a1 collapses again.
        t.reveal(&m, id(&["a"]));
        assert_eq!(rows(&mut t, &m), ["root", "a", "a1", "a2", "b"]);
    }
}
