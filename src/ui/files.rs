//! "Largest files" list: the biggest files under the chart's current root.

use std::cmp::Reverse;
use std::ops::Range;

use egui::{Align, Align2, Layout, RichText, ScrollArea, Sense, Ui, vec2};

use crate::format::{date, human_size};
use crate::model::{Metric, Model, NO_NODE};
use crate::ui::chart::modified_line;
use crate::ui::item_menu;
use crate::ui::tree::TreeAction;

/// Number of files listed.
const LIMIT: usize = 100;
pub(super) const ROW_HEIGHT: f32 = 20.0;
/// Widths of the size and date columns, in points.
const SIZE_WIDTH: f32 = 60.0;
const DATE_WIDTH: f32 = 72.0;
/// Choices of the age filter, in years (0 = any age).
const AGE_CHOICES: [u32; 5] = [0, 1, 2, 5, 10];
/// Average length of a year, in seconds.
const YEAR_SECS: u32 = 31_557_600;

/// Order of the rows of a list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ListSort {
    #[default]
    Size,
    /// Least recently modified first; unknown times last.
    Oldest,
    /// Most recently modified first.
    Newest,
}

impl ListSort {
    const ALL: [ListSort; 3] = [ListSort::Size, ListSort::Oldest, ListSort::Newest];

    fn label(self) -> &'static str {
        match self {
            ListSort::Size => tr!("Largest first", "Сначала большие"),
            ListSort::Oldest => tr!("Oldest first", "Сначала старые"),
            ListSort::Newest => tr!("Newest first", "Сначала новые"),
        }
    }

    /// Sort `rows`, whose item ids `id` gives. Equal keys fall back to the
    /// size, then to the id, so the order is the same on every run; by size
    /// it is the order the lists are built in.
    pub(super) fn apply<T>(self, model: &Model, metric: Metric, rows: &mut [T], id: impl Fn(&T) -> u32) {
        let size = |i: u32| Reverse(model.node(i).metric(metric));
        match self {
            ListSort::Size => rows.sort_by_key(|r| {
                let i = id(r);
                (size(i), i)
            }),
            ListSort::Oldest => rows.sort_by_key(|r| {
                let i = id(r);
                let t = model.node(i).modified;
                (if t == 0 { u32::MAX } else { t }, size(i), i)
            }),
            ListSort::Newest => rows.sort_by_key(|r| {
                let i = id(r);
                (Reverse(model.node(i).modified), size(i), i)
            }),
        }
    }
}

/// Order picker above a list.
pub(super) fn sort_combo(ui: &mut Ui, salt: &str, sort: &mut ListSort) {
    egui::ComboBox::from_id_salt(salt)
        .selected_text(sort.label())
        .show_ui(ui, |ui| {
            for s in ListSort::ALL {
                ui.selectable_value(sort, s, s.label());
            }
        })
        .response
        .on_hover_text(tr!(
            "Order of the list; the date is that of the last change",
            "Порядок списка; дата — время последнего изменения"
        ));
}

fn age_label(years: u32) -> String {
    match years {
        0 => tr!("Any age", "Любой возраст").into(),
        1 => tr!("Older than 1 year", "Старше 1 года").into(),
        n => tr!(format!("Older than {n} years"), format!("Старше {n} лет")),
    }
}

#[derive(Default)]
pub struct FilesView {
    /// Order of the rows (kept in the settings).
    pub sort: ListSort,
    /// List only files not modified for this many years, 0 for all (kept
    /// in the settings).
    pub older_than: u32,
    /// `(root, metric, older_than, sort)` the rows were built for.
    key: Option<(u32, Metric, u32, ListSort)>,
    /// Largest first: file id and its folder relative to the root.
    rows: Vec<(u32, String)>,
    /// File last clicked; stays highlighted after the chart moves into its
    /// folder (it is still in the list there).
    selected: Option<u32>,
    scroll_to_selected: bool,
}

