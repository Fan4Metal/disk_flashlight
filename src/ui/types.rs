//! "Types" tab: the file types under the chart's centre with their total
//! size, share and number of files. A click colours the files of a type on
//! the chart, a double click lists them in the Search tab, and the arrow
//! unfolds the largest of them in place.

use std::collections::{HashMap, HashSet};

use egui::{Align, Layout, RichText, ScrollArea, Sense, Ui, vec2};

use crate::format::{human_size, percent};
use crate::i18n::{count, files};
use crate::model::{Metric, Model};
use crate::render::Palette;
use crate::types::{NO_EXTENSION, TypeStat};
use crate::ui::files::{ROW_HEIGHT, cell, folder_under, item_row};
use crate::ui::item_menu;
use crate::ui::tree::{ARROW_WIDTH, TreeAction, arrow};

/// Widths of the share and size columns, in points.
const SHARE_WIDTH: f32 = 44.0;
const SIZE_WIDTH: f32 = 60.0;
const SWATCH: f32 = 10.0;
/// Files listed under an unfolded type.
const FILES_SHOWN: usize = 20;

/// What the tab asks the app to do.
#[derive(Default)]
pub struct TypesAction {
    /// Navigation, hover and context menu of the files listed, as in the
    /// other lists.
    pub tree: TreeAction,
    /// Open the Search tab with this query (`*.mp4`).
    pub search: Option<String>,
}

/// One line of the list.
#[derive(Clone, Copy)]
enum Line {
    /// The type at this index of `rows`.
    Type(usize),
    /// A file of an unfolded type.
    File(u32),
    /// "All N files in Search" after the files of type `.0`, which has
    /// `.1` files here.
    More(u16, u64),
}

#[derive(Default)]
pub struct TypesView {
    /// `(root, metric)` the rows and files were built for.
    key: Option<(u32, Metric)>,
    /// Types under the root, largest first.
    rows: Vec<TypeStat>,
    /// Size of the root by the metric, for the shares.
    total: u64,
    /// Unfolded types; kept while moving through the folders.
    expanded: HashSet<u16>,
    /// Largest files of each unfolded type under the root, with their
    /// folders relative to it.
    files: HashMap<u16, Vec<(u32, String)>>,
    /// Lines of the list, rebuilt when the rows or the unfolding change.
    lines: Vec<Line>,
    lines_dirty: bool,
    /// Type whose files are coloured on the chart.
    selected: Option<u16>,
    /// File last clicked; stays highlighted after the chart moves into its
    /// folder.
    selected_file: Option<u32>,
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

    fn description(&mut self, model: &Model, ty: u16) -> Option<String> {
        self.descriptions
            .entry(ty)
            .or_insert_with(|| {
                (ty != NO_EXTENSION)
                    .then(|| crate::scan::win::type_description(model.types.name(ty)))
                    .flatten()
            })
            .clone()
    }

    /// The lines for the current rows and unfolded types, listing the
    /// largest files of each unfolded type (found once per root).
    fn build_lines(&mut self, model: &Model, root: u32, metric: Metric) {
        self.lines.clear();
        for (i, s) in self.rows.iter().enumerate() {
            self.lines.push(Line::Type(i));
            if !self.expanded.contains(&s.ty) {
                continue;
            }
            let ty = s.ty;
            let list = self.files.entry(ty).or_insert_with(|| {
                model
                    .largest_files_where(root, metric, FILES_SHOWN, |id, _| model.types.of(id) == ty)
                    .into_iter()
                    .map(|id| (id, folder_under(model, root, id)))
                    .collect()
            });
            self.lines.extend(list.iter().map(|&(id, _)| Line::File(id)));
            if s.files > list.len() as u64 && ty != NO_EXTENSION {
                self.lines.push(Line::More(ty, s.files));
            }
        }
        self.lines_dirty = false;
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        model: &Model,
        root: u32,
        metric: Metric,
        palette: &Palette,
        chart_hovered: Option<u32>,
    ) -> TypesAction {
        let mut action = TypesAction::default();
        if self.key != Some((root, metric)) {
            self.key = Some((root, metric));
            self.rows = model.types.stats(model, root, metric);
            self.total = model.node(root).metric(metric);
            self.files.clear();
            self.lines_dirty = true;
        }
        if self.lines_dirty {
            self.build_lines(model, root, metric);
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
            "Click a type to colour its files on the chart, the arrow to list its largest files; \
             double-click to list all of them in Search",
            "Щелчок по типу выделяет его файлы на диаграмме, стрелка раскрывает самые большие из них, \
             двойной щелчок показывает все во вкладке поиска"
        ));

