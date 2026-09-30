//! The sunburst chart widget: caches layout + mesh, handles hover, tooltip,
//! click-to-navigate, zoom towards the cursor and panning.

use std::sync::Arc;

use egui::{
    Align2, CursorIcon, FontId, Mesh, PointerButton, Pos2, Rect, Response, Sense, Shape, Stroke,
    StrokeKind, Ui, Vec2, pos2, vec2,
};

use crate::format::{ago, date, human_size, percent, thousands};
use crate::i18n::{count, ru_plural};
use crate::layout::{self, Layout, LayoutParams, Sector, Step};
use crate::model::{Metric, Model, Node};
use crate::render::{self, ColorMode, Palette};
use crate::scan::win::DiskSpace;
use crate::ui::{ItemCommand, item_menu};

/// Approximate extent of the standard Windows arrow cursor below and to the
/// right of its hot spot, in points (it scales with DPI like the UI does).
const CURSOR_SIZE: egui::Vec2 = egui::vec2(12.0, 20.0);

const MIN_ZOOM: f32 = 0.5;
const MAX_ZOOM: f32 = 200.0;
/// Zoom factor applied by a click on a group of small items.
const GROUP_CLICK_ZOOM: f32 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartAction {
    None,
    Navigate(u32),
    Up,
    Command(ItemCommand),
}

#[derive(PartialEq, Clone, Copy)]
struct CacheKey {
    root: u32,
    metric: Metric,
    center: (i32, i32),
    radius: i32,
    view: (i32, i32, i32, i32),
    free: u64,
    /// Generation of the search matches coloured in, 0 for none.
    hits: u64,
    dark: bool,
}

pub struct ChartView {
    pub params: LayoutParams,
    pub palette: Palette,
    /// Chart radius relative to the fitted size.
    pub zoom: f32,
    /// Offset of the chart centre from the centre of the widget, in points.
    pub pan: Vec2,
    /// Single item under the mouse during the last frame (not groups).
    pub hovered: Option<u32>,
    /// Whether the pointer was over the chart during the last frame.
    pub pointer_inside: bool,
    /// The chart is driven by the keyboard: `selected` stands in for the
    /// item under the mouse until the mouse moves.
    keyboard: bool,
    /// Item chosen with the keyboard; `None` is the centre.
    selected: Option<u32>,
    /// Select the largest item once the next layout is built (after
    /// entering a folder from the keyboard).
    select_first: bool,
    /// Node whose context menu is open (right-clicked sector or the root).
    menu_node: Option<u32>,
    /// Root the current zoom/pan belongs to; a new root resets the view.
    view_root: Option<u32>,
    layout: Option<Layout>,
    mesh: Option<Arc<Mesh>>,
    key: Option<CacheKey>,
}

impl Default for ChartView {
    fn default() -> Self {
        Self {
            params: LayoutParams::default(),
            palette: Palette::default(),
            zoom: 1.0,
            pan: Vec2::ZERO,
            hovered: None,
            pointer_inside: false,
            keyboard: false,
            selected: None,
            select_first: false,
            menu_node: None,
            view_root: None,
            layout: None,
            mesh: None,
            key: None,
        }
    }
}

impl ChartView {
    /// Force a rebuild on the next frame (e.g. after a rescan).
    pub fn invalidate(&mut self) {
        self.key = None;
        self.layout = None;
        self.mesh = None;
        self.menu_node = None;
        // Ids of the old model; the next frame sets it again.
        self.hovered = None;
        self.keyboard = false;
        self.selected = None;
    }

    /// Move the keyboard selection; the first move starts from the item
    /// under the mouse.
    pub fn step(&mut self, model: &Model, step: Step) {
        let Some(layout) = &self.layout else { return };
        let from = if self.keyboard { self.selected } else { self.hovered };
        self.selected = layout.step(model, from, step);
        self.keyboard = true;
    }

    /// Select `id` from the keyboard (`None`: the largest item) once the
    /// chart shows the new centre.
    pub fn select(&mut self, id: Option<u32>) {
        self.keyboard = true;
        self.selected = id;
        self.select_first = id.is_none();
    }

    /// Back to following the mouse.
    pub fn clear_selection(&mut self) {
        self.keyboard = false;
        self.selected = None;
        self.select_first = false;
    }