impl FilesView {
    /// Forget everything but the order and the filter (new model).
    pub fn reset(&mut self) {
        *self = Self {
            sort: self.sort,
            older_than: self.older_than,
            ..Self::default()
        };
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
        ui.horizontal(|ui| {
            sort_combo(ui, "files_sort", &mut self.sort);
            egui::ComboBox::from_id_salt("files_age")
                .selected_text(age_label(self.older_than))
                .show_ui(ui, |ui| {
                    for years in AGE_CHOICES {
                        ui.selectable_value(&mut self.older_than, years, age_label(years));
                    }
                })
                .response
                .on_hover_text(tr!(
                    "List only files not modified for this long before the scan",
                    "Показывать только файлы, не изменявшиеся столько времени до сканирования"
                ));
        });

        let key = (root, metric, self.older_than, self.sort);
        if self.key != Some(key) {
            self.key = Some(key);
            let before = (self.older_than > 0)
                .then(|| model.scanned_at.saturating_sub(self.older_than * YEAR_SECS));
            self.rows = model
                .largest_files(root, metric, LIMIT, before)
                .into_iter()
                .map(|id| (id, folder_under(model, root, id)))
                .collect();
            self.sort.apply(model, metric, &mut self.rows, |r| r.0);
            self.scroll_to_selected = self.selected.is_some();
        }

        let mut action = TreeAction::default();
        if self.rows.is_empty() {
            let years = self.older_than;
            ui.weak(match years {
                0 => tr!("No files here", "Здесь нет файлов").to_string(),
                1 => tr!("No files here older than 1 year", "Здесь нет файлов старше 1 года").to_string(),
                n => tr!(
                    format!("No files here older than {n} years"),
                    format!("Здесь нет файлов старше {n} лет")
                ),
            });
            return action;
        }
        let show_date = self.sort != ListSort::Size || self.older_than > 0;
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
                let highlighted = Some(id) == self.selected || Some(id) == chart_hovered;
                let row = item_row(ui, model, id, folder, &[], metric, highlighted, show_date);
                if row.clicked() {
                    self.selected = Some(id);
                    action.selected = Some(model.node(id).parent);
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

/// Background of the parts of a name that a search matched, on the light
/// and on the dark theme.
const MARK_BG: egui::Color32 = egui::Color32::from_rgb(255, 226, 130);
const MARK_BG_DARK: egui::Color32 = egui::Color32::from_rgb(112, 86, 18);

/// Weak `text` right-aligned in a cell `width` wide, so that the column
/// lines up whatever the width of its values.
pub(super) fn cell(ui: &mut Ui, width: f32, text: String) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_HEIGHT), Sense::hover());
    let font = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().weak_text_color();
    ui.painter().text(rect.right_center(), Align2::RIGHT_CENTER, text, font, color);
}

/// One list row: the item's name with the byte ranges `marks` on a
/// highlighter background, then its `folder` (weak), and on the right its
/// last change (with `show_date`) and its size; the full path and the
/// time are in the tooltip.
#[allow(clippy::too_many_arguments)]
pub(super) fn item_row(
    ui: &mut Ui,
    model: &Model,
    id: u32,
    folder: &str,
    marks: &[Range<usize>],
    metric: Metric,
    highlighted: bool,
    show_date: bool,
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.set_min_height(ROW_HEIGHT);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let node = model.node(id);
            cell(ui, SIZE_WIDTH, human_size(node.metric(metric)));
            if show_date {
                cell(ui, DATE_WIDTH, date(node.modified));
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let mut text = egui::text::LayoutJob::default();
                let style = ui.style();
                let mark = if style.visuals.dark_mode { MARK_BG_DARK } else { MARK_BG };
                let name = model.name(id);
                let mut piece = |s: &str, marked: bool| {
                    let mut rich = RichText::new(s);
                    if marked {
                        rich = rich.background_color(mark);
                    }
                    rich.append_to(&mut text, style, egui::FontSelection::Default, Align::Center);
                };
                let mut at = 0;
                for r in marks {
                    piece(&name[at..r.start], false);
                    piece(&name[r.clone()], true);
                    at = r.end;
                }
                piece(&name[at..], false);
                if node.is_dir {
                    piece("\\", false);
                }
                if !folder.is_empty() {
                    RichText::new(format!("   {folder}")).weak().append_to(
                        &mut text,
                        style,
                        egui::FontSelection::Default,
                        Align::Center,
                    );
                }
                ui.add(egui::Button::selectable(highlighted, text).truncate())
                    .on_hover_ui(|ui| {
                        ui.label(model.path(id));
                        if let Some(line) = modified_line(model, node) {
                            ui.weak(line);
                        }
                    })
            })
            .inner
        })
        .inner
    })
    .inner
}

/// Folder of `item` relative to `root` (`""` directly under the root).
pub(super) fn folder_under(model: &Model, root: u32, item: u32) -> String {
    let mut parts = Vec::new();
    let mut cur = model.node(item).parent;
    while cur != root && cur != NO_NODE {
        parts.push(model.name(cur));
        cur = model.node(cur).parent;
    }
    parts.reverse();
    parts.join("\\")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RawDir, RawFile};

    #[test]
    fn sorts_by_size_and_by_date() {
        let f = |name: &str, size: u64, modified: u32| RawFile {
            name: name.into(),
            size,
            alloc: size,
            modified,
        };
        let raw = RawDir {
            name: "root".into(),
            files: vec![f("big", 30, 200), f("mid", 20, 100), f("unknown", 15, 0), f("small", 10, 300)],
            ..Default::default()
        };
        let m = Model::from_raw(raw, "X:\\".into(), 1);
        let names = |sort: ListSort| {
            let mut ids: Vec<u32> = m.children(0).collect();
            ids.reverse();
            sort.apply(&m, Metric::Logical, &mut ids, |&i| i);
            ids.iter().map(|&i| m.name(i)).collect::<Vec<_>>()
        };
        assert_eq!(names(ListSort::Size), ["big", "mid", "unknown", "small"]);
        assert_eq!(names(ListSort::Oldest), ["mid", "big", "small", "unknown"]);
        assert_eq!(names(ListSort::Newest), ["small", "big", "mid", "unknown"]);
    }
}
