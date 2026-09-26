//! Sunburst layout: turns a subtree into rings of angular sectors.
//!
//! Children are visited largest first. Items whose arc would be thinner than
//! `merge_arc_px` are not drawn one by one: all of them are merged into a
//! single "N smaller items" group sector. Zooming in makes arcs longer, so
//! items leave the group one by one as they become wide enough. Sectors that
//! fall outside the visible rectangle are culled (and their subtrees are not
//! visited), so the work stays bounded by what is on screen even at high
//! zoom, not by the size of the tree.

use std::collections::HashMap;
use std::f32::consts::{PI, TAU};

use egui::{Pos2, Rect};

use crate::model::{Metric, Model};

#[derive(Clone, Copy, Debug)]
pub struct Sector {
    /// The item itself, or for a group the directory whose small children
    /// were merged.
    pub node: u32,
    /// Start angle in radians, 0 = top, increasing clockwise.
    pub a0: f32,
    pub a1: f32,
    /// Size relative to the largest sibling (1.0 for the largest), used by
    /// the size-encoded palette.
    pub rel: f32,
    /// 0 for a single item; otherwise the number of merged small items.
    pub count: u32,
    /// Total size (in the layout metric) of the merged items of a group.
    pub group_size: u64,
}

impl Sector {
    #[inline]
    pub fn is_group(&self) -> bool {
        self.count > 0
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LayoutParams {
    pub max_depth: usize,
    /// Items with a thinner arc (at the ring's outer edge) are merged into a
    /// group sector.
    pub merge_arc_px: f32,
    /// Groups thinner than this are not drawn at all.
    pub min_arc_px: f32,
    /// Radius of the central disc as a fraction of the outer radius.
    pub center_frac: f32,
    /// Each ring is this much thinner than the previous one.
    pub ring_shrink: f32,
}

impl Default for LayoutParams {
    fn default() -> Self {
        Self {
            max_depth: 7,
            merge_arc_px: 4.0,
            min_arc_px: 1.0,
            center_frac: 0.24,
            ring_shrink: 0.78,
        }
    }
}

/// The visible rectangle expressed in the chart's polar coordinates.
#[derive(Clone, Copy, Debug)]
pub struct ViewBounds {
    /// Distance from the centre to the nearest visible point.
    pub r_min: f32,
    /// Distance from the centre to the farthest visible point.
    pub r_max: f32,
    /// Visible angles as `(start, length)`, `None` when the centre is inside
    /// the view (all angles visible). `length` is at most PI.
    pub arc: Option<(f32, f32)>,
}

impl ViewBounds {
    pub fn new(center: Pos2, view: Rect) -> Self {
        let corners = [
            view.left_top(),
            view.right_top(),
            view.right_bottom(),
            view.left_bottom(),
        ];
        let r_max = corners
            .iter()
            .map(|c| (*c - center).length())
            .fold(0.0, f32::max);
        if view.contains(center) {
            return Self {
                r_min: 0.0,
                r_max,
                arc: None,
            };
        }
        let r_min = view.distance_to_pos(center);
        // The view does not contain the centre, so it subtends less than a
        // half turn: measure corner angles relative to the view's direction.
        let mid = polar_angle(view.center() - center);
        let (mut lo, mut hi) = (0.0f32, 0.0f32);
        for c in corners {
            let mut d = polar_angle(c - center) - mid;
            if d > PI {
                d -= TAU;
            } else if d < -PI {
                d += TAU;
            }
            lo = lo.min(d);
            hi = hi.max(d);
        }
        let start = (mid + lo).rem_euclid(TAU);
        Self {
            r_min,
            r_max,
            arc: Some((start, hi - lo)),
        }
    }

    /// Whether the annulus between `r_in` and `r_out` can be seen at all.
    #[inline]
    pub fn radial(&self, r_in: f32, r_out: f32) -> bool {
        r_out >= self.r_min && r_in <= self.r_max
    }

    /// Parts of `[a0, a1]` (within `[0, TAU]`) that are visible, widened by
    /// `margin` radians. At most two pieces (when the view straddles 0).
    pub fn clip(&self, a0: f32, a1: f32, margin: f32) -> [Option<(f32, f32)>; 2] {
        let Some((start, len)) = self.arc else {
            return [Some((a0, a1)), None];
        };
        let (vs, ve) = (start - margin, start + len + margin);
        let mut out = [None, None];
        let mut k = 0;
        for shift in [-TAU, 0.0, TAU] {
            let lo = a0.max(vs + shift);
            let hi = a1.min(ve + shift);
            if lo < hi && k < 2 {
                out[k] = Some((lo, hi));
                k += 1;
            }
        }
        out
    }

    #[inline]
    pub fn angular(&self, a0: f32, a1: f32) -> bool {
        self.clip(a0, a1, 0.0)[0].is_some()
    }
}

/// Angle of a vector in `[0, TAU)`: 0 = up, clockwise (screen y downward).
#[inline]
pub fn polar_angle(d: egui::Vec2) -> f32 {
    d.x.atan2(-d.y).rem_euclid(TAU)
}

#[derive(Debug)]
pub struct Layout {
    pub metric: Metric,
    pub center: Pos2,
    pub outer_radius: f32,
    /// `(inner, outer)` radius per ring; ring 0 = children of root.
    pub radii: Vec<(f32, f32)>,
    /// Sectors per ring, sorted by angle.
    pub rings: Vec<Vec<Sector>>,
    /// node id -> (ring, index) for every emitted single-item sector.
    pub index: HashMap<u32, (usize, usize)>,
    /// What part of the chart is on screen; used for culling and clipping.
    pub view: ViewBounds,
}

impl Layout {
    pub fn center_radius(&self) -> f32 {
        self.radii.first().map(|r| r.0).unwrap_or(self.outer_radius)
    }

    pub fn sector_count(&self) -> usize {
        self.rings.iter().map(Vec::len).sum()
    }

    /// Angle in `[0, TAU)` and radius of a screen position.
    pub fn polar(&self, p: Pos2) -> (f32, f32) {
        let d = p - self.center;
        (polar_angle(d), d.length())
    }

    /// Sector under a screen position, if any.
    pub fn hit_test(&self, p: Pos2) -> Option<(usize, usize)> {
        let (angle, r) = self.polar(p);
        let ring = self.radii.iter().position(|&(ri, ro)| r >= ri && r < ro)?;
        let sectors = &self.rings[ring];
        // First sector whose a0 > angle, minus one.
        let i = sectors.partition_point(|s| s.a0 <= angle);
        if i == 0 {
            return None;
        }
        let s = &sectors[i - 1];
        (angle < s.a1).then_some((ring, i - 1))
    }

    pub fn is_center(&self, p: Pos2) -> bool {
        (p - self.center).length() < self.center_radius()
    }
}

/// Lay out the subtree of `root` as a chart of `outer_radius` around
/// `center`; only the part inside `view` (screen coordinates) is produced.
pub fn build(
    model: &Model,
    root: u32,
    metric: Metric,
    center: Pos2,
    outer_radius: f32,
    view: Rect,
    params: &LayoutParams,
) -> Layout {
    let depth = params.max_depth.max(1);
    let r0 = outer_radius * params.center_frac;
    let s = params.ring_shrink;
    // Geometric series so rings exactly fill (outer_radius - r0).
    let t0 = if (s - 1.0).abs() < 1e-4 {
        (outer_radius - r0) / depth as f32
    } else {
        (outer_radius - r0) * (1.0 - s) / (1.0 - s.powi(depth as i32))
    };
    let mut radii = Vec::with_capacity(depth);
    let mut r = r0;
    let mut t = t0;
    for _ in 0..depth {
        radii.push((r, r + t));
        r += t;
        t *= s;
    }

    let mut layout = Layout {
        metric,
        center,
        outer_radius,
        radii,
        rings: vec![Vec::new(); depth],
        index: HashMap::new(),
        view: ViewBounds::new(center, view),
    };

    let total = model.node(root).metric(metric);
    if total > 0 {
        place(model, &mut layout, params, root, 0.0, TAU, 0);
    }
    for (ring, sectors) in layout.rings.iter().enumerate() {
        for (i, s) in sectors.iter().enumerate() {
            if !s.is_group() {
                layout.index.insert(s.node, (ring, i));
            }
        }
    }
    layout
}

fn place(
    model: &Model,
    layout: &mut Layout,
    params: &LayoutParams,
    node: u32,
    a0: f32,
    a1: f32,
    ring: usize,
) {
    if ring >= layout.radii.len() || layout.radii[ring].0 > layout.view.r_max {
        return; // too deep, or this ring and all outer ones are off screen
    }
    let metric = layout.metric;
    let total = model.node(node).metric(metric);
    if total == 0 {
        return;
    }
    let range = model.children(node);
    let n = range.len();
    // Children are stored sorted by logical size; for the physical metric the
    // order can differ slightly, so sort a copy.
    let pending = if metric == Metric::Logical {
        emit(model, layout, params, node, range, n, total, a0, a1, ring)
    } else {
        let mut sorted: Vec<u32> = range.collect();
        sorted.sort_unstable_by_key(|&c| std::cmp::Reverse(model.node(c).alloc));
        emit(model, layout, params, node, sorted.into_iter(), n, total, a0, a1, ring)
    };
    for (c, ca0, ca1) in pending {
        place(model, layout, params, c, ca0, ca1, ring + 1);
    }
}

/// Emit sectors for `children` (largest first, `n` of them) of `parent` into
/// `ring`; returns the visible directories whose own children go on the next
/// ring.
#[allow(clippy::too_many_arguments)]
fn emit(
    model: &Model,
    layout: &mut Layout,
    params: &LayoutParams,
    parent: u32,
    children: impl Iterator<Item = u32>,
    n: usize,
    total: u64,
    a0: f32,
    a1: f32,
    ring: usize,
) -> Vec<(u32, f32, f32)> {
    let metric = layout.metric;
    let span = a1 - a0;
    let (r_in, r_out) = layout.radii[ring];
    let merge_angle = params.merge_arc_px / r_out;
    let min_angle = params.min_arc_px / r_out;
    let ring_visible = layout.view.radial(r_in, r_out);
    let angle_of = |m: u64| span * (m as f64 / total as f64) as f32;

    let mut a = a0;
    let mut used = 0u64;
    let mut pending = Vec::new();
    // Children arrive largest first, so the first one is the largest sibling.
    let mut largest = 0u64;
    let mut single = |layout: &mut Layout, c: u32, m: u64, ca: f32, largest: u64, a: f32| {
        let visible = layout.view.angular(a, a + ca);
        if visible && ring_visible {
            layout.rings[ring].push(Sector {
                node: c,
                a0: a,
                a1: a + ca,
                rel: (m as f64 / largest as f64) as f32,
                count: 0,
                group_size: 0,
            });
        }
        // Deeper rings can be on screen even when this one is not.
        if visible && model.node(c).is_dir {
            pending.push((c, a, a + ca));
        }
    };
    for (i, c) in children.enumerate() {
        let m = model.node(c).metric(metric);
        if m == 0 {
            break; // the rest are empty too
        }
        if largest == 0 {
            largest = m;
        }
        let ca = angle_of(m);
        if ca < merge_angle {
            // This and every following child are too thin: merge them.
            let rest = total.saturating_sub(used);
            let ga = angle_of(rest).min(a1 - a);
            let count = (n - i) as u32;
            if count == 1 {
                if ca >= min_angle {
                    single(layout, c, m, ca, largest, a);
                }
            } else if ga >= min_angle && ring_visible && layout.view.angular(a, a + ga) {
                layout.rings[ring].push(Sector {
                    node: parent,
                    a0: a,
                    a1: a + ga,
                    rel: 0.0,
                    count,
                    group_size: rest,
                });
            }
            break;
        }
        single(layout, c, m, ca, largest, a);
        a += ca;
        used += m;
    }
    pending
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RawDir, RawFile};

    fn f(name: &str, size: u64) -> RawFile {
        RawFile { name: name.into(), size, alloc: size }
    }

    /// A view large enough to show the whole chart.
    fn everything() -> Rect {
        Rect::from_center_size(Pos2::ZERO, egui::vec2(1e6, 1e6))
    }

    fn model(files: Vec<RawFile>) -> Model {
        let raw = RawDir {
            name: "r".into(),
            files,
            ..Default::default()
        };
        Model::from_raw(raw, "X:\\".into(), 1)
    }

    #[test]
    fn largest_first_and_hit_test() {
        let m = model(vec![f("a", 10), f("b", 30), f("c", 60)]);
        let l = build(
            &m,
            0,
            Metric::Logical,
            Pos2::new(0.0, 0.0),
            100.0,
            everything(),
            &LayoutParams::default(),
        );
        let names: Vec<&str> = l.rings[0].iter().map(|s| m.name(s.node)).collect();
        assert_eq!(names, vec!["c", "b", "a"]);
        // Sizes 60, 30, 10: relative to the largest sibling.
        for (s, want) in l.rings[0].iter().zip([1.0, 0.5, 1.0 / 6.0]) {
            assert!((s.rel - want).abs() < 1e-6, "{} != {want}", s.rel);
        }
        assert!((l.rings[0][0].a1 - TAU * 0.6).abs() < 1e-4);
        let r = l.radii[0].0 + 1.0;
        let name_at = |deg: f32| {
            let a = deg.to_radians();
            let p = Pos2::new(r * a.sin(), -r * a.cos());
            let (ring, i) = l.hit_test(p).unwrap();
            m.name(l.rings[ring][i].node)
        };
        assert_eq!(name_at(0.5), "c"); // 0..216 deg
        assert_eq!(name_at(215.0), "c");
        assert_eq!(name_at(270.0), "b"); // 216..324 deg
        assert_eq!(name_at(350.0), "a"); // 324..360 deg
        assert!(l.hit_test(Pos2::new(0.0, 0.0)).is_none());
        assert!(l.is_center(Pos2::new(1.0, 1.0)));
    }

    /// One big file and 200 tiny ones: the tiny ones become one group, and a
    /// larger chart (zoom) shows them individually.
    #[test]
    fn small_items_merge_and_split_on_zoom() {
        let mut files = vec![f("big", 1_000_000)];
        files.extend((0..200).map(|i| f(&format!("t{i}"), 50)));
        let m = model(files);
        let p = LayoutParams::default();
        let small = build(&m, 0, Metric::Logical, Pos2::ZERO, 200.0, everything(), &p);
        let ring = &small.rings[0];
        assert_eq!(ring.len(), 2, "big file + one group");
        assert!(!ring[0].is_group());
        let g = ring[1];
        assert!(g.is_group());
        assert_eq!((g.node, g.count, g.group_size), (0, 200, 10_000));
        assert!(!small.index.contains_key(&0), "groups are not indexed");

        // 50 bytes of 1.01 MB is ~0.0003 rad: >= 3 px once the ring is ~10k px.
        let big = build(&m, 0, Metric::Logical, Pos2::ZERO, 60_000.0, everything(), &p);
        assert_eq!(big.rings[0].len(), 201);
        assert!(big.rings[0].iter().all(|s| !s.is_group()));
    }

    /// A single thin leftover is drawn as itself, not as a group of one.
    #[test]
    fn lone_small_item_is_not_grouped() {
        // 3000 of ~1 MB at ring radius ~88 px is ~1.6 px: drawn, but below the merge width.
        let m = model(vec![f("big", 1_000_000), f("tiny", 3000)]);
        let l = build(&m, 0, Metric::Logical, Pos2::ZERO, 200.0, everything(), &LayoutParams::default());
        let names: Vec<&str> = l.rings[0].iter().map(|s| m.name(s.node)).collect();
        assert_eq!(names, vec!["big", "tiny"]);
    }

    /// Only the right half of the chart is visible: sectors on the left are
    /// culled, and a view beyond the outer radius yields nothing.
    #[test]
    fn culls_sectors_outside_the_view() {
        // Four equal quarters: 0-90, 90-180, 180-270, 270-360 degrees.
        let m = model(vec![f("q1", 25), f("q2", 25), f("q3", 25), f("q4", 25)]);
        let p = LayoutParams::default();
        let right = Rect::from_min_max(Pos2::new(10.0, -500.0), Pos2::new(500.0, 500.0));
        let l = build(&m, 0, Metric::Logical, Pos2::ZERO, 100.0, right, &p);
        let names: Vec<&str> = l.rings[0].iter().map(|s| m.name(s.node)).collect();
        assert_eq!(names, vec!["q1", "q2"]);

        let far = Rect::from_min_max(Pos2::new(300.0, 300.0), Pos2::new(400.0, 400.0));
        let l = build(&m, 0, Metric::Logical, Pos2::ZERO, 100.0, far, &p);
        assert_eq!(l.sector_count(), 0);
    }

    #[test]
    fn view_clip_handles_wrap_around() {
        // View straddling angle 0 (straight up), centre below it.
        let v = ViewBounds::new(
            Pos2::new(0.0, 100.0),
            Rect::from_min_max(Pos2::new(-10.0, -10.0), Pos2::new(10.0, 10.0)),
        );
        let (start, len) = v.arc.unwrap();
        assert!(start > PI && len < 0.3, "start {start}, len {len}");
        // A sector covering almost the whole turn is visible in two pieces.
        let pieces = v.clip(0.01, TAU - 0.01, 0.0);
        assert!(pieces[0].is_some() && pieces[1].is_some());
        // A sector on the opposite side is not visible.
        assert!(!v.angular(PI - 0.2, PI + 0.2));
    }
}