    pub fn reset_view(&mut self) {
        self.zoom = 1.0;
        self.pan = Vec2::ZERO;
    }

    /// Change the zoom so that the chart point under `anchor` stays put.
    fn zoom_at(&mut self, rect: Rect, anchor: Pos2, factor: f32) {
        let new_zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let k = new_zoom / self.zoom;
        let center = rect.center() + self.pan;
        let new_center = anchor - (anchor - center) * k;
        self.pan = new_center - rect.center();
        self.zoom = new_zoom;
    }

    /// Keep at least part of the chart inside the widget.
    fn clamp_pan(&mut self, rect: Rect, radius: f32) {
        let lim = Vec2::new(rect.width() * 0.5 + radius * 0.9, rect.height() * 0.5 + radius * 0.9);
        self.pan = self.pan.clamp(-lim, lim);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut Ui,
        model: &Model,
        root: u32,
        metric: Metric,
        external_highlight: Option<u32>,
        disk: Option<DiskSpace>,
        hits: Option<(u64, &[u64])>,
    ) -> ChartAction {
        let size = ui.available_size();
        let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
        let rect = response.rect;

        if self.view_root != Some(root) {
            self.view_root = Some(root);
            self.reset_view();
        }
        // The palette follows the theme, keeping the colour mode.
        let dark = ui.visuals().dark_mode;
        if self.palette.dark != dark {
            self.palette = Palette {
                mode: self.palette.mode,
                ..Palette::for_theme(dark)
            };
        }

        // Wheel: zoom towards the cursor.
        if response.hovered() {
            let dy = ui.input(|i| i.smooth_scroll_delta.y);
            if dy != 0.0
                && let Some(p) = response.hover_pos()
            {
                self.zoom_at(rect, p, (dy * 0.002).exp());
            }
        }
        // Middle button: drag to pan, double click to reset.
        if response.double_clicked_by(PointerButton::Middle) {
            self.reset_view();
        } else if response.dragged_by(PointerButton::Middle) {
            self.pan += response.drag_delta();
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }

        let fit_radius = rect.width().min(rect.height()) * 0.5 * 0.96;
        let radius = fit_radius * self.zoom;
        self.clamp_pan(rect, radius);
        let center = rect.center() + self.pan;
        let key = CacheKey {
            root,
            metric,
            center: (center.x.round() as i32, center.y.round() as i32),
            radius: radius.round() as i32,
            view: (
                rect.min.x as i32,
                rect.min.y as i32,
                rect.max.x as i32,
                rect.max.y as i32,
            ),
            free: disk.map_or(0, |d| d.free),
            hits: hits.map_or(0, |(generation, _)| generation),
            dark,
        };
        if self.key != Some(key) || self.layout.is_none() {
            let l = layout::build_with_free(
                model,
                root,
                metric,
                center,
                radius,
                rect,
                &self.params,
                key.free,
            );
            let m = render::build_mesh(model, &l, &self.palette, hits.map(|(_, h)| h));
            self.layout = Some(l);
            self.mesh = Some(Arc::new(m));
            self.key = Some(key);
        }
        let layout = self.layout.as_ref().expect("layout built above");
        let mesh = self.mesh.as_ref().expect("mesh built above");
        // The mouse takes over from the keyboard as soon as it moves.
        if self.keyboard && ui.input(|i| i.pointer.delta() != Vec2::ZERO || i.pointer.any_pressed()) {
            self.keyboard = false;
            self.selected = None;
            self.select_first = false;
        }
        if std::mem::take(&mut self.select_first) {
            self.selected = layout.step(model, None, Step::Next);
        }

        // Background and guide rings.
        let painter = painter.with_clip_rect(rect);
        let pal = self.palette;
        painter.rect_filled(rect, 0.0, pal.background);
        let guide = Stroke::new(1.0, pal.guide);
        for &(_, r_out) in &layout.radii {
            for arc in render::guide_arcs(layout, r_out) {
                painter.add(Shape::line(arc, guide));
            }
        }

        painter.add(Shape::mesh(mesh.clone()));

        // Centre disc with root name and size.
        let r0 = layout.center_radius();
        painter.add(Shape::mesh(Arc::new(render::disc_mesh(
            layout,
            pal.center,
        ))));
        let root_node = model.node(root);
        let title = model.name(root);
        let font = FontId::proportional((r0 * 0.34).clamp(12.0, 30.0));
        painter.text(
            center - egui::vec2(0.0, font.size * 0.55),
            Align2::CENTER_CENTER,
            title,
            font.clone(),
            pal.center_text,
        );
        painter.text(
            center + egui::vec2(0.0, font.size * 0.55),
            Align2::CENTER_CENTER,
            human_size(root_node.metric(metric)),
            FontId::proportional((font.size * 0.7).max(11.0)),
            pal.center_subtext,
        );

        match self.palette.mode {
            ColorMode::Age => age_legend(&painter, rect, &self.palette),
            ColorMode::Type => type_legend(&painter, rect, &self.palette, model),
            _ => {}
        }

        // "Reset view" button in the corner, while zoomed or panned; the
        // chart underneath gets neither its hover nor its clicks.
        let reset = (self.zoom != 1.0 || self.pan != Vec2::ZERO).then(|| reset_button(ui, rect, self.zoom));
        let on_reset = reset.as_ref().is_some_and(|r| r.contains_pointer());

        // Hover / highlight.
        let panning = response.dragged_by(PointerButton::Middle);
        let pointer = response.hover_pos().filter(|_| !panning && !on_reset);
        // While a context menu is open its sector stays highlighted and the
        // tooltip is hidden, so the highlight shows what the menu acts on.
        let hit = match self.menu_node {
            Some(n) => layout.index.get(&n).copied(),
            None if self.keyboard => self.selected.and_then(|n| layout.index.get(&n).copied()),
            None => pointer.and_then(|p| layout.hit_test(p)),
        };
        // The tooltip of a keyboard selection opens at the sector's middle.
        let tip_at = if self.keyboard {
            hit.map(|(ring, i)| layout.sector_middle(ring, i))
        } else {
            pointer
        };
        let hit_sector: Option<Sector> = hit.map(|(ring, i)| layout.rings[ring][i]);
        self.pointer_inside = response.contains_pointer();
        self.hovered = match self.menu_node {
            Some(n) => Some(n),
            None => hit_sector.filter(|s| s.is_item()).map(|s| s.node),
        };

        if let Some((ring, i)) = hit {
            let overlay = render::highlight_mesh(layout, ring, i, pal.hover_overlay);
            painter.add(Shape::mesh(Arc::new(overlay)));
            for outline in render::sector_outline(layout, ring, i) {
                painter.add(Shape::closed_line(outline, Stroke::new(1.5, pal.outline)));
            }
        }
        if let Some(ext) = external_highlight
            && Some(ext) != self.hovered
            && let Some(&(ring, i)) = layout.index.get(&ext)
        {
            for outline in render::sector_outline(layout, ring, i) {
                painter.add(Shape::closed_line(
                    outline,
                    Stroke::new(2.0, pal.external),
                ));
            }
        }

        // Click handling.
        let mut action = ChartAction::None;
        let mut zoom_to: Option<Pos2> = None;
        // A left click while a menu is open only dismisses the menu.
        let menu_was_open = self.menu_node.is_some();
        if response.secondary_clicked() {
            // Right click: context menu for a single item, or for the current
            // root when the centre disc is clicked; nothing on empty space or
            // on a group of small items.
            self.menu_node = response.interact_pointer_pos().and_then(|p| {
                if layout.is_center(p) {
                    Some(root)
                } else {
                    layout
                        .hit_test(p)
                        .map(|(ring, i)| layout.rings[ring][i])
                        .filter(|s| s.is_item())
                        .map(|s| s.node)
                }
            });
        } else if response.clicked()
            && !menu_was_open
            && let Some(p) = response.interact_pointer_pos()
        {
            if layout.is_center(p) {
                action = ChartAction::Up;
            } else if let Some((ring, i)) = layout.hit_test(p) {
                let s = layout.rings[ring][i];
                if s.is_group() {
                    zoom_to = Some(p);
                } else if s.is_item() {
                    // A folder becomes the centre, a file takes the chart to
                    // its folder (no change for a file right in the centre).
                    let n = model.node(s.node);
                    action = ChartAction::Navigate(if n.is_dir { s.node } else { n.parent });
                }
            }
        }

        if let Some(node) = self.menu_node {
            // Popup::context_menu opens on this frame's secondary click and
            // returns None once the menu has been closed.
            let shown = response.context_menu(|ui| {
                if let Some(c) = item_menu(ui, node) {
                    action = ChartAction::Command(c);
                }
            });
            if shown.is_none() {
                self.menu_node = None;
            }
        }

        // Tooltip, shown immediately (no hover delay) like in OverDisk. It is
        // anchored to a box covering the arrow cursor rather than to the
        // pointer itself, so it opens below the arrow instead of under it;
        // near screen edges egui flips it to the other side of that box.
        if let (None, Some(s), Some(p)) = (self.menu_node, hit_sector, tip_at) {
            let cursor_box = egui::Rect::from_min_size(p, CURSOR_SIZE);
            egui::Tooltip::always_open(
                ui.ctx().clone(),
                response.layer_id,
                response.id,
                egui::PopupAnchor::ParentRect(cursor_box),
            )
            .show(|ui| {
                ui.set_max_width(360.0);
                // Three tiers: the name large, the numbers in plain text,
                // where it is and what a click does faint, with a gap
                // between them.
                ui.spacing_mut().item_spacing.y = 2.0;
                if s.is_free() {
                    tooltip_title(ui, tr!("Free space", "Свободное место"));
                    let total = disk.map_or(0, |d| d.total);
                    size_line(ui, s.group_size, None);
                    let (share, total) = (percent(s.group_size, total), human_size(total));
                    ui.label(tr!(format!("{share} of {total}"), format!("{share} от {total}")));
                } else if s.is_group() {
                    tooltip_title(
                        ui,
                        &count(
                            s.count as u64,
                            ["smaller item", "smaller items"],
                            ["мелкий элемент", "мелких элемента", "мелких элементов"],
                        ),
                    );
                    size_line(ui, s.group_size, None);
                    ui.label(shares(model, root, s.node, s.group_size, metric));
                    matches_line(ui, model, layout, &s, hits);
                    ui.add_space(TOOLTIP_GAP);
                    let path = model.path(s.node);
                    ui.weak(tr!(format!("in {path}"), format!("в {path}")));
                    ui.weak(tr!("Click or scroll to zoom in", "Щелчок или колесо мыши — увеличить"));
                } else {
                    let n = model.node(s.node);
                    tooltip_title(ui, model.name(s.node));
                    size_line(ui, n.size, Some(n.alloc));
                    ui.label(shares(model, root, n.parent, n.metric(metric), metric));
                    if let Some(line) = modified_line(model, n) {
                        ui.label(line);
                    }
                    matches_line(ui, model, layout, &s, hits);
                    ui.add_space(TOOLTIP_GAP);
                    if n.is_dir {
                        ui.weak(counts(n.files, n.dirs));
                    }
                    ui.weak(model.path(s.node));
                    if !n.is_dir && n.parent != root {
                        ui.weak(tr!("Click to open its folder", "Щелчок — перейти в его папку"));
                    }
                }
            });
        }

        if let Some(p) = zoom_to {
            self.zoom_at(rect, p, GROUP_CLICK_ZOOM);
            ui.ctx().request_repaint();
        }
        if reset.is_some_and(|r| r.clicked()) {
            self.reset_view();
            ui.ctx().request_repaint();
        }
        action
    }
}

