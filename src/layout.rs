//! Sunburst layout: turns a subtree into rings of angular sectors.
//!
//! Only sectors whose arc length is at least `min_arc_px` are emitted, and
//! because children are visited largest-first the walk stops at the first
//! sub-threshold child. The number of sectors is therefore bounded by what is
//! visible, not by the size of the tree.

use std::collections::HashMap;
use std::f32::consts::TAU;

use egui::Pos2;

use crate::model::{Metric, Model};

#[derive(Clone, Copy, Debug)]
pub struct Sector {
    pub node: u32,
    /// Start angle in radians, 0 = top, increasing clockwise.
    pub a0: f32,
    pub a1: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct LayoutParams {
    pub max_depth: usize,
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
            min_arc_px: 1.0,
            center_frac: 0.24,
            ring_shrink: 0.78,
        }
    }
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
    /// node id -> (ring, index) for every emitted sector.
    pub index: HashMap<u32, (usize, usize)>,
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
        let r = d.length();
        // Screen y grows downward; 0 at top, clockwise positive.
        let mut a = d.x.atan2(-d.y);
        if a < 0.0 {
            a += TAU;
        }
        (a, r)
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

pub fn build(
    model: &Model,
    root: u32,
    metric: Metric,
    center: Pos2,
    outer_radius: f32,
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
    };

    let total = model.node(root).metric(metric);
    if total > 0 {
        place(model, &mut layout, params, root, 0.0, TAU, 0);
    }
    for (ring, sectors) in layout.rings.iter().enumerate() {
        for (i, s) in sectors.iter().enumerate() {
            layout.index.insert(s.node, (ring, i));
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
    if ring >= layout.radii.len() {
        return;
    }
    let metric = layout.metric;
    let total = model.node(node).metric(metric);
    if total == 0 {
        return;
    }
    let range = model.children(node);
    // Children are stored sorted by logical size; for the physical metric the
    // order can differ slightly, so sort a copy.
    let pending = if metric == Metric::Logical {
        emit(model, layout, params, range, total, a0, a1, ring)
    } else {
        let mut sorted: Vec<u32> = range.collect();
        sorted.sort_unstable_by_key(|&c| std::cmp::Reverse(model.node(c).alloc));
        emit(model, layout, params, sorted.into_iter(), total, a0, a1, ring)
    };
    for (c, ca0, ca1) in pending {
        place(model, layout, params, c, ca0, ca1, ring + 1);
    }
}

/// Emit sectors for `children` (largest first) into `ring`; returns the
/// directories that need their own children placed on the next ring.
#[allow(clippy::too_many_arguments)]
fn emit(
    model: &Model,
    layout: &mut Layout,
    params: &LayoutParams,
    children: impl Iterator<Item = u32>,
    total: u64,
    a0: f32,
    a1: f32,
    ring: usize,
) -> Vec<(u32, f32, f32)> {
    let metric = layout.metric;
    let span = a1 - a0;
    let r_out = layout.radii[ring].1;
    let min_angle = params.min_arc_px / r_out;
    let mut a = a0;
    let mut pending = Vec::new();
    for c in children {
        let n = model.node(c);
        let m = n.metric(metric);
        if m == 0 {
            break;
        }
        let ca = span * (m as f64 / total as f64) as f32;
        if ca < min_angle {
            break; // everything after is smaller
        }
        layout.rings[ring].push(Sector {
            node: c,
            a0: a,
            a1: a + ca,
        });
        if n.is_dir {
            pending.push((c, a, a + ca));
        }
        a += ca;
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

    #[test]
    fn largest_first_and_hit_test() {
        let raw = RawDir {
            name: "r".into(),
            files: vec![f("a", 10), f("b", 30), f("c", 60)],
            ..Default::default()
        };
        let m = Model::from_raw(raw, "X:\\".into(), 1);
        let l = build(
            &m,
            0,
            Metric::Logical,
            Pos2::new(0.0, 0.0),
            100.0,
            &LayoutParams::default(),
        );
        let names: Vec<&str> = l.rings[0].iter().map(|s| m.name(s.node)).collect();
        assert_eq!(names, vec!["c", "b", "a"]);
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
}
