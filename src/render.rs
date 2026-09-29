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
    /// Files by type: the largest types of the scan get a colour each, the
    /// rest one neutral colour; folders are grey.
    Type,
}

/// Colours are built with egui's `Hsva`, which works in linear RGB; the
/// conversion to sRGB lightens them, which gives the pastel look. There is
/// one palette per theme ([`Palette::for_theme`]): on the dark one the
/// sectors are deeper and fade towards the dark background with depth, as
/// on the light one they fade towards white.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub mode: ColorMode,
    /// Made for the dark theme.
    pub dark: bool,
    pub sat_dir: f32,
    pub sat_file: f32,
    pub val_even: f32,
    pub val_odd: f32,
    // --- Depth mode ---
    /// Hues (degrees) from the innermost ring to the outermost, spread over
    /// `rings` rings.
    pub hues: [f32; 6],
    /// Number of rings the chart is laid out with; [`build_mesh`] sets it
    /// from the layout.
    pub rings: usize,
    // --- Size mode ---
    /// Hue (degrees) of the largest sibling.
    pub hue_largest: f32,
    /// Hue (degrees) that a vanishingly small sibling approaches.
    pub hue_smallest: f32,
    /// Share of saturation lost by the outermost visible ring (0 = none).
    pub fade_max: f32,
    /// Share of brightness lost by the outermost visible ring: the fade
    /// towards a dark background.
    pub fade_val: f32,
    /// Saturation and brightness of "N smaller items" group sectors.
    pub sat_group: f32,
    pub val_group: f32,
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
    // --- Around the sectors ---
    pub background: Color32,
    /// Circles at the ring boundaries.
    pub guide: Color32,
    /// Centre disc and its two lines of text.
    pub center: Color32,
    pub center_text: Color32,
    pub center_subtext: Color32,
    /// Laid over the sector under the mouse, and its outline.
    pub hover_overlay: Color32,
    pub outline: Color32,
    /// Outline of the item hovered in the tree or a list.
    pub external: Color32,
    /// Legend of the age mode: text and the backing behind it.
    pub legend_text: Color32,
    pub legend_backing: Color32,
    // --- Type mode ---
    /// Hue (degrees) and brightness factor of each colour slot, neighbours
    /// far apart so that the largest types, which sit next to each other in
    /// the legend and often on the chart, never look alike. Twelve pastel
    /// hues alone are too close, so the last slots are darker shades.
    pub type_hues: [(f32, f32); crate::types::COLOURED],
    /// Folders, and files of a type without a colour of its own.
    pub type_folder: Color32,
    pub type_other: Color32,
}

impl Default for Palette {
    fn default() -> Self {
        Self::for_theme(false)
    }
}

impl Palette {
    /// The palette of the dark or the light theme.
    pub fn for_theme(dark: bool) -> Self {
        let light = Self::light();
        if !dark {
            return light;
        }
        Self {
            dark: true,
            sat_dir: 0.90,
            sat_file: 0.72,
            val_even: 0.62,
            val_odd: 0.45,
            // Yellow turns olive when dark: stop at gold.
            hue_smallest: 48.0,
            fade_max: 0.45,
            fade_val: 0.70,
            sat_group: 0.15,
            val_group: 0.13,
            unknown: Color32::from_gray(85),
            free: Color32::from_rgb(44, 52, 62),
            dimmed: Color32::from_gray(40),
            background: Color32::from_gray(20),
            guide: Color32::from_gray(40),
            center: Color32::from_gray(64),
            center_text: Color32::from_gray(235),
            center_subtext: Color32::from_gray(195),
            hover_overlay: Color32::from_white_alpha(45),
            outline: Color32::from_gray(235),
            external: Color32::from_rgb(110, 175, 255),
            legend_text: Color32::from_gray(205),
            legend_backing: Color32::from_black_alpha(200),
            // Darker than the centre disc, which would merge with them.
            type_folder: Color32::from_gray(46),
            type_other: Color32::from_rgb(98, 93, 86),
            ..light
        }
    }

