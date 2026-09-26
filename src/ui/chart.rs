//! The sunburst chart widget: caches layout + mesh, handles hover, tooltip,
//! click-to-navigate and wheel zoom.

use std::sync::Arc;

use egui::{Align2, Color32, FontId, Mesh, Sense, Shape, Stroke, Ui};

use crate::format::{human_size, thousands};
use crate::layout::{self, Layout, LayoutParams};
use crate::model::{Metric, Model};
use crate::render::{self, Palette};

/// Approximate extent of the standard Windows arrow cursor below and to the
/// right of its hot spot, in points (it scales with DPI like the UI does).
const CURSOR_SIZE: egui::Vec2 = egui::vec2(12.0, 20.0);

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
}

pub struct ChartView {
    pub params: LayoutParams,
    pub palette: Palette,
    pub zoom: f32,
    /// Node under the mouse during the last frame.
    pub hovered: Option<u32>,
    /// Node whose context menu is open (right-clicked sector or the root).
    menu_node: Option<u32>,
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
            hovered: None,
            menu_node: None,
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

    pub fn show(
        &mut self,
        ui: &mut Ui,
        model: &Model,
        root: u32,
        metric: Metric,
        external_highlight: Option<u32>,
    ) -> ChartAction {
        let size = ui.available_size();
        let (response, painter) = ui.allocate_painter(size, Sense::click());
        let rect = response.rect;

        // Wheel zoom.
        if response.hovered() {
            let dy = ui.input(|i| i.smooth_scroll_delta.y);
            if dy != 0.0 {
                self.zoom = (self.zoom * (1.0 + dy * 0.002)).clamp(0.5, 6.0);
            }
        }

        let center = rect.center();
        let radius = rect.width().min(rect.height()) * 0.5 * 0.96 * self.zoom;
        let key = CacheKey {
            root,
            metric,
            center: (center.x.round() as i32, center.y.round() as i32),
            radius: radius.round() as i32,
        };
        if self.key != Some(key) || self.layout.is_none() {
            let l = layout::build(model, root, metric, center, radius, &self.params);
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
            painter.add(Shape::closed_line(
                render::circle_points(center, r_out),
                guide,
            ));
        }

        painter.add(Shape::mesh(mesh.clone()));

        // Centre disc with root name and size.
        let r0 = layout.center_radius();
        painter.circle_filled(center, r0, Color32::from_gray(118));
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
        let pointer = response.hover_pos();
        // While a context menu is open its sector stays highlighted and the
        // tooltip is hidden, so the highlight shows what the menu acts on.
        let hit = match self.menu_node {
            Some(n) => layout.index.get(&n).copied(),
            None => pointer.and_then(|p| layout.hit_test(p)),
        };
        self.hovered = match self.menu_node {
            Some(n) => Some(n),
            None => hit.map(|(ring, i)| layout.rings[ring][i].node),
        };

        if let Some((ring, i)) = hit {
            let overlay = render::highlight_mesh(layout, ring, i, Color32::from_white_alpha(70));
            painter.add(Shape::mesh(Arc::new(overlay)));
            painter.add(Shape::closed_line(
                render::sector_outline(layout, ring, i),
                Stroke::new(1.5, Color32::from_gray(30)),
            ));
        }
        if let Some(ext) = external_highlight
            && Some(ext) != self.hovered
                && let Some(&(ring, i)) = layout.index.get(&ext) {
                    painter.add(Shape::closed_line(
                        render::sector_outline(layout, ring, i),
                        Stroke::new(2.0, Color32::from_rgb(20, 90, 200)),
                    ));
                }

        // Click handling.
        let mut action = ChartAction::None;
        // A left click while a menu is open only dismisses the menu.
        let menu_was_open = self.menu_node.is_some();
        if response.secondary_clicked() {
            // Right click: context menu for a sector, or for the current root
            // when the centre disc is clicked; nothing on empty space.
            self.menu_node = response.interact_pointer_pos().and_then(|p| {
                if layout.is_center(p) {
                    Some(root)
                } else {
                    layout.hit_test(p).map(|(ring, i)| layout.rings[ring][i].node)
                }
            });
        } else if response.clicked()
            && !menu_was_open
            && let Some(p) = response.interact_pointer_pos() {
                if layout.is_center(p) {
                    action = ChartAction::Up;
                } else if let Some((ring, i)) = layout.hit_test(p) {
                    let node = layout.rings[ring][i].node;
                    if model.node(node).is_dir {
                        action = ChartAction::Navigate(node);
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
        if let (None, Some(node), Some(p)) = (self.menu_node, self.hovered, pointer) {
            let n = model.node(node);
            let cursor_box = egui::Rect::from_min_size(p, CURSOR_SIZE);
            egui::Tooltip::always_open(
                ui.ctx().clone(),
                response.layer_id,
                response.id,
                egui::PopupAnchor::ParentRect(cursor_box),
            )
            .show(|ui| {
                ui.set_max_width(360.0);
                ui.strong(model.name(node));
                ui.label(format!(
                    "{} ({})",
                    human_size(n.size),
                    human_size(n.alloc)
                ));
                if n.is_dir {
                    ui.label(format!("Dirs: {}", thousands(n.dirs as u64)));
                    ui.label(format!("Files: {}", thousands(n.files as u64)));
                }
                ui.weak(model.path(node));
            });
        }

        action
    }
}
