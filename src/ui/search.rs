//! Search tab: files and folders of the whole scan whose names contain the
//! query. The scope is the whole scan rather than the chart's centre, so a
//! click that moves the chart does not change the list.

use std::ops::Range;

use egui::{
    Align, Align2, Color32, CursorIcon, FontId, Key, Layout, Margin, Pos2, Rect, Response, RichText,
    ScrollArea, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2,
};

use crate::format::{human_size, thousands};
use crate::model::{ItemKind, Metric, Model, match_ranges};
use crate::ui::files::{ListSort, ROW_HEIGHT, folder_under, item_row, sort_combo};
use crate::ui::item_menu;
use crate::ui::tree::TreeAction;

/// Number of matches listed (the largest ones).
const LIMIT: usize = 200;

#[derive(Default)]
pub struct SearchView {
    pub query: String,
    /// Put the keyboard focus into the field on the next frame (Ctrl+F).
    pub focus: bool,
    /// Match whole words only (kept in the settings).
    pub whole_word: bool,
    /// Files, folders or both (kept in the settings).
    pub kind: ItemKind,
    /// Order of the rows (kept in the settings).
    pub sort: ListSort,
    /// Order the rows are in now; `None` right after a search.
    sorted_by: Option<ListSort>,
    /// `(query, metric, whole_word, kind)` the rows were built for.
    key: Option<(String, Metric, bool, ItemKind)>,
    count: usize,
    /// Size of all matches, each byte once.
    total: u64,
    /// Matching bytes per node, for colouring the chart.
    hits: Vec<u64>,
    /// Bumped whenever `hits` changes; never 0 once searched.
    generation: u64,
    /// The largest matches in the order `sorted_by`: item id, its folder
    /// relative to the scan root and the parts of its name that match.
    rows: Vec<(u32, String, Vec<Range<usize>>)>,
    selected: Option<u32>,
}

impl SearchView {
    /// Forget the results (new model); the query and the mode stay and are
    /// run again.
    pub fn reset(&mut self) {
        *self = Self {
            query: std::mem::take(&mut self.query),
            whole_word: self.whole_word,
            kind: self.kind,
            sort: self.sort,
            generation: self.generation,
            ..Self::default()
        };
    }

    /// Matches to colour in on the chart, with their generation: while
    /// there are any.
    pub fn highlight(&self) -> Option<(u64, &[u64])> {
        (self.count > 0 && !self.query.trim().is_empty()).then_some((self.generation, &self.hits[..]))
    }

