//! Search tab: files and folders of the whole scan whose names contain the
//! query. The scope is the whole scan rather than the chart's centre, so a
//! click that moves the chart does not change the list.

use egui::{
    Align2, CursorIcon, FontId, Key, Margin, Pos2, Rect, Response, ScrollArea, Sense, Stroke, Ui,
    Vec2, pos2, vec2,
};

use crate::format::{human_size, thousands};
use crate::model::{Metric, Model};
use crate::ui::files::{ROW_HEIGHT, folder_under, item_row};
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
    /// `(query, metric, whole_word)` the rows were built for.
    key: Option<(String, Metric, bool)>,
    count: usize,
    /// Size of all matches, each byte once.
    total: u64,
    /// Matching bytes per node, for colouring the chart.
    hits: Vec<u64>,
    /// Bumped whenever `hits` changes; never 0 once searched.
    generation: u64,
    /// Largest first: item id and its folder relative to the scan root.
    rows: Vec<(u32, String)>,
    selected: Option<u32>,
}

impl SearchView {
    /// Forget the results (new model); the query and the mode stay and are
    /// run again.
    pub fn reset(&mut self) {
        *self = Self {
            query: std::mem::take(&mut self.query),
            whole_word: self.whole_word,
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
                    right: (4.0 + CLEAR_SIZE + 2.0 + WORD_SIZE.x + 3.0) as i8,
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
        // Buttons inside the field's right end: [x] [ab], the toggle outermost
        // so that it does not move when the clear button appears.
        let word = Rect::from_center_size(
            pos2(field.rect.right() - 3.0 - WORD_SIZE.x / 2.0, field.rect.center().y),
            WORD_SIZE,
        );
        let mask = self.query.contains(['*', '?']);
        if whole_word_button(ui, word, self.whole_word, !mask).clicked() {
            self.whole_word = !self.whole_word;
            focus = true;
        }
        let clear = pos2(word.left() - 2.0 - CLEAR_SIZE / 2.0, word.center().y);
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
            .is_none_or(|(q, m, w)| *q != self.query || *m != metric || *w != self.whole_word);
        if stale {
            let found = model.search(0, self.query.trim(), metric, LIMIT, self.whole_word);
            (self.count, self.total, self.hits) = (found.count, found.total, found.hits);
            self.generation += 1;
            self.rows = found.ids.into_iter().map(|id| (id, folder_under(model, 0, id))).collect();
            self.key = Some((self.query.clone(), metric, self.whole_word));
        }

        let mut action = TreeAction::default();
        if self.query.trim().is_empty() {
            ui.weak("Type part of a name, or a mask such as *.mp4, to search the whole scan");
            return action;
        }
        let summary = match self.count {
            0 => "No matches".to_string(),
            1 => format!("1 match ({})", human_size(self.total)),
            n => format!("{} matches ({})", thousands(n as u64), human_size(self.total)),
        };
        ui.weak(if self.count > LIMIT {
            format!("{summary}, the {LIMIT} largest shown")
        } else {
            summary
        })
        .on_hover_text("Total size of the matches; files inside a matching folder count once");
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, self.rows.len(), |ui, range| {
                for (id, folder) in &self.rows[range] {
                    let id = *id;
                    let highlighted = Some(id) == self.selected || Some(id) == chart_hovered;
                    let row = item_row(ui, model, id, folder, metric, highlighted);
                    if row.clicked() {
                        self.selected = Some(id);
                        let n = model.node(id);
                        action.selected = Some(if n.is_dir { id } else { n.parent });
                    }
                    if row.hovered() {
                        action.hovered = Some(id);
                    }
                }
            });
        action
    }
}

/// Side of the clear button's square, in points.
const CLEAR_SIZE: f32 = 16.0;

/// Size of the whole-word toggle, in points.
const WORD_SIZE: Vec2 = vec2(22.0, 16.0);

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

/// The whole-word toggle in `rect`: "ab" over a bracket, as in code editors;
/// highlighted with the selection colour while `on`, faded unless `applies`
/// (masks ignore it).
fn whole_word_button(ui: &mut Ui, rect: Rect, on: bool, applies: bool) -> Response {
    let resp = ui
        .interact(rect, ui.id().with("whole_word"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(if applies {
            "Match whole words only"
        } else {
            "Match whole words only (not used with * and ?)"
        });
    let fade = if applies { 1.0 } else { 0.4 };
    let visuals = ui.visuals();
    let painter = ui.painter();
    if on {
        painter.rect_filled(rect, 3.0, visuals.selection.bg_fill.gamma_multiply(fade));
    } else if resp.hovered() {
        painter.rect_filled(rect, 3.0, visuals.widgets.hovered.weak_bg_fill);
    }
    let color = if on || resp.hovered() {
        visuals.strong_text_color()
    } else {
        visuals.weak_text_color()
    }
    .gamma_multiply(fade);
    let c = rect.center();
    painter.text(c - vec2(0.0, 2.0), Align2::CENTER_CENTER, "ab", FontId::proportional(11.0), color);
    let (y, x0, x1) = (rect.bottom() - 3.0, c.x - 6.0, c.x + 6.0);
    painter.line(
        vec![pos2(x0, y - 2.5), pos2(x0, y), pos2(x1, y), pos2(x1, y - 2.5)],
        Stroke::new(1.0, color),
    );
    resp
}
