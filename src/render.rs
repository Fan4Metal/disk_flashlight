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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorMode {
    /// Hue from the size relative to the largest sibling (red = largest,
    /// yellow = small), faded towards white with depth. OverDisk's
    /// "size-encoded" scheme, which fades towards grey instead.
    #[default]
    Size,
    /// Hue from the ring (red core to yellow rim), brightness alternating
    /// between neighbours.
    Depth,
    /// Hue from the last write time (red = just now, blue = ten years or
    /// more), on a logarithmic scale; folders by the newest item inside.
    Age,
}

/// Colours are built with egui's `Hsva`, which works in linear RGB; the
/// conversion to sRGB lightens them, which gives the pastel look.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub mode: ColorMode,
    pub sat_dir: f32,
    pub sat_file: f32,
    pub val_even: f32,
    pub val_odd: f32,
    // --- Depth mode ---
    /// Hue (degrees) for each ring, innermost first; the last value repeats.
    pub hues: [f32; 6],
    // --- Size mode ---
    /// Hue (degrees) of the largest sibling.
    pub hue_largest: f32,
    /// Hue (degrees) that a vanishingly small sibling approaches.
    pub hue_smallest: f32,
    /// Share of saturation lost by the outermost visible ring (0 = none).
    pub fade_max: f32,
    /// Saturation of "N smaller items" group sectors.
    pub sat_group: f32,
    // --- Age mode ---
    /// Hue (degrees) of an item modified just before the scan.
    pub hue_new: f32,
    /// Hue (degrees) of an item `age_span_days` old or older.
    pub hue_old: f32,
    pub age_span_days: f32,
    /// Items whose time is unknown.
    pub unknown: Color32,
    /// Free space of a drive: a pale cool grey, apart from the warm palette.
    pub free: Color32,
    // --- Search highlight ---
    /// Colour of every sector without matches, one flat grey so that the
    /// matches stand out.
    pub dimmed: Color32,
    /// How much colour a folder with a sliver of matches gets, so that it
    /// stands apart from folders without any.
    pub partial_min: f32,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            mode: ColorMode::default(),
            sat_dir: 0.92,
            sat_file: 0.72,
            val_even: 0.92,
            val_odd: 0.78,
            hues: [0.0, 10.0, 22.0, 34.0, 44.0, 52.0],
            hue_largest: 0.0,
            hue_smallest: 56.0,
            fade_max: 0.6,
            sat_group: 0.22,
            hue_new: 0.0,
            hue_old: 225.0,
            age_span_days: 3650.0,
            unknown: Color32::from_gray(200),
            free: Color32::from_rgb(218, 228, 238),
            dimmed: Color32::from_gray(246),
            partial_min: 0.3,
        }
    }
}

impl Palette {
    /// Position of an item modified at `modified` on the age scale, from 0
    /// (at `scanned_at`) to 1 (`age_span_days` before it or earlier);
    /// `None` when the time is unknown.
    pub fn age_t(&self, scanned_at: u32, modified: u32) -> Option<f32> {
        (modified != 0).then(|| self.age_t_of_days(scanned_at.saturating_sub(modified) as f32 / 86_400.0))
    }

    /// Scale position (see [`Self::age_t`]) of an age of `days`.
    pub fn age_t_of_days(&self, days: f32) -> f32 {
        (days.ln_1p() / self.age_span_days.ln_1p()).clamp(0.0, 1.0)
    }

    /// Colour of age scale position `t` with saturation `sat`, for the
    /// chart and its legend.
    pub fn age_color(&self, t: f32, sat: f32) -> Color32 {
        let hue = self.hue_new + (self.hue_old - self.hue_new) * t;
        Color32::from(Hsva::new(hue / 360.0, sat, self.val_even, 1.0))
    }

    /// Share of saturation lost towards white by `ring` of `n_rings`.
    fn fade(&self, ring: usize, n_rings: usize) -> f32 {
        if n_rings > 1 {
            self.fade_max * ring.min(n_rings - 1) as f32 / (n_rings - 1) as f32
        } else {
            0.0
        }
    }

