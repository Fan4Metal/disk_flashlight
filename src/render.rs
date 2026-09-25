//! Tessellation of the sunburst layout into a single GPU mesh, plus the
//! colour palette and helper shapes for hover highlighting.

use std::f32::consts::TAU;

use egui::ecolor::Hsva;
use egui::{Color32, Mesh, Pos2, Vec2};

use crate::layout::{Layout, Sector};
use crate::model::Model;

/// Target arc length per tessellation segment, in pixels.
const SEGMENT_PX: f32 = 4.0;
/// Angular gap between neighbouring sectors, in pixels at the outer radius.
const GAP_PX: f32 = 0.8;

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    /// Hue (degrees) for each ring, innermost first; the last value repeats.
    pub hues: [f32; 6],
    pub sat_dir: f32,
    pub sat_file: f32,
    pub val_even: f32,
    pub val_odd: f32,
}

impl Default for Palette {
    fn default() -> Self {
        // Red core fading to orange and yellow towards the rim, as in OverDisk.
        Self {
            hues: [0.0, 10.0, 22.0, 34.0, 44.0, 52.0],
            sat_dir: 0.92,
            sat_file: 0.72,
            val_even: 0.92,
            val_odd: 0.78,
        }
    }
}

impl Palette {
    pub fn color(&self, ring: usize, index: usize, is_dir: bool) -> Color32 {
        let hue = self.hues[ring.min(self.hues.len() - 1)] / 360.0;
        let sat = if is_dir { self.sat_dir } else { self.sat_file };
        let val = if index.is_multiple_of(2) {
            self.val_even
        } else {
            self.val_odd
        };
        Color32::from(Hsva::new(hue, sat, val, 1.0))
    }
}

#[inline]
fn point(center: Pos2, r: f32, angle: f32) -> Pos2 {
    // angle 0 = top, clockwise; screen y grows downward.
    let (s, c) = angle.sin_cos();
    center + Vec2::new(r * s, -r * c)
}

/// Append one annular sector to `mesh`.
fn push_sector(mesh: &mut Mesh, center: Pos2, r_in: f32, r_out: f32, a0: f32, a1: f32, color: Color32) {
    let span = a1 - a0;
    if span <= 0.0 {
        return;
    }
    let segments = ((span * r_out) / SEGMENT_PX).ceil().max(1.0) as u32;
    let base = mesh.vertices.len() as u32;
    mesh.vertices.reserve(2 * (segments as usize + 1));
    mesh.indices.reserve(6 * segments as usize);
    for i in 0..=segments {
        let a = a0 + span * (i as f32 / segments as f32);
        mesh.colored_vertex(point(center, r_in, a), color);
        mesh.colored_vertex(point(center, r_out, a), color);
    }
    for i in 0..segments {
        let i0 = base + 2 * i;
        mesh.add_triangle(i0, i0 + 1, i0 + 2);
        mesh.add_triangle(i0 + 1, i0 + 3, i0 + 2);
    }
}

/// Trim a sector's angles to leave a visual gap to its neighbours. Very thin
/// sectors keep their full span so they do not vanish.
#[inline]
fn with_gap(s: &Sector, r_out: f32) -> (f32, f32) {
    let gap = GAP_PX / r_out;
    let span = s.a1 - s.a0;
    if span > 3.0 * gap {
        (s.a0 + gap * 0.5, s.a1 - gap * 0.5)
    } else {
        (s.a0, s.a1)
    }
}

/// Build the full chart mesh. One draw call regardless of sector count.
pub fn build_mesh(model: &Model, layout: &Layout, palette: &Palette) -> Mesh {
    let mut mesh = Mesh::default();
    let approx_vertices: usize = layout
        .rings
        .iter()
        .map(|r| r.len() * 8)
        .sum();
    mesh.vertices.reserve(approx_vertices);
    mesh.indices.reserve(approx_vertices * 3);
    for (ring, sectors) in layout.rings.iter().enumerate() {
        let (r_in, r_out) = layout.radii[ring];
        for (i, s) in sectors.iter().enumerate() {
            let is_dir = model.node(s.node).is_dir;
            let color = palette.color(ring, i, is_dir);
            let (a0, a1) = with_gap(s, r_out);
            push_sector(&mut mesh, layout.center, r_in, r_out, a0, a1, color);
        }
    }
    mesh
}

/// Semi-transparent overlay mesh for one highlighted sector.
pub fn highlight_mesh(layout: &Layout, ring: usize, idx: usize, color: Color32) -> Mesh {
    let mut mesh = Mesh::default();
    let (r_in, r_out) = layout.radii[ring];
    let s = &layout.rings[ring][idx];
    let (a0, a1) = with_gap(s, r_out);
    push_sector(&mut mesh, layout.center, r_in, r_out, a0, a1, color);
    mesh
}

/// Closed polyline around one sector, for stroking an outline.
pub fn sector_outline(layout: &Layout, ring: usize, idx: usize) -> Vec<Pos2> {
    let (r_in, r_out) = layout.radii[ring];
    let s = &layout.rings[ring][idx];
    let (a0, a1) = with_gap(s, r_out);
    let span = a1 - a0;
    let segments = ((span * r_out) / SEGMENT_PX).ceil().max(1.0) as usize;
    let mut pts = Vec::with_capacity(2 * (segments + 1));
    for i in 0..=segments {
        pts.push(point(layout.center, r_out, a0 + span * i as f32 / segments as f32));
    }
    for i in (0..=segments).rev() {
        pts.push(point(layout.center, r_in, a0 + span * i as f32 / segments as f32));
    }
    pts
}

/// Full circle outline at radius `r` (guide rings behind the chart).
pub fn circle_points(center: Pos2, r: f32) -> Vec<Pos2> {
    let segments = ((TAU * r) / (SEGMENT_PX * 2.0)).ceil().max(16.0) as usize;
    (0..segments)
        .map(|i| point(center, r, TAU * i as f32 / segments as f32))
        .collect()
}