    fn light() -> Self {
        Self {
            mode: ColorMode::default(),
            dark: false,
            sat_dir: 0.92,
            sat_file: 0.72,
            val_even: 0.92,
            val_odd: 0.78,
            hues: [0.0, 10.0, 22.0, 34.0, 44.0, 52.0],
            rings: crate::layout::DEFAULT_RINGS,
            hue_largest: 0.0,
            hue_smallest: 56.0,
            fade_max: 0.6,
            fade_val: 0.0,
            sat_group: 0.22,
            val_group: 0.78,
            hue_new: 0.0,
            hue_old: 225.0,
            age_span_days: 3650.0,
            unknown: Color32::from_gray(200),
            free: Color32::from_rgb(218, 228, 238),
            dimmed: Color32::from_gray(246),
            partial_min: 0.3,
            background: Color32::WHITE,
            guide: Color32::from_gray(225),
            center: Color32::from_gray(118),
            center_text: Color32::WHITE,
            center_subtext: Color32::from_gray(235),
            hover_overlay: Color32::from_white_alpha(70),
            outline: Color32::from_gray(30),
            external: Color32::from_rgb(20, 90, 200),
            legend_text: Color32::from_gray(80),
            legend_backing: Color32::from_white_alpha(215),
            type_hues: [
                (215.0, 1.0),
                (25.0, 1.0),
                (130.0, 1.0),
                (290.0, 1.0),
                (55.0, 1.0),
                (185.0, 1.0),
                (335.0, 1.0),
                (95.0, 1.0),
                (0.0, 0.62),
                (250.0, 0.62),
                (160.0, 0.5),
                (30.0, 0.42),
            ],
            type_folder: Color32::from_gray(212),
            type_other: Color32::from_rgb(238, 233, 224),
        }
    }

    /// Colour of a file whose type has colour slot `rank` (see
    /// [`crate::types::FileTypes::rank`]) in `ring` of `n_rings`.
    pub fn type_color(&self, rank: u8, ring: usize, n_rings: usize) -> Color32 {
        match self.type_hues.get(rank as usize) {
            Some(&(hue, dim)) => {
                let (sat, val) = self.faded(self.sat_dir, self.val_even * dim, 0.5, ring, n_rings);
                Color32::from(Hsva::new(hue / 360.0, sat, val, 1.0))
            }
            None => self.type_other,
        }
    }

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

    /// Colour of age scale position `t` with saturation `sat` and
    /// brightness `val`, for the chart and its legend.
    pub fn age_color(&self, t: f32, sat: f32, val: f32) -> Color32 {
        let hue = self.hue_new + (self.hue_old - self.hue_new) * t;
        Color32::from(Hsva::new(hue / 360.0, sat, val, 1.0))
    }

    /// How far out `ring` of `n_rings` is, from 0 (innermost) to 1.
    fn depth(ring: usize, n_rings: usize) -> f32 {
        if n_rings > 1 {
            ring.min(n_rings - 1) as f32 / (n_rings - 1) as f32
        } else {
            0.0
        }
    }

    /// Hue of `ring` in the depth mode: `hues` stretched over `rings`, so
    /// that the outermost ring gets the last one however many there are.
    fn depth_hue(&self, ring: usize) -> f32 {
        let last = self.hues.len() - 1;
        let t = Self::depth(ring, self.rings) * last as f32;
        let i = (t as usize).min(last - 1);
        let (a, b) = (self.hues[i], self.hues[i + 1]);
        a + (b - a) * (t - i as f32)
    }

