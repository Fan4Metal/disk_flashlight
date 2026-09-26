//! The sunburst chart widget: caches layout + mesh, handles hover, tooltip,
//! click-to-navigate, zoom towards the cursor and panning.

use std::sync::Arc;

use egui::{Align2, Color32, FontId, Mesh, PointerButton, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};

use crate::format::{human_size, thousands};
use crate::layout::{self, Layout, LayoutParams, Sector};
use crate::model::{Metric, Model};
use crate::render::{self, Palette};
use crate::scan::win::DiskSpace;

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
    OpenInExplorer(u32),
    Properties(u32),
}

#[derive(PartialEq, Clone, Copy)]
struct CacheKey {
    root: u32,
    metric: Metric,
    center: (i32, i32),
    radius: i32,
    view: (i32, i32, i32, i32),
    free: u64,
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

    pub fn show(
        &mut self,
        ui: &mut Ui,
        model: &Model,
        root: u32,
        metric: Metric,
        external_highlight: Option<u32>,
        disk: Option<DiskSpace>,
    ) -> ChartAction {
        let size = ui.available_size();
        let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
        let rect = response.rect;

        if self.view_root != Some(root) {
            self.view_root = Some(root);
            self.reset_view();
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
            let m = render::build_mesh(model, &l, &self.palette);
            self.layout = Some(l);
            self.mesh = Some(Arc::new(m));
            self.key = Some(key);
        }
        let layout = self.layout.as_ref().expect("layout built above");
        let mesh = self.mesh.as_ref().expect("mesh built above");

        // Background and guide rings.
        let painter = painter.with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, Color32::WHITE);
        let guide = Stroke::new(1.0, Color32::from_gray(225));
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
            Color32::from_gray(118),
        ))));
        let root_node = model.node(root);
        let title = model.name(root);
        let font = FontId::proportional((r0 * 0.34).clamp(12.0, 30.0));
        painter.text(
            center - egui::vec2(0.0, font.size * 0.55),
            Align2::CENTER_CENTER,
            title,
            font.clone(),
            Color32::WHITE,
        );
        painter.text(
            center + egui::vec2(0.0, font.size * 0.55),
            Align2::CENTER_CENTER,
            human_size(root_node.metric(metric)),
            FontId::proportional((font.size * 0.7).max(11.0)),
            Color32::from_gray(235),
        );

        // Hover / highlight.
        let panning = response.dragged_by(PointerButton::Middle);
        let pointer = response.hover_pos().filter(|_| !panning);
        // While a context menu is open its sector stays highlighted and the
        // tooltip is hidden, so the highlight shows what the menu acts on.
        let hit = match self.menu_node {
            Some(n) => layout.index.get(&n).copied(),
            None => pointer.and_then(|p| layout.hit_test(p)),
        };
        let hit_sector: Option<Sector> = hit.map(|(ring, i)| layout.rings[ring][i]);
        self.hovered = match self.menu_node {
            Some(n) => Some(n),
            None => hit_sector.filter(|s| s.is_item()).map(|s| s.node),
        };

        if let Some((ring, i)) = hit {
            let overlay = render::highlight_mesh(layout, ring, i, Color32::from_white_alpha(70));
            painter.add(Shape::mesh(Arc::new(overlay)));
            for outline in render::sector_outline(layout, ring, i) {
                painter.add(Shape::closed_line(outline, Stroke::new(1.5, Color32::from_gray(30))));
            }
        }
        if let Some(ext) = external_highlight
            && Some(ext) != self.hovered
            && let Some(&(ring, i)) = layout.index.get(&ext)
        {
            for outline in render::sector_outline(layout, ring, i) {
                painter.add(Shape::closed_line(
                    outline,
                    Stroke::new(2.0, Color32::from_rgb(20, 90, 200)),
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
                } else if s.is_item() && model.node(s.node).is_dir {
                    action = ChartAction::Navigate(s.node);
                }
            }
        }

        if let Some(node) = self.menu_node {
            // Popup::context_menu opens on this frame's secondary click and
            // returns None once the menu has been closed.
            let shown = response.context_menu(|ui| {
                if ui.button("Open in Explorer").clicked() {
                    action = ChartAction::OpenInExplorer(node);
                    ui.close();
                }
                if ui.button("Properties").clicked() {
                    action = ChartAction::Properties(node);
                    ui.close();
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
        if let (None, Some(s), Some(p)) = (self.menu_node, hit_sector, pointer) {
            let cursor_box = egui::Rect::from_min_size(p, CURSOR_SIZE);
            egui::Tooltip::always_open(
                ui.ctx().clone(),
                response.layer_id,
                response.id,
                egui::PopupAnchor::ParentRect(cursor_box),
            )
            .show(|ui| {
                ui.set_max_width(360.0);
                if s.is_free() {
                    ui.strong("Free space");
                    let total = disk.map_or(0, |d| d.total);
                    let share = s.group_size as f64 / total.max(1) as f64 * 100.0;
                    ui.label(format!("{} ({share:.0}% of {})", human_size(s.group_size), human_size(total)));
                } else if s.is_group() {
                    ui.strong(format!("{} smaller items", thousands(s.count as u64)));
                    ui.label(human_size(s.group_size));
                    ui.weak(format!("in {}", model.path(s.node)));
                    ui.weak("Click or scroll to zoom in");
                } else {
                    let n = model.node(s.node);
                    ui.strong(model.name(s.node));
                    ui.label(format!(
                        "{} ({})",
                        human_size(n.size),
                        human_size(n.alloc)
                    ));
                    if n.is_dir {
                        ui.label(format!("Dirs: {}", thousands(n.dirs as u64)));
                        ui.label(format!("Files: {}", thousands(n.files as u64)));
                    }
                    ui.weak(model.path(s.node));
                }
            });
        }

        if let Some(p) = zoom_to {
            self.zoom_at(rect, p, GROUP_CLICK_ZOOM);
            ui.ctx().request_repaint();
        }
        action
    }
}
