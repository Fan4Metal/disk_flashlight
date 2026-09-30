//! Left-hand directory tree, virtualised (only visible rows are laid out).

use std::collections::HashSet;

use egui::{Align, Layout, ScrollArea, Ui};

use crate::format::human_size;
use crate::model::{Metric, Model, NO_NODE};
use crate::ui::{ItemCommand, item_menu};

const ROW_HEIGHT: f32 = 20.0;
const INDENT: f32 = 14.0;
pub(super) const ARROW_WIDTH: f32 = 16.0;

/// Expand/collapse triangle painted directly (the default fonts lack the
/// ▲/▼ glyphs), returning the click response.
pub(super) fn arrow(ui: &mut Ui, expanded: bool) -> egui::Response {
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
    /// Chosen in an item's context menu.
    pub command: Option<ItemCommand>,
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
    /// `peek`, expanded temporarily until another directory is hovered or
    /// the pointer rests on the chart off the sectors.
    peek_target: Option<u32>,
    peek: Vec<u32>,
    /// Scroll offset and height of the list during the last frame.
    viewport: (f32, f32),
    /// How much wider than the list its rows were in the last frame.
    #[cfg(test)]
    overflow: f32,
    /// The export button was clicked: `App` saves [`Self::rows`].
    pub export: bool,
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
            #[cfg(test)]
            overflow: 0.0,
            export: false,
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

    /// Expand `id` and its ancestors, keeping everything else as it is, and
    /// scroll to it.
    pub fn expand_to(&mut self, model: &Model, id: u32) {
        let mut cur = id;
        while cur != NO_NODE {
            self.expanded.insert(cur);
            cur = model.node(cur).parent;
        }
        self.dirty = true;
        self.scroll_to = Some(id);
    }

    /// Collapse everything but the scan root, the peek included (the next
    /// folder hovered in the chart is peeked at again), and scroll to the
    /// top.
    fn collapse_all(&mut self) {
        self.expanded.clear();
        self.expanded.insert(0);
        self.peek.clear();
        self.peek_target = None;
        self.dirty = true;
        self.scroll_to = Some(0);
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

    /// Follow the chart: peek at the directory of the hovered item; with
    /// the pointer on the chart but on no sector (empty space, the centre),
    /// drop the peek as if the centre were hovered. Off the chart the peek
    /// stays, so that the pointer can move into the tree to use it.
    fn follow(&mut self, model: &Model, current_root: u32, hovered_dir: Option<u32>, over_chart: bool) {
        if !self.follow_hover {
            if self.peek_target.take().is_some() {
                self.peek.clear();
                self.dirty = true;
            }
        } else if let Some(d) = hovered_dir {
            if self.peek_target != Some(d) {
                self.peek_at(model, d);
            }
        } else if over_chart && self.peek_target.take().is_some() {
            self.peek.clear();
            self.dirty = true;
            self.ensure_visible = Some(current_root);
        }
    }

    /// The rows shown, `(folder, depth)`: what is expanded, the folders
    /// peeked at from the chart included.
    pub fn rows(&self) -> &[(u32, u16)] {
        &self.rows
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
        over_chart: bool,
    ) -> TreeAction {
        // The tree lists directories only: a hovered file shows its folder.
        let hovered_dir = chart_hovered.map(|id| {
            let n = model.node(id);
            if n.is_dir { id } else { n.parent }
        });
        self.follow(model, current_root, hovered_dir, over_chart);
        if self.dirty || self.sorted_by != Some(metric) {
            self.rebuild(model, metric);
        }
        // Collapse all on the left of a row of its own, Export at its right end.
        ui.horizontal(|ui| {
            let can_collapse = self.expanded.len() > 1 || !self.peek.is_empty();
            if ui
                .add_enabled(can_collapse, egui::Button::new(tr!("Collapse all", "Свернуть все")))
                .on_hover_text(tr!(
                    "Collapse every folder but the scanned one",
                    "Свернуть все папки, кроме просканированной"
                ))
                .clicked()
            {
                self.collapse_all();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let tip = tr!(
                    "Save the folders shown, as far as they are expanded, as a CSV file, which Excel opens",
                    "Сохранить показанные папки, насколько они развёрнуты, в файл CSV, который открывается в Excel"
                );
                self.export |= super::files::export_button(ui, !self.rows.is_empty(), tip).clicked();
            });
        });
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
            // The sizes are right-aligned at the edge of the list, and egui
            // rounds them outwards to whole pixels: a row could end up a
            // fraction of a point wider than the list, which showed an
            // empty horizontal scroll bar. A point to spare absorbs that;
            // really long rows (deep folders) still scroll.
            ui.set_max_width(ui.available_width() - 1.0);
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
                    label.context_menu(|ui| {
                        if let Some(c) = item_menu(ui, id) {
                            action.command = Some(c);
                        }
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.weak(human_size(node.metric(metric)));
                    });
                });
            }
        });
        self.viewport = (out.state.offset.y, out.inner_rect.height());
        #[cfg(test)]
        {
            self.overflow = out.content_size.x - out.inner_rect.width();
        }

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
            files: vec![crate::model::RawFile { name: "f".into(), size, alloc: size, modified: 0 }],
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

        // Collapse all leaves the root's folders, peek included.
        t.expanded.insert(id(&["b"]));
        t.follow(&m, 0, Some(id(&["a", "a1", "deep"])), true);
        t.collapse_all();
        assert_eq!(rows(&mut t, &m), ["root", "a", "b"]);
        // The same folder hovered again is peeked at again.
        t.follow(&m, 0, Some(id(&["a", "a1", "deep"])), true);
        assert_eq!(rows(&mut t, &m), ["root", "a", "a1", "deep", "a2", "b"]);
        // Showing a folder expands its path and keeps the rest.
        t.collapse_all();
        t.expanded.insert(id(&["b"]));
        t.expand_to(&m, id(&["a", "a1"]));
        assert_eq!(rows(&mut t, &m), ["root", "a", "a1", "deep", "a2", "b", "b1"]);
    }

    /// In a side panel like the app's, at the usual display scales and at
    /// panel widths a drag can leave, rows never overflow the list, so no
    /// horizontal scroll bar shows up.
    #[test]
    fn rows_fit_the_list_at_any_scale() {
        let names = ["Users", "Program Files (x86)", "Windows", "ProgramData", "$Recycle.Bin", "System Volume Information"];
        let subs = names.iter().enumerate().map(|(i, n)| dir(n, 1000 - i as u64, vec![dir("x", 1, vec![])])).collect();
        let m = Model::from_raw(dir("C:", 1, subs), "C:\\".into(), 1);
        for ppp in [1.0f32, 1.25, 1.5, 1.75, 2.0] {
            for width in [300.0f32, 301.0, 302.5, 333.3, 417.7] {
                let ctx = egui::Context::default();
                ctx.all_styles_mut(|s| s.spacing.scroll = egui::style::ScrollStyle::solid());
                ctx.set_pixels_per_point(ppp);
                let mut t = TreeView::default();
                for _ in 0..3 {
                    let input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))),
                        ..Default::default()
                    };
                    let mut out = ctx.run_ui(input, |ui| {
                        egui::Panel::left("tree").resizable(true).default_size(width).show(ui, |ui| {
                            t.show(ui, &m, 0, Metric::Logical, None, false);
                        });
                    });
                    out.textures_delta.clear();
                }
                assert!(t.overflow <= 0.0, "scale {ppp}, width {width}: rows {} wider", t.overflow);
            }
        }
    }

    #[test]
    fn peek_follows_the_chart_and_folds_off_the_sectors() {
        let raw = dir(
            "root",
            1,
            vec![
                dir("a", 30, vec![dir("a1", 20, vec![dir("deep", 1, vec![])])]),
                dir("b", 5, vec![dir("b1", 1, vec![])]),
            ],
        );
        let m = Model::from_raw(raw, "X:\\".into(), 1);
        let id = |path: &[&str]| m.find_dir(path);
        let mut t = TreeView::default();
        let peeked = ["root", "a", "a1", "deep", "b"];

        // Hovering "deep" opens its ancestors.
        t.follow(&m, 0, Some(id(&["a", "a1", "deep"])), true);
        assert_eq!(rows(&mut t, &m), peeked);
        // Off the chart (on the way to the tree) the peek stays.
        t.follow(&m, 0, None, false);
        assert_eq!(rows(&mut t, &m), peeked);
        // On the chart but on no sector it folds back to the centre.
        t.follow(&m, 0, None, true);
        assert_eq!(rows(&mut t, &m), ["root", "a", "b"]);
        assert_eq!(t.ensure_visible, Some(0));

        // Another directory replaces the peek; turning Follow off drops it.
        t.follow(&m, 0, Some(id(&["a", "a1", "deep"])), true);
        t.follow(&m, 0, Some(id(&["b", "b1"])), true);
        assert_eq!(rows(&mut t, &m), ["root", "a", "b", "b1"]);
        t.follow_hover = false;
        t.follow(&m, 0, Some(id(&["a", "a1", "deep"])), true);
        assert_eq!(rows(&mut t, &m), ["root", "a", "b"]);
    }
}