    /// Saturation `sat` and brightness `val` faded towards the background
    /// by `strength` (1 = fully) for `ring` of `n_rings`.
    fn faded(&self, sat: f32, val: f32, strength: f32, ring: usize, n_rings: usize) -> (f32, f32) {
        let f = Self::depth(ring, n_rings) * strength;
        (sat * (1.0 - self.fade_max * f), val * (1.0 - self.fade_val * f))
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
            // The type is not known here; `type_color` gives files theirs.
            ColorMode::Type => {
                if is_dir {
                    self.type_folder
                } else {
                    self.type_other
                }
            }
            ColorMode::Age => match age {
                // Half the fade of the size mode: the hue carries the
                // meaning here, and faded hues are harder to tell apart.
                Some(t) => {
                    let (sat, val) = self.faded(sat, self.val_even, 0.5, ring, n_rings);
                    self.age_color(t, sat, val)
                }
                None => self.unknown,
            },
            ColorMode::Depth => {
                let hue = self.depth_hue(ring);
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
                // Linear fade with depth, like OverDisk, but towards the
                // background.
                let (sat, val) = self.faded(sat, self.val_even, 1.0, ring, n_rings);
                Color32::from(Hsva::new(hue / 360.0, sat, val, 1.0))
            }
        }
    }

    /// Colour of a group of merged small items: a muted, warm neutral that
    /// reads as "the rest" next to the coloured sectors.
    pub fn group_color(&self, ring: usize, n_rings: usize) -> Color32 {
        let (hue, sat) = match self.mode {
            ColorMode::Size => (self.hue_smallest, self.sat_group),
            ColorMode::Depth => (self.depth_hue(ring), self.sat_group),
            // A near grey: any hue would read as an age or a type.
            ColorMode::Age | ColorMode::Type => (40.0, 0.08),
        };
        let (sat, val) = self.faded(sat, self.val_group, 1.0, ring, n_rings);
        Color32::from(Hsva::new(hue / 360.0, sat, val, 1.0))
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
    let palette = &Palette {
        rings: layout.radii.len(),
        ..*palette
    };
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
            } else if palette.mode == ColorMode::Type {
                let n = model.node(s.node);
                if n.is_dir {
                    palette.type_folder
                } else {
                    let rank = model.types.rank(model.types.of(s.node));
                    palette.type_color(rank, ring, n_rings)
                }
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
    fn depth_hues_span_the_rings_however_many() {
        for rings in 1..=crate::layout::MAX_RINGS {
            let p = Palette { rings, ..Palette::default() };
            let hues: Vec<f32> = (0..rings).map(|r| p.depth_hue(r)).collect();
            assert_eq!(hues[0], p.hues[0]);
            if rings > 1 {
                assert!((hues[rings - 1] - p.hues[p.hues.len() - 1]).abs() < 1e-3, "{rings}: {hues:?}");
            }
            assert!(hues.windows(2).all(|w| w[1] > w[0]), "{rings}: {hues:?}");
        }
        // With as many rings as hues, each ring gets its own.
        let p = Palette { rings: 6, ..Palette::default() };
        for (r, &h) in p.hues.iter().enumerate() {
            assert!((p.depth_hue(r) - h).abs() < 1e-3);
        }
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

    #[test]
    fn dark_palette_is_deeper_and_fades_towards_the_background() {
        let (light, dark) = (Palette::for_theme(false), Palette::for_theme(true));
        assert!(!light.dark && dark.dark);
        let lum = |c: Color32| c.r() as u32 + c.g() as u32 + c.b() as u32;
        for mode in [ColorMode::Size, ColorMode::Depth, ColorMode::Age] {
            let (l, d) = (Palette { mode, ..light }, Palette { mode, ..dark });
            let (cl, cd) = (l.color(0, 0, 4, 1.0, Some(0.3), true), d.color(0, 0, 4, 1.0, Some(0.3), true));
            assert!(lum(cd) + 20 < lum(cl), "{mode:?}: dark colours are deeper");
            assert!(lum(cd) > lum(dark.background) + 150, "{mode:?}: but stand out from the background");
        }
        // Further out the dark palette gets darker, the light one lighter.
        let (inner, outer) = (dark.color(0, 0, 4, 1.0, None, true), dark.color(3, 0, 4, 1.0, None, true));
        assert!(lum(outer) < lum(inner));
        let (inner, outer) = (light.color(0, 0, 4, 1.0, None, true), light.color(3, 0, 4, 1.0, None, true));
        assert!(lum(outer) > lum(inner));
        // Search: non-matches sink into the background.
        assert!(lum(dark.dimmed) < lum(dark.color(2, 0, 4, 0.1, None, false)));
    }

    #[test]
    fn type_colours_are_distinct_and_the_rest_neutral() {
        for dark in [false, true] {
            let p = Palette {
                mode: ColorMode::Type,
                ..Palette::for_theme(dark)
            };
            let colours: Vec<Color32> = (0..crate::types::COLOURED as u8).map(|r| p.type_color(r, 0, 4)).collect();
            for (i, a) in colours.iter().enumerate() {
                for b in &colours[i + 1..] {
                    let d = (a.r() as i32 - b.r() as i32).abs()
                        + (a.g() as i32 - b.g() as i32).abs()
                        + (a.b() as i32 - b.b() as i32).abs();
                    assert!(d > 30, "dark {dark}: {a:?} and {b:?} look alike");
                }
            }
            assert_eq!(p.type_color(crate::types::OTHER, 0, 4), p.type_other);
            assert_eq!(p.color(0, 0, 4, 1.0, None, true), p.type_folder);
            assert_ne!(p.type_folder, p.type_other);
        }
    }
}