    /// A click on a folder navigates to it, on a file to its folder.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        model: &Model,
        metric: Metric,
        chart_hovered: Option<u32>,
    ) -> TreeAction {
        let field = ui.add(
            egui::TextEdit::singleline(&mut self.query)
                .hint_text("Name or mask like *.mp4 (Ctrl+F)")
                .desired_width(f32::INFINITY)
                .margin(Margin {
                    left: 4,
                    right: (4.0 + CLEAR_SIZE + 2.0 + 2.0 * (TOGGLE_SIZE.x + 2.0) + 1.0) as i8,
                    top: 2,
                    bottom: 2,
                }),
        );
        let mut focus = std::mem::take(&mut self.focus);
        // Esc in a non-empty field clears it and keeps the focus; in an
        // empty one it just leaves the field (egui's default).
        if field.lost_focus()
            && ui.input(|i| i.key_pressed(Key::Escape))
            && !self.query.is_empty()
        {
            self.query.clear();
            focus = true;
        }
        // Buttons inside the field's right end: [x] [kind] [ab], the toggles
        // outermost so that they do not move when the clear button appears.
        let word = Rect::from_center_size(
            pos2(field.rect.right() - 3.0 - TOGGLE_SIZE.x / 2.0, field.rect.center().y),
            TOGGLE_SIZE,
        );
        let applies = !crate::model::masks_only(&self.query);
        if whole_word_button(ui, word, self.whole_word, applies).clicked() {
            self.whole_word = !self.whole_word;
            focus = true;
        }
        let kind = word.translate(vec2(-(TOGGLE_SIZE.x + 2.0), 0.0));
        if kind_button(ui, kind, self.kind).clicked() {
            self.kind = match self.kind {
                ItemKind::All => ItemKind::Files,
                ItemKind::Files => ItemKind::Folders,
                ItemKind::Folders => ItemKind::All,
            };
            focus = true;
        }
        let clear = pos2(kind.left() - 2.0 - CLEAR_SIZE / 2.0, word.center().y);
        if !self.query.is_empty() && clear_button(ui, clear).clicked() {
            self.query.clear();
            focus = true;
        }
        if focus {
            field.request_focus();
        }
        let stale = self
            .key
            .as_ref()
            .is_none_or(|(q, m, w, k)| {
                *q != self.query || *m != metric || *w != self.whole_word || *k != self.kind
            });
        if stale {
            let found = model.search(0, self.query.trim(), metric, LIMIT, self.whole_word, self.kind);
            (self.count, self.total, self.hits) = (found.count, found.total, found.hits);
            self.generation += 1;
            let query = self.query.trim();
            self.rows = found
                .ids
                .into_iter()
                .map(|id| {
                    let marks = match_ranges(query, self.whole_word, model.name(id));
                    (id, folder_under(model, 0, id), marks)
                })
                .collect();
            self.key = Some((self.query.clone(), metric, self.whole_word, self.kind));
            self.sorted_by = None;
        }

        let mut action = TreeAction::default();
        if self.query.trim().is_empty() {
            ui.weak(
                "Type part of a name, or a mask such as *.mp4, to search the whole scan. \
                 Several words must all match, *.mp4;*.mkv (or mp4|mkv) matches either, \
                 \"quotes\" keep a phrase together.",
            );
            return action;
        }
        let summary = match self.count {
            0 => "No matches".to_string(),
            1 => format!("1 match ({})", human_size(self.total)),
            n => format!("{} matches ({})", thousands(n as u64), human_size(self.total)),
        };
        let summary = if self.count > LIMIT {
            format!("{summary}, the {LIMIT} largest shown")
        } else {
            summary
        };
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                sort_combo(ui, "search_sort", &mut self.sort);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(&summary).weak()).truncate())
                        .on_hover_text(format!(
                            "{summary}\nTotal size of the matches; files inside a matching folder count once"
                        ));
                });
            });
        });
        if self.sorted_by != Some(self.sort) {
            self.sort.apply(model, metric, &mut self.rows, |r| r.0);
            self.sorted_by = Some(self.sort);
        }
        let show_date = self.sort != ListSort::Size;
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, self.rows.len(), |ui, range| {
                for (id, folder, marks) in &self.rows[range] {
                    let id = *id;
                    let highlighted = Some(id) == self.selected || Some(id) == chart_hovered;
                    let row = item_row(ui, model, id, folder, marks, metric, highlighted, show_date);
                    if row.clicked() {
                        self.selected = Some(id);
                        let n = model.node(id);
                        action.selected = Some(if n.is_dir { id } else { n.parent });
                    }
                    if row.hovered() {
                        action.hovered = Some(id);
                    }
                    row.context_menu(|ui| {
                        if let Some(c) = item_menu(ui, id) {
                            action.command = Some(c);
                        }
                    });
                }
            });
        action
    }
}

/// Side of the clear button's square, in points.
const CLEAR_SIZE: f32 = 16.0;

/// Size of the toggles in the field, in points.
const TOGGLE_SIZE: Vec2 = vec2(22.0, 16.0);