        let (mut clicked, mut toggled) = (None, None);
        let lines = std::mem::take(&mut self.lines);
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, lines.len(), |ui, range| {
                for &line in &lines[range] {
                    match line {
                        Line::Type(i) => {
                            let s = self.rows[i];
                            let expanded = self.expanded.contains(&s.ty);
                            let (row, arrow) = self.type_row(ui, model, s, metric, palette, expanded);
                            if arrow {
                                toggled = Some(s.ty);
                            } else if row.double_clicked() && s.ty != NO_EXTENSION {
                                action.search = Some(format!("*.{}", model.types.name(s.ty)));
                            } else if row.clicked() {
                                clicked = Some(s.ty);
                            }
                        }
                        Line::File(id) => {
                            let folder = self.folder_of(id);
                            let highlighted = Some(id) == self.selected_file || Some(id) == chart_hovered;
                            let row = ui
                                .horizontal(|ui| {
                                    ui.add_space(ARROW_WIDTH + 4.0);
                                    item_row(ui, model, id, &folder, &[], metric, highlighted, false)
                                })
                                .inner;
                            if row.clicked() {
                                self.selected_file = Some(id);
                                action.tree.selected = Some(model.node(id).parent);
                            }
                            if row.hovered() {
                                action.tree.hovered = Some(id);
                            }
                            row.context_menu(|ui| {
                                if let Some(c) = item_menu(ui, id) {
                                    action.tree.command = Some(c);
                                }
                            });
                        }
                        Line::More(ty, n) => {
                            ui.horizontal(|ui| {
                                ui.set_min_height(ROW_HEIGHT);
                                ui.add_space(ARROW_WIDTH + 8.0);
                                let text = tr!(
                                    format!("All {} in Search", files(n)),
                                    format!("Все {} во вкладке поиска", files(n))
                                );
                                if ui.link(text).clicked() {
                                    action.search = Some(format!("*.{}", model.types.name(ty)));
                                }
                            });
                        }
                    }
                }
            });
        self.lines = lines;
        if let Some(ty) = toggled {
            if !self.expanded.remove(&ty) {
                self.expanded.insert(ty);
            }
            self.lines_dirty = true;
        }
        if let Some(ty) = clicked {
            // A second click on the selected type clears the colouring.
            self.selected = (self.selected != Some(ty)).then_some(ty);
            self.hits_key = None;
        }
        action
    }

    /// Folder of a listed file, relative to the root.
    fn folder_of(&self, id: u32) -> String {
        self.files
            .values()
            .flat_map(|list| list.iter())
            .find(|(f, _)| *f == id)
            .map(|(_, folder)| folder.clone())
            .unwrap_or_default()
    }

    /// One type: the unfolding arrow, a colour swatch, the extension and
    /// Windows' description of it, the share and the size. Returns the
    /// row's response and whether the arrow was clicked.
    fn type_row(
        &mut self,
        ui: &mut Ui,
        model: &Model,
        s: TypeStat,
        metric: Metric,
        palette: &Palette,
        expanded: bool,
    ) -> (egui::Response, bool) {
        let name = match s.ty {
            NO_EXTENSION => tr!("(no extension)", "(без расширения)").to_string(),
            ty => format!(".{}", model.types.name(ty)),
        };
        let description = self.description(model, s.ty);
        let colour = palette.type_color(model.types.rank(s.ty), 0, 1);
        let selected = self.selected == Some(s.ty);
        let total = self.total;
        let mut arrow_clicked = false;
        let row = ui
            .horizontal(|ui| {
                ui.set_min_height(ROW_HEIGHT);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    cell(ui, SIZE_WIDTH, human_size(s.metric(metric)));
                    cell(ui, SHARE_WIDTH, percent(s.metric(metric), total));
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        arrow_clicked = arrow(ui, expanded).clicked();
                        let (rect, _) = ui.allocate_exact_size(vec2(SWATCH, ROW_HEIGHT), Sense::hover());
                        let swatch = egui::Rect::from_center_size(rect.center(), vec2(SWATCH, SWATCH));
                        ui.painter().rect_filled(swatch, 2.0, colour);
                        let mut text = egui::text::LayoutJob::default();
                        let style = ui.style();
                        RichText::new(&name).append_to(&mut text, style, egui::FontSelection::Default, Align::Center);
                        if let Some(d) = &description {
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
            .inner;
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
        (row, arrow_clicked)
    }
}