/// Small button in the top right corner of the chart `rect`: a "fit" icon
/// (four corners) and the current `zoom` factor.
fn reset_button(ui: &mut Ui, rect: Rect, zoom: f32) -> Response {
    let size = vec2(64.0, 24.0);
    let btn = Rect::from_min_size(pos2(rect.right() - size.x - 8.0, rect.top() + 8.0), size);
    let resp = ui
        .interact(btn, ui.id().with("reset_view"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(tr!(
            "Reset the view (or double-click with the middle mouse button)",
            "Исходный вид (или двойной щелчок средней кнопкой мыши)"
        ));
    let visuals = ui.style().interact(&resp);
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect(btn, 4.0, visuals.weak_bg_fill, visuals.bg_stroke, StrokeKind::Inside);
    let (color, c) = (visuals.text_color(), pos2(btn.left() + 14.0, btn.center().y));
    let (half, arm) = (5.5, 3.5);
    for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let corner = c + vec2(sx * half, sy * half);
        painter.line(
            vec![corner - vec2(sx * arm, 0.0), corner, corner - vec2(0.0, sy * arm)],
            Stroke::new(1.5, color),
        );
    }
    let label = if zoom >= 10.0 {
        format!("{zoom:.0}×")
    } else {
        format!("{zoom:.1}×")
    };
    painter.text(
        pos2(btn.left() + 26.0, btn.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(13.0),
        color,
    );
    resp
}

/// Space between the tiers of the tooltip, in points.
const TOOLTIP_GAP: f32 = 5.0;

/// The tooltip's first line, larger than the rest.
fn tooltip_title(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(17.0).strong());
    ui.add_space(2.0);
}

/// `75.9 GB` in bold and, when it differs, the space taken on disk.
fn size_line(ui: &mut Ui, size: u64, alloc: Option<u64>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(egui::RichText::new(human_size(size)).size(15.0).strong());
        if let Some(alloc) = alloc.filter(|&a| human_size(a) != human_size(size)) {
            let alloc = human_size(alloc);
            ui.weak(tr!(format!("{alloc} on disk"), format!("{alloc} на диске")));
        }
    });
}