/// A round "x" button centred at `center` inside the search field, drawn
/// rather than taken from a font so it stays crisp at any scale.
fn clear_button(ui: &mut Ui, center: Pos2) -> Response {
    let rect = Rect::from_center_size(center, Vec2::splat(CLEAR_SIZE));
    let resp = ui
        .interact(rect, ui.id().with("clear_search"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text("Clear (Esc)");
    let visuals = ui.visuals();
    let fill = if resp.hovered() {
        visuals.text_color()
    } else {
        visuals.weak_text_color()
    };
    let painter = ui.painter();
    let c = rect.center();
    painter.circle_filled(c, CLEAR_SIZE * 0.4, fill);
    let d = CLEAR_SIZE * 0.14;
    let stroke = Stroke::new(1.5, visuals.extreme_bg_color);
    painter.line_segment([c + vec2(-d, -d), c + vec2(d, d)], stroke);
    painter.line_segment([c + vec2(-d, d), c + vec2(d, -d)], stroke);
    resp
}

/// A toggle in the field at `rect`: its background, highlighted with the
/// selection colour while `on` and faded to `fade`, and the colour to draw
/// its glyph with.
fn toggle(ui: &mut Ui, rect: Rect, id: &str, on: bool, fade: f32, tip: &str) -> (Response, Color32) {
    let resp = ui
        .interact(rect, ui.id().with(id), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(tip);
    let visuals = ui.visuals();
    if on {
        ui.painter()
            .rect_filled(rect, 3.0, visuals.selection.bg_fill.gamma_multiply(fade));
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, visuals.widgets.hovered.weak_bg_fill);
    }
    let color = if on || resp.hovered() {
        visuals.strong_text_color()
    } else {
        visuals.weak_text_color()
    };
    (resp, color.gamma_multiply(fade))
}

/// The whole-word toggle: "ab" over a bracket, as in code editors; faded
/// unless it `applies` (a query of masks only ignores it).
fn whole_word_button(ui: &mut Ui, rect: Rect, on: bool, applies: bool) -> Response {
    let tip = if applies {
        "Match whole words only"
    } else {
        "Match whole words only (not used with masks)"
    };
    let (resp, color) = toggle(ui, rect, "whole_word", on, if applies { 1.0 } else { 0.4 }, tip);
    let painter = ui.painter();
    let c = rect.center();
    painter.text(c - vec2(0.0, 2.0), Align2::CENTER_CENTER, "ab", FontId::proportional(11.0), color);
    let (y, x0, x1) = (rect.bottom() - 3.0, c.x - 6.0, c.x + 6.0);
    painter.line(
        vec![pos2(x0, y - 2.5), pos2(x0, y), pos2(x1, y), pos2(x1, y - 2.5)],
        Stroke::new(1.0, color),
    );
    resp
}

/// The files / folders filter, cycling through the kinds on click: a page,
/// a folder, or both side by side; highlighted while it filters.
fn kind_button(ui: &mut Ui, rect: Rect, kind: ItemKind) -> Response {
    let tip = match kind {
        ItemKind::All => "Files and folders (click: files only)",
        ItemKind::Files => "Files only (click: folders only)",
        ItemKind::Folders => "Folders only (click: files and folders)",
    };
    let (resp, color) = toggle(ui, rect, "item_kind", kind != ItemKind::All, 1.0, tip);
    let stroke = Stroke::new(1.0, color);
    let painter = ui.painter();
    let c = rect.center();
    // A page 7x10 with a folded corner, centred at `p`.
    let page = |p: Pos2| {
        let (l, r, t, b) = (p.x - 3.5, p.x + 3.5, p.y - 5.0, p.y + 5.0);
        painter.add(Shape::closed_line(
            vec![pos2(l, t), pos2(r - 3.0, t), pos2(r, t + 3.0), pos2(r, b), pos2(l, b)],
            stroke,
        ));
        painter.line(vec![pos2(r - 3.0, t), pos2(r - 3.0, t + 3.0), pos2(r, t + 3.0)], stroke);
    };
    // A folder 11x8 with a tab, centred at `p`.
    let folder = |p: Pos2| {
        let (l, r, t, b) = (p.x - 5.5, p.x + 5.5, p.y - 4.0, p.y + 4.0);
        painter.add(Shape::closed_line(
            vec![
                pos2(l, t),
                pos2(l + 4.0, t),
                pos2(l + 5.5, t + 1.5),
                pos2(r, t + 1.5),
                pos2(r, b),
                pos2(l, b),
            ],
            stroke,
        ));
    };
    match kind {
        ItemKind::All => {
            folder(c + vec2(-4.5, 0.5));
            page(c + vec2(6.0, 0.0));
        }
        ItemKind::Files => page(c),
        ItemKind::Folders => folder(c + vec2(0.0, 0.5)),
    }
    resp
}
