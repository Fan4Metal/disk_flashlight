//! "Types" tab: the file types under the chart's centre with their total
//! size, share and number of files; a click colours the files of a type on
//! the chart, a double click lists them in the Search tab.

use std::collections::HashMap;

use egui::{Align, Layout, RichText, ScrollArea, Sense, Ui, vec2};

use crate::format::{human_size, percent};
use crate::i18n::{count, files};
use crate::model::{Metric, Model};
use crate::render::Palette;
use crate::types::{NO_EXTENSION, TypeStat};
use crate::ui::files::{ROW_HEIGHT, cell};

/// Widths of the share and size columns, in points.
const SHARE_WIDTH: f32 = 44.0;
const SIZE_WIDTH: f32 = 60.0;
const SWATCH: f32 = 10.0;

/// What the tab asks the app to do.
#[derive(Default)]
pub struct TypesAction {
    /// Open the Search tab with this query (`*.mp4`).
    pub search: Option<String>,
}

#[derive(Default)]
pub struct TypesView {
    /// `(root, metric)` the rows were built for.
    key: Option<(u32, Metric)>,
    /// Types under the root, largest first.
    rows: Vec<TypeStat>,
    /// Size of the root by the metric, for the shares.
    total: u64,
    /// Type whose files are coloured on the chart.
    selected: Option<u16>,
    /// Matching bytes per node for `selected`, and what they were built for.
    hits: Vec<u64>,
    hits_key: Option<(u16, Metric)>,
    generation: u64,
    /// Windows' descriptions of the types shown so far.
    descriptions: HashMap<u16, Option<String>>,
}

impl TypesView {
    /// Forget everything (new model); the type ids change with it.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Files of the selected type to colour on the chart.
    pub fn highlight(&self) -> Option<(u64, &[u64])> {
        self.selected.map(|_| (self.generation, &self.hits[..]))
    }

    fn description(&mut self, model: &Model, ty: u16) -> Option<&str> {
        self.descriptions
            .entry(ty)
            .or_insert_with(|| {
                (ty != NO_EXTENSION)
                    .then(|| crate::scan::win::type_description(model.types.name(ty)))
                    .flatten()
            })
            .as_deref()
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        model: &Model,
        root: u32,
        metric: Metric,
        palette: &Palette,
    ) -> TypesAction {
        let mut action = TypesAction::default();
        if self.key != Some((root, metric)) {
            self.key = Some((root, metric));
            self.rows = model.types.stats(model, root, metric);
            self.total = model.node(root).metric(metric);
        }
        if let Some(ty) = self.selected
            && self.hits_key != Some((ty, metric))
        {
            self.hits = model.types.hits(model, ty, metric);
            self.hits_key = Some((ty, metric));
            self.generation = crate::ui::next_generation();
        }

        if self.rows.is_empty() {
            ui.weak(tr!("No files here", "Здесь нет файлов"));
            return action;
        }
        let summary = count(self.rows.len() as u64, ["type", "types"], ["тип", "типа", "типов"]);
        ui.weak(summary).on_hover_text(tr!(
            "Click a type to colour its files on the chart; double-click to list them in Search",
            "Щелчок по типу выделяет его файлы на диаграмме, двойной щелчок показывает их во вкладке поиска"
        ));

        let mut clicked = None;
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, self.rows.len(), |ui, range| {
                for i in range {
                    let s = self.rows[i];
                    let name = match s.ty {
                        NO_EXTENSION => tr!("(no extension)", "(без расширения)").to_string(),
                        ty => format!(".{}", model.types.name(ty)),
                    };
                    let description = self.description(model, s.ty).map(str::to_string);
                    let colour = palette.type_color(model.types.rank(s.ty), 0, 1);
                    let selected = self.selected == Some(s.ty);
                    let row = type_row(ui, &name, description.as_deref(), colour, selected, |ui| {
                        cell(ui, SIZE_WIDTH, human_size(s.metric(metric)));
                        cell(ui, SHARE_WIDTH, percent(s.metric(metric), self.total));
                    });
                    let row = row.on_hover_ui(|ui| {
                        ui.strong(match &description {
                            Some(d) => format!("{name}  —  {d}"),
                            None => name.clone(),
                        });
                        ui.label(files(s.files));
                        let (size, alloc) = (human_size(s.size), human_size(s.alloc));
                        ui.label(tr!(
                            format!("{size}, {alloc} on disk"),
                            format!("{size}, на диске {alloc}")
                        ));
                    });
                    if row.double_clicked() && s.ty != NO_EXTENSION {
                        action.search = Some(format!("*.{}", model.types.name(s.ty)));
                    } else if row.clicked() {
                        clicked = Some(s.ty);
                    }
                }
            });
        if let Some(ty) = clicked {
            // A second click on the selected type clears the colouring.
            self.selected = (self.selected != Some(ty)).then_some(ty);
            self.hits_key = None;
        }
        action
    }
}

/// One row: a colour swatch, the extension and Windows' description of it,
/// and the `cells` added on the right (right to left).
fn type_row(
    ui: &mut Ui,
    name: &str,
    description: Option<&str>,
    colour: egui::Color32,
    selected: bool,
    cells: impl FnOnce(&mut Ui),
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.set_min_height(ROW_HEIGHT);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            cells(ui);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(SWATCH, ROW_HEIGHT), Sense::hover());
                let swatch = egui::Rect::from_center_size(rect.center(), vec2(SWATCH, SWATCH));
                ui.painter().rect_filled(swatch, 2.0, colour);
                let mut text = egui::text::LayoutJob::default();
                let style = ui.style();
                RichText::new(name).append_to(&mut text, style, egui::FontSelection::Default, Align::Center);
                if let Some(d) = description {
                    RichText::new(format!("   {d}")).weak().append_to(
                        &mut text,
                        style,
                        egui::FontSelection::Default,
                        Align::Center,
                    );
                }
                ui.add(egui::Button::selectable(selected, text).truncate())
            })
            .inner
        })
        .inner
    })
    .inner
}