/// `1 234 files in 56 folders`, or `14 files` without subfolders.
fn counts(files: u32, dirs: u32) -> String {
    let (files, n) = (crate::i18n::files(files as u64), dirs as u64);
    if n == 0 {
        return files;
    }
    let dirs = thousands(n);
    tr!(
        format!("{files} in {}", crate::i18n::folders(n)),
        format!("{files} в {dirs} {}", ru_plural(n, "папке", "папках", "папках"))
    )
}

/// Tooltip line with the share `part` bytes take of `parent` and, deeper
/// in, of the centre `root`: `12% of Users, 3.4% of C:`.
fn shares(model: &Model, root: u32, parent: u32, part: u64, metric: Metric) -> String {
    let of = |id: u32| {
        let (share, name) = (percent(part, model.node(id).metric(metric)), model.name(id));
        tr!(format!("{share} of {name}"), format!("{share} от {name}"))
    };
    if parent == root {
        of(root)
    } else {
        format!("{}, {}", of(parent), of(root))
    }
}

/// `Modified: 2024-03-15 (5 months ago)` for a file; a folder gives the
/// newest time inside it. `None` when the time is unknown.
pub fn modified_line(model: &Model, n: &Node) -> Option<String> {
    (n.modified != 0).then(|| {
        let label = if n.is_dir {
            tr!("Last change inside", "Последнее изменение внутри")
        } else {
            tr!("Modified", "Последнее изменение")
        };
        let age = ago(model.scanned_at.saturating_sub(n.modified));
        format!("{label}: {} ({age})", date(n.modified))
    })
}

