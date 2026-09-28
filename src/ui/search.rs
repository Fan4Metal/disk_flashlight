//! Search tab: files and folders of the whole scan whose names contain the
//! query. The scope is the whole scan rather than the chart's centre, so a
//! click that moves the chart does not change the list.

use egui::{
    CursorIcon, Key, Margin, Rect, Response, ScrollArea, Sense, Stroke, Ui, Vec2, pos2, vec2,
};

use crate::format::thousands;
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
    /// `(query, metric)` the rows were built for.
    key: Option<(String, Metric)>,
    count: usize,
    /// Largest first: item id and its folder relative to the scan root.
    rows: Vec<(u32, String)>,
    selected: Option<u32>,
}

impl SearchView {
    /// Forget the results (new model); the query stays and is run again.
    pub fn reset(&mut self) {
        *self = Self {
            query: std::mem::take(&mut self.query),
            ..Self::default()
        };
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
                .hint_text("Search names (Ctrl+F)")
                .desired_width(f32::INFINITY)
                .margin(Margin {
                    left: 4,
                    right: 4 + CLEAR_SIZE as i8,
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
        if !self.query.is_empty() && clear_button(ui, field.rect).clicked() {
            self.query.clear();
            focus = true;
        }
        if focus {
            field.request_focus();
        }
        let stale = self
            .key
            .as_ref()
            .is_none_or(|(q, m)| *q != self.query || *m != metric);
        if stale {
            let (count, ids) = model.search(0, self.query.trim(), metric, LIMIT);
            self.count = count;
            self.rows = ids.into_iter().map(|id| (id, folder_under(model, 0, id))).collect();
            self.key = Some((self.query.clone(), metric));
        }

        let mut action = TreeAction::default();
        if self.query.trim().is_empty() {
            ui.weak("Type part of a name to search the whole scan");
            return action;
        }
        ui.weak(match self.count {
            0 => "No matches".to_string(),
            n if n > LIMIT => format!("{} matches, the {LIMIT} largest shown", thousands(n as u64)),
            n => format!("{} matches", thousands(n as u64)),
        });
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

/// A round "x" button at the right end of the search field (`field`), drawn
/// rather than taken from a font so it stays crisp at any scale.
fn clear_button(ui: &mut Ui, field: Rect) -> Response {
    let rect = Rect::from_center_size(
        pos2(field.right() - 4.0 - CLEAR_SIZE / 2.0, field.center().y),
        Vec2::splat(CLEAR_SIZE),
    );
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