    /// Colour of the `index`-th sector of `ring` (0 = innermost) when
    /// `n_rings` rings are shown; `rel` is its size relative to the largest
    /// sibling (see [`Sector::rel`]), `age` its [`Self::age_t`], used in the
    /// age mode only.
    pub fn color(
        &self,
        ring: usize,
        index: usize,
        n_rings: usize,
        rel: f32,
        age: Option<f32>,
        is_dir: bool,
    ) -> Color32 {
        let sat = if is_dir { self.sat_dir } else { self.sat_file };
        match self.mode {
            ColorMode::Age => match age {
                // Half the fade of the size mode: the hue carries the
                // meaning here, and pale hues are harder to tell apart.
                Some(t) => self.age_color(t, sat * (1.0 - 0.5 * self.fade(ring, n_rings))),
                None => self.unknown,
            },
            ColorMode::Depth => {
                let hue = self.hues[ring.min(self.hues.len() - 1)];
                let val = if index.is_multiple_of(2) {
                    self.val_even
                } else {
                    self.val_odd
                };
                Color32::from(Hsva::new(hue / 360.0, sat, val, 1.0))
            }
            ColorMode::Size => {
                let rel = rel.clamp(0.0, 1.0);
                let hue = self.hue_smallest + (self.hue_largest - self.hue_smallest) * rel;
                // Linear fade with depth, like OverDisk, but towards white.
                let fade = self.fade(ring, n_rings);
                Color32::from(Hsva::new(hue / 360.0, sat * (1.0 - fade), self.val_even, 1.0))
            }
        }
    }

    /// Colour of a group of merged small items: a muted, warm neutral that
    /// reads as "the rest" next to the coloured sectors.
    pub fn group_color(&self, ring: usize, n_rings: usize) -> Color32 {
        let (hue, sat) = match self.mode {
            ColorMode::Size => (self.hue_smallest, self.sat_group),
            ColorMode::Depth => (self.hues[ring.min(self.hues.len() - 1)], self.sat_group),
            // A near grey: any hue would read as an age.
            ColorMode::Age => (40.0, 0.08),
        };
        let fade = self.fade(ring, n_rings);
        Color32::from(Hsva::new(hue / 360.0, sat * (1.0 - fade), self.val_odd, 1.0))
    }