/// Colours of the type mode in the bottom left corner of the chart `rect`:
/// the coloured types in two columns, then the colour of all the others.
fn type_legend(painter: &egui::Painter, rect: Rect, palette: &Palette, model: &Model) {
    const COLUMN: f32 = 96.0;
    const ROW: f32 = 17.0;
    let mut entries: Vec<(egui::Color32, String)> = model
        .types
        .coloured()
        .into_iter()
        .map(|ty| {
            let name = match ty {
                crate::types::NO_EXTENSION => tr!("no ext.", "без расш.").to_string(),
                ty => format!(".{}", model.types.name(ty)),
            };
            (palette.type_color(model.types.rank(ty), 0, 1), name)
        })
        .collect();
    entries.push((palette.type_other, tr!("other", "прочие").to_string()));
    let rows = entries.len().div_ceil(2);
    let text = palette.legend_text;
    let backing = Rect::from_min_size(
        pos2(rect.left() + 8.0, rect.bottom() - 8.0 - (30.0 + rows as f32 * ROW)),
        vec2(2.0 * COLUMN + 20.0, 30.0 + rows as f32 * ROW),
    );
    painter.rect_filled(backing, 4.0, palette.legend_backing);
    painter.text(
        pos2(backing.left() + 12.0, backing.top() + 5.0),
        Align2::LEFT_TOP,
        tr!("File types", "Типы файлов"),
        FontId::proportional(12.0),
        text,
    );
    let small = FontId::proportional(11.0);
    for (i, (colour, name)) in entries.into_iter().enumerate() {
        let (col, row) = (i / rows, i % rows);
        let x = backing.left() + 12.0 + col as f32 * COLUMN;
        let y = backing.top() + 24.0 + row as f32 * ROW;
        let swatch = Rect::from_min_size(pos2(x, y + 3.0), vec2(10.0, 10.0));
        painter.rect_filled(swatch, 2.0, colour);
        painter.text(pos2(x + 15.0, y + 8.0), Align2::LEFT_CENTER, name, small.clone(), text);
    }
}

