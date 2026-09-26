//! Search tab: files and folders of the whole scan whose names contain the
//! query. The scope is the whole scan rather than the chart's centre, so a
//! click that moves the chart does not change the list.

use egui::{ScrollArea, Ui};

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
                .desired_width(f32::INFINITY),
        );
        if std::mem::take(&mut self.focus) {
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