    /// `color` of a sector while search matches are highlighted; `share` is
    /// the part of it (by size) that matches. Matches keep their colour,
    /// sectors without any are `dimmed`, and folders holding some are in
    /// between.
    pub fn highlight(&self, color: Color32, share: f32) -> Color32 {
        if share <= 0.0 {
            self.dimmed
        } else {
            let t = self.partial_min + (1.0 - self.partial_min) * share.min(1.0);
            self.dimmed.lerp_to_gamma(color, t)
        }
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

/// Screen-space margin kept around the view when clipping arcs, so that
/// clipped ends (and their outlines) stay off screen.
const CLIP_MARGIN_PX: f32 = 16.0;

/// Visible pieces of `[a0, a1]` at radius `r`. At high zoom only a small
/// part of a large sector is on screen; tessellating just that part keeps
/// the vertex count bounded by the window size.
#[inline]
fn visible_pieces(layout: &Layout, a0: f32, a1: f32, r: f32) -> impl Iterator<Item = (f32, f32)> {
    let margin = CLIP_MARGIN_PX / r.max(1.0);
    layout.view.clip(a0, a1, margin).into_iter().flatten()
}

/// Part of sector `s` (by size) covered by search matches, from
/// [`crate::model::Found::hits`].
pub fn match_share(model: &Model, layout: &Layout, s: &Sector, hits: &[u64]) -> f32 {
    let (hit, size) = if s.is_free() {
        (0, 0)
    } else if s.is_group() {
        let hit = model
            .children(s.node)
            .filter(|&c| s.group_has(model, layout.metric, c))
            .map(|c| hits[c as usize])
            .sum();
        (hit, s.group_size)
    } else {
        (hits[s.node as usize], model.node(s.node).metric(layout.metric))
    };
    if size == 0 {
        0.0
    } else {
        (hit as f64 / size as f64) as f32
    }
}

/// Build the full chart mesh. One draw call regardless of sector count.
/// With `hits` (search matches), sectors are coloured by how much of them
/// matches ([`Palette::highlight`]).
pub fn build_mesh(model: &Model, layout: &Layout, palette: &Palette, hits: Option<&[u64]>) -> Mesh {
    let mut mesh = Mesh::default();
    let approx_vertices: usize = layout
        .rings
        .iter()
        .map(|r| r.len() * 8)
        .sum();
    mesh.vertices.reserve(approx_vertices);
    mesh.indices.reserve(approx_vertices * 3);
    let n_rings = layout.rings.iter().filter(|r| !r.is_empty()).count();
    for (ring, sectors) in layout.rings.iter().enumerate() {
        for (i, s) in sectors.iter().enumerate() {
            let (r_in, r_out) = layout.sector_radii(ring, s);
            let color = if s.is_free() {
                palette.free
            } else if s.is_group() && hits.is_some() {
                // The muted group colour would read as dimmed: colour a group
                // holding matches like a small item instead.
                palette.color(ring, i, n_rings, 0.0, None, false)
            } else if s.is_group() {
                palette.group_color(ring, n_rings)
            } else {
                let n = model.node(s.node);
                let age = if palette.mode == ColorMode::Age {
                    palette.age_t(model.scanned_at, n.modified)
                } else {
                    None
                };
                palette.color(ring, i, n_rings, s.rel, age, n.is_dir)
            };
            let color = match hits {
                Some(hits) => palette.highlight(color, match_share(model, layout, s, hits)),
                None => color,
            };
            let (a0, a1) = with_gap(s, r_out);
            for (p0, p1) in visible_pieces(layout, a0, a1, r_out) {
                push_sector(&mut mesh, layout.center, r_in, r_out, p0, p1, color);
            }
        }
    }
    mesh
}

/// Filled centre disc, tessellated like the sectors so it stays round at
/// any zoom (egui's own circles use a fixed number of segments).
pub fn disc_mesh(layout: &Layout, color: Color32) -> Mesh {
    let mut mesh = Mesh::default();
    let r = layout.center_radius();
    if layout.view.radial(0.0, r) {
        for (p0, p1) in visible_pieces(layout, 0.0, TAU, r) {
            push_sector(&mut mesh, layout.center, 0.0, r, p0, p1, color);
        }
    }
    mesh
}

/// Semi-transparent overlay mesh for one highlighted sector.
pub fn highlight_mesh(layout: &Layout, ring: usize, idx: usize, color: Color32) -> Mesh {
    let mut mesh = Mesh::default();
    let s = &layout.rings[ring][idx];
    let (r_in, r_out) = layout.sector_radii(ring, s);
    let (a0, a1) = with_gap(s, r_out);
    for (p0, p1) in visible_pieces(layout, a0, a1, r_out) {
        push_sector(&mut mesh, layout.center, r_in, r_out, p0, p1, color);
    }
    mesh
}

/// Closed polylines around the visible parts of one sector, for stroking an
/// outline.
pub fn sector_outline(layout: &Layout, ring: usize, idx: usize) -> Vec<Vec<Pos2>> {
    let s = &layout.rings[ring][idx];
    let (r_in, r_out) = layout.sector_radii(ring, s);
    let (a0, a1) = with_gap(s, r_out);
    visible_pieces(layout, a0, a1, r_out)
        .map(|(p0, p1)| {
            let span = p1 - p0;
            let segments = ((span * r_out) / SEGMENT_PX).ceil().max(1.0) as usize;
            let mut pts = Vec::with_capacity(2 * (segments + 1));
            for i in 0..=segments {
                pts.push(point(layout.center, r_out, p0 + span * i as f32 / segments as f32));
            }
            for i in (0..=segments).rev() {
                pts.push(point(layout.center, r_in, p0 + span * i as f32 / segments as f32));
            }
            pts
        })
        .collect()
}

/// Visible parts of the guide circle at radius `r`, as open polylines.
pub fn guide_arcs(layout: &Layout, r: f32) -> Vec<Vec<Pos2>> {
    if !layout.view.radial(r, r) {
        return Vec::new();
    }
    visible_pieces(layout, 0.0, TAU, r)
        .map(|(p0, p1)| {
            let span = p1 - p0;
            let segments = ((span * r) / (SEGMENT_PX * 2.0)).ceil().max(8.0) as usize;
            (0..=segments)
                .map(|i| point(layout.center, r, p0 + span * i as f32 / segments as f32))
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(c: Color32) -> (i32, i32, i32) {
        (c.r() as i32, c.g() as i32, c.b() as i32)
    }

    #[test]
    fn size_mode_hue_follows_relative_size_and_fades_with_depth() {
        let p = Palette::default();
        assert_eq!(p.mode, ColorMode::Size);
        let (r, g, _) = rgb(p.color(0, 0, 4, 1.0, None, true));
        assert!(r > 200 && g < r / 2, "largest sibling should be red");
        let (r2, g2, _) = rgb(p.color(0, 1, 4, 0.05, None, true));
        assert!(g2 > g + 60 && r2 > 200, "small sibling should be yellowish");
        // Same relative size further out: lighter (closer to white).
        let (_, g_in, b_in) = rgb(p.color(0, 0, 4, 1.0, None, true));
        let (_, g_out, b_out) = rgb(p.color(3, 0, 4, 1.0, None, true));
        assert!(g_out > g_in && b_out > b_in, "outer ring should be paler");
        // Neighbours of equal size get the same colour (no alternation).
        assert_eq!(p.color(1, 0, 4, 0.5, None, true), p.color(1, 1, 4, 0.5, None, true));
    }

    #[test]
    fn highlight_fades_by_matching_share() {
        let p = Palette::default();
        let c = p.color(0, 0, 3, 1.0, None, true);
        assert_eq!(p.highlight(c, 1.0), c);
        let (none, some, most) = (p.highlight(c, 0.0), p.highlight(c, 0.01), p.highlight(c, 0.8));
        // Red fading towards grey gains green: the less matches, the paler.
        assert!(none.g() > some.g() + 10 && some.g() > most.g() && most.g() > c.g());
        // Without matches every colour turns into the same grey.
        assert_eq!(none, p.dimmed);
        assert_eq!(p.highlight(p.color(2, 1, 3, 0.1, None, false), 0.0), p.dimmed);
    }

    #[test]
    fn depth_mode_alternates_brightness() {
        let p = Palette {
            mode: ColorMode::Depth,
            ..Palette::default()
        };
        assert_ne!(p.color(0, 0, 3, 1.0, None, true), p.color(0, 1, 3, 1.0, None, true));
        assert_eq!(p.color(0, 0, 3, 1.0, None, true), p.color(0, 0, 3, 0.1, None, true));
    }

    #[test]
    fn age_mode_runs_from_red_to_blue() {
        let p = Palette {
            mode: ColorMode::Age,
            ..Palette::default()
        };
        const DAY: u32 = 86_400;
        let now = 20_000 * DAY;
        assert_eq!(p.age_t(now, now), Some(0.0));
        assert_eq!(p.age_t(now, 0), None);
        assert_eq!(p.age_t(now, now + DAY), Some(0.0), "a time after the scan is new");
        assert_eq!(p.age_t(now, now - 5000 * DAY), Some(1.0));
        let year = p.age_t(now, now - 365 * DAY).unwrap();
        let month = p.age_t(now, now - 30 * DAY).unwrap();
        assert!(0.0 < month && month < year && year < 1.0);
        assert_eq!(p.age_t_of_days(365.0), year);

        let (r, _, b) = rgb(p.color(0, 0, 3, 1.0, Some(0.0), false));
        assert!(r > 200 && b + 80 < r, "new should be red");
        let (r, _, b) = rgb(p.color(0, 0, 3, 1.0, Some(1.0), false));
        assert!(b > 200 && r + 60 < b, "old should be blue");
        assert_eq!(p.color(0, 0, 3, 1.0, None, true), p.unknown);
        // Size and position do not matter, only the age.
        assert_eq!(p.color(1, 0, 3, 1.0, Some(0.5), true), p.color(1, 1, 3, 0.1, Some(0.5), true));
    }
}