/// Colour scale of the age mode in the bottom left corner of the chart
/// `rect`, on a light backing so that it reads over the sectors.
fn age_legend(painter: &egui::Painter, rect: Rect, palette: &Palette) {
    const WIDTH: f32 = 220.0;
    const BAR: f32 = 10.0;
    let text = palette.legend_text;
    let small = FontId::proportional(11.0);
    let backing = Rect::from_min_size(
        pos2(rect.left() + 8.0, rect.bottom() - 58.0),
        vec2(WIDTH + 24.0, 50.0),
    );
    painter.rect_filled(backing, 4.0, palette.legend_backing);
    let left = backing.left() + 12.0;
    painter.text(
        pos2(left, backing.top() + 5.0),
        Align2::LEFT_TOP,
        tr!("Last modified", "Последнее изменение"),
        FontId::proportional(12.0),
        text,
    );
    let bar = Rect::from_min_size(pos2(left, backing.top() + 22.0), vec2(WIDTH, BAR));
    let mut mesh = Mesh::default();
    const STEPS: u32 = 48;
    for i in 0..=STEPS {
        let t = i as f32 / STEPS as f32;
        let color = palette.age_color(t, palette.sat_dir, palette.val_even);
        let x = bar.left() + WIDTH * t;
        mesh.colored_vertex(pos2(x, bar.top()), color);
        mesh.colored_vertex(pos2(x, bar.bottom()), color);
        if i > 0 {
            let v = 2 * i;
            mesh.add_triangle(v - 2, v - 1, v);
            mesh.add_triangle(v - 1, v + 1, v);
        }
    }
    painter.add(Shape::mesh(mesh));
    let ticks = [
        (0.0, tr!("now", "сейчас")),
        (7.0, tr!("week", "нед.")),
        (30.0, tr!("month", "мес.")),
        (365.0, tr!("year", "год")),
        (3650.0, tr!("10 years", "10 лет")),
    ];
    let last = ticks.len() - 1;
    for (i, (days, label)) in ticks.into_iter().enumerate() {
        let x = bar.left() + WIDTH * palette.age_t_of_days(days);
        painter.line_segment(
            [pos2(x, bar.bottom()), pos2(x, bar.bottom() + 3.0)],
            Stroke::new(1.0, text),
        );
        // The ends stay inside the bar.
        let align = match i {
            0 => Align2::LEFT_TOP,
            _ if i == last => Align2::RIGHT_TOP,
            _ => Align2::CENTER_TOP,
        };
        painter.text(pos2(x, bar.bottom() + 3.0), align, label, small.clone(), text);
    }
}

/// Tooltip line with how much of sector `s` the search matches, while they
/// are coloured in.
fn matches_line(ui: &mut Ui, model: &Model, layout: &Layout, s: &Sector, hits: Option<(u64, &[u64])>) {
    if let Some((_, hits)) = hits {
        let share = render::match_share(model, layout, s, hits);
        let size = if s.is_group() {
            s.group_size
        } else {
            model.node(s.node).metric(layout.metric)
        };
        let bytes = (share as f64 * size as f64).round() as u64;
        let (bytes, share) = (human_size(bytes), format!("{:.0}%", share * 100.0));
        ui.label(tr!(
            format!("Search matches: {bytes} ({share})"),
            format!("Совпадения поиска: {bytes} ({share})")
        ));
    }
}
