//! Arena-based directory tree.
//!
//! All nodes live in one `Vec<Node>` packed in DFS order; the children of a
//! node form a contiguous slice sorted by logical size (descending). Names
//! live in a shared string arena. This keeps traversal allocation-free and
//! makes "largest first" ordering free at layout time.

use std::ops::Range;

pub const NO_NODE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    Logical,
    Physical,
}

#[derive(Clone, Debug)]
pub struct Node {
    name_start: u32,
    name_len: u16,
    pub is_dir: bool,
    pub parent: u32,
    first_child: u32,
    child_count: u32,
    /// Logical size in bytes (sum over the subtree for directories).
    pub size: u64,
    /// Allocated size on disk (cluster-rounded), summed over the subtree.
    pub alloc: u64,
    /// Number of files in the subtree.
    pub files: u32,
    /// Number of directories in the subtree (excluding this one).
    pub dirs: u32,
}

impl Node {
    #[inline]
    pub fn children(&self) -> Range<u32> {
        self.first_child..self.first_child + self.child_count
    }

    #[inline]
    pub fn metric(&self, m: Metric) -> u64 {
        match m {
            Metric::Logical => self.size,
            Metric::Physical => self.alloc,
        }
    }
}

/// Intermediate tree produced by a scanner before packing.
#[derive(Debug, Default)]
pub struct RawDir {
    pub name: String,
    pub files: Vec<RawFile>,
    pub subdirs: Vec<RawDir>,
    /// Subtree totals, computed by `RawDir::finalize`.
    pub size: u64,
    pub alloc: u64,
    pub file_count: u32,
    pub dir_count: u32,
}

#[derive(Debug)]
pub struct RawFile {
    pub name: String,
    pub size: u64,
    pub alloc: u64,
}

impl RawDir {
    /// Compute subtree aggregates bottom-up.
    pub fn finalize(&mut self) {
        let mut size = 0u64;
        let mut alloc = 0u64;
        let mut files = self.files.len() as u32;
        let mut dirs = self.subdirs.len() as u32;
        for f in &self.files {
            size += f.size;
            alloc += f.alloc;
        }
        for d in &mut self.subdirs {
            d.finalize();
            size += d.size;
            alloc += d.alloc;
            files += d.file_count;
            dirs += d.dir_count;
        }
        self.size = size;
        self.alloc = alloc;
        self.file_count = files;
        self.dir_count = dirs;
    }
}

#[derive(Debug, Default)]
pub struct Model {
    nodes: Vec<Node>,
    names: String,
    /// Root path as scanned, e.g. `C:\` or `D:\Projects`.
    pub root_path: String,
    pub cluster_size: u64,
}

impl Model {
    /// Pack a raw tree into the arena. Children of every node are sorted by
    /// logical size, descending; files and directories are interleaved.
    pub fn from_raw(mut root: RawDir, root_path: String, cluster_size: u64) -> Self {
        root.finalize();
        let total = 1 + root.file_count as usize + root.dir_count as usize;
        let mut m = Model {
            nodes: Vec::with_capacity(total),
            names: String::new(),
            root_path,
            cluster_size,
        };
        m.push_dir(&root, NO_NODE);
        m.pack_children(&root, 0);
        m
    }

    fn push_dir(&mut self, d: &RawDir, parent: u32) -> u32 {
        let id = self.nodes.len() as u32;
        let name_start = self.names.len() as u32;
        self.names.push_str(&d.name);
        self.nodes.push(Node {
            name_start,
            name_len: d.name.len().min(u16::MAX as usize) as u16,
            is_dir: true,
            parent,
            first_child: 0,
            child_count: 0,
            size: d.size,
            alloc: d.alloc,
            files: d.file_count,
            dirs: d.dir_count,
        });
        id
    }

    fn push_file(&mut self, f: &RawFile, parent: u32) -> u32 {
        let id = self.nodes.len() as u32;
        let name_start = self.names.len() as u32;
        self.names.push_str(&f.name);
        self.nodes.push(Node {
            name_start,
            name_len: f.name.len().min(u16::MAX as usize) as u16,
            is_dir: false,
            parent,
            first_child: 0,
            child_count: 0,
            size: f.size,
            alloc: f.alloc,
            files: 0,
            dirs: 0,
        });
        id
    }

    /// Emit the children of `d` (whose packed id is `id`) as one contiguous
    /// block, then recurse into subdirectories.
    fn pack_children(&mut self, d: &RawDir, id: u32) {
        enum Ref<'a> {
            D(&'a RawDir),
            F(&'a RawFile),
        }
        let mut order: Vec<(u64, Ref)> = Vec::with_capacity(d.files.len() + d.subdirs.len());
        for sd in &d.subdirs {
            order.push((sd.size, Ref::D(sd)));
        }
        for f in &d.files {
            order.push((f.size, Ref::F(f)));
        }
        // Stable sort keeps scanner order for equal sizes (deterministic).
        order.sort_by_key(|(size, _)| std::cmp::Reverse(*size));

        let first_child = self.nodes.len() as u32;
        let child_count = order.len() as u32;
        self.nodes[id as usize].first_child = first_child;
        self.nodes[id as usize].child_count = child_count;

        // First pass: emit all children contiguously.
        let mut dir_ids: Vec<(u32, &RawDir)> = Vec::with_capacity(d.subdirs.len());
        for (_, r) in &order {
            match r {
                Ref::D(sd) => {
                    let cid = self.push_dir(sd, id);
                    dir_ids.push((cid, sd));
                }
                Ref::F(f) => {
                    self.push_file(f, id);
                }
            }
        }
        // Second pass: recurse (their children blocks come after ours).
        for (cid, sd) in dir_ids {
            self.pack_children(sd, cid);
        }
    }

    #[inline]
    pub fn node(&self, id: u32) -> &Node {
        &self.nodes[id as usize]
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    #[inline]
    pub fn name(&self, id: u32) -> &str {
        let n = &self.nodes[id as usize];
        let s = n.name_start as usize;
        &self.names[s..s + n.name_len as usize]
    }

    /// Iterate child ids of `id` in stored (size-descending) order.
    #[inline]
    pub fn children(&self, id: u32) -> Range<u32> {
        self.nodes[id as usize].children()
    }

    /// Full path of a node, e.g. `C:\Users\Me\file.txt`.
    pub fn path(&self, mut id: u32) -> String {
        let mut parts: Vec<&str> = Vec::new();
        while id != NO_NODE {
            let n = &self.nodes[id as usize];
            if n.parent == NO_NODE {
                break;
            }
            parts.push(self.name(id));
            id = n.parent;
        }
        let mut out = self.root_path.clone();
        for p in parts.iter().rev() {
            if !out.ends_with('\\') && !out.ends_with('/') {
                out.push('\\');
            }
            out.push_str(p);
        }
        out
    }

    /// Names on the way from the scan root down to `id` (root excluded).
    pub fn rel_path(&self, mut id: u32) -> Vec<&str> {
        let mut parts = Vec::new();
        while self.nodes[id as usize].parent != NO_NODE {
            parts.push(self.name(id));
            id = self.nodes[id as usize].parent;
        }
        parts.reverse();
        parts
    }

    /// Directory reached by following `parts` from the root: the directory
    /// itself if it still exists, otherwise its deepest surviving ancestor.
    pub fn find_dir(&self, parts: &[&str]) -> u32 {
        let mut cur = 0;
        for p in parts {
            match self
                .children(cur)
                .find(|&c| self.node(c).is_dir && self.name(c) == *p)
            {
                Some(c) => cur = c,
                None => break,
            }
        }
        cur
    }

    /// The `limit` largest files under `root` by `metric`, largest first.
    /// Directories no larger than the smallest file kept so far are not
    /// entered: nothing inside them can make the list. Among files of equal
    /// size the choice is deterministic but otherwise unspecified.
    pub fn largest_files(&self, root: u32, metric: Metric, limit: usize) -> Vec<u32> {
        let mut best = TopN::new(limit);
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            for c in self.children(dir) {
                let n = self.node(c);
                let m = n.metric(metric);
                if !best.could_take(m) {
                    continue;
                }
                if n.is_dir {
                    stack.push(c);
                } else {
                    best.push(m, c);
                }
            }
        }
        best.into_ids()
    }

    /// Files and directories under `root` whose name contains `query`,
    /// ignoring case: how many there are, and the `limit` largest of them by
    /// `metric`, largest first.
    pub fn search(&self, root: u32, query: &str, metric: Metric, limit: usize) -> (usize, Vec<u32>) {
        let needle = query.to_lowercase();
        if needle.is_empty() {
            return (0, Vec::new());
        }
        // An ASCII query compares bytes without lowercasing the names; other
        // bytes of UTF-8 never equal ASCII ones, so any name works with it.
        let ascii = needle.is_ascii();
        let mut lowered = String::new();
        let mut matches = |name: &str| {
            if ascii {
                let (h, n) = (name.as_bytes(), needle.as_bytes());
                n.len() <= h.len() && h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
            } else if name.is_ascii() {
                false // lowercases to ASCII, so it cannot hold the non-ASCII needle
            } else {
                lowered.clear();
                lowered.extend(name.chars().flat_map(char::to_lowercase));
                lowered.contains(&needle)
            }
        };
        let mut best = TopN::new(limit);
        let mut count = 0;
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            for c in self.children(dir) {
                if matches(self.name(c)) {
                    count += 1;
                    best.push(self.node(c).metric(metric), c);
                }
                if self.node(c).is_dir {
                    stack.push(c);
                }
            }
        }
        (count, best.into_ids())
    }
}

/// The `limit` largest of the ids pushed, kept in a min-heap.
struct TopN {
    limit: usize,
    heap: std::collections::BinaryHeap<std::cmp::Reverse<(u64, std::cmp::Reverse<u32>)>>,
}

impl TopN {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            heap: std::collections::BinaryHeap::with_capacity(limit + 1),
        }
    }

    /// Whether an item of size `m` would get into the list now.
    fn could_take(&self, m: u64) -> bool {
        self.heap.len() < self.limit || self.heap.peek().is_some_and(|min| m > min.0.0)
    }

    fn push(&mut self, m: u64, id: u32) {
        if self.could_take(m) {
            self.heap.push(std::cmp::Reverse((m, std::cmp::Reverse(id))));
            if self.heap.len() > self.limit {
                self.heap.pop();
            }
        }
    }

    /// Ids, largest first.
    fn into_ids(self) -> Vec<u32> {
        self.heap
            .into_sorted_vec()
            .into_iter()
            .map(|std::cmp::Reverse((_, std::cmp::Reverse(id)))| id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64) -> RawFile {
        RawFile { name: name.into(), size, alloc: size }
    }

    #[test]
    fn packs_sorted_and_contiguous() {
        let raw = RawDir {
            name: "root".into(),
            files: vec![file("small", 1), file("big", 100)],
            subdirs: vec![RawDir {
                name: "sub".into(),
                files: vec![file("a", 10), file("b", 20)],
                ..Default::default()
            }],
            ..Default::default()
        };
        let m = Model::from_raw(raw, "X:\\".into(), 4096);
        assert_eq!(m.len(), 6);
        let root = m.node(0);
        assert_eq!(root.size, 131);
        assert_eq!(root.files, 4);
        assert_eq!(root.dirs, 1);
        let kids: Vec<&str> = m.children(0).map(|c| m.name(c)).collect();
        assert_eq!(kids, vec!["big", "sub", "small"]);
        let sub = m.children(0).find(|&c| m.node(c).is_dir).unwrap();
        let sk: Vec<&str> = m.children(sub).map(|c| m.name(c)).collect();
        assert_eq!(sk, vec!["b", "a"]);
        assert_eq!(m.path(m.children(sub).next().unwrap()), "X:\\sub\\b");
    }

    #[test]
    fn largest_files_match_brute_force() {
        // Deterministic pseudo-random tree, 4 levels deep.
        let mut seed = 12345u64;
        let mut rnd = move |n: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        fn grow(rnd: &mut impl FnMut(u64) -> u64, depth: u32, name: String) -> RawDir {
            let files = (0..rnd(6))
                .map(|i| {
                    let bits = rnd(20);
                    let size = rnd(1 << bits);
                    RawFile { name: format!("f{i}"), size, alloc: size.div_ceil(4096) * 4096 }
                })
                .collect();
            let subdirs = if depth == 0 {
                Vec::new()
            } else {
                (0..rnd(4)).map(|i| grow(rnd, depth - 1, format!("d{i}"))).collect()
            };
            RawDir { name, files, subdirs, ..Default::default() }
        }
        let m = Model::from_raw(grow(&mut rnd, 4, "root".into()), "X:\\".into(), 4096);
        let under = |root: u32, mut id: u32| loop {
            if id == root {
                return true;
            }
            if id == NO_NODE {
                return false;
            }
            id = m.node(id).parent;
        };
        let roots: Vec<u32> = (0..m.len() as u32).filter(|&i| m.node(i).is_dir).take(8).collect();
        for &root in &roots {
            for metric in [Metric::Logical, Metric::Physical] {
                for limit in [0, 1, 3, 10, 1000] {
                    let mut all: Vec<u64> = (0..m.len() as u32)
                        .filter(|&i| !m.node(i).is_dir && under(root, i))
                        .map(|i| m.node(i).metric(metric))
                        .collect();
                    all.sort_unstable_by(|a, b| b.cmp(a));
                    all.truncate(limit);
                    let got = m.largest_files(root, metric, limit);
                    assert!(got.iter().all(|&i| !m.node(i).is_dir && under(root, i)));
                    let sizes: Vec<u64> = got.iter().map(|&i| m.node(i).metric(metric)).collect();
                    assert_eq!(sizes, all, "root {root}, {metric:?}, limit {limit}");
                }
            }
        }
    }

    #[test]
    fn search_ignores_case_and_counts_all_matches() {
        let raw = RawDir {
            name: "root".into(),
            files: vec![file("Report.PDF", 50), file("notes.txt", 5)],
            subdirs: vec![RawDir {
                name: "Отчёты".into(),
                files: vec![file("report-2025.pdf", 70), file("ОТЧЁТ.docx", 30)],
                ..Default::default()
            }],
            ..Default::default()
        };
        let m = Model::from_raw(raw, "X:\\".into(), 1);
        let names = |ids: Vec<u32>| ids.into_iter().map(|i| m.name(i).to_string()).collect::<Vec<_>>();

        let (count, ids) = m.search(0, "REPORT", Metric::Logical, 10);
        assert_eq!(count, 2);
        assert_eq!(names(ids), ["report-2025.pdf", "Report.PDF"]);

        // Cyrillic, case-insensitive: the folder and a file inside it.
        let (count, ids) = m.search(0, "отчё", Metric::Logical, 10);
        assert_eq!(count, 2);
        assert_eq!(names(ids), ["Отчёты", "ОТЧЁТ.docx"]);

        // The limit keeps the largest, the count stays complete.
        let (count, ids) = m.search(0, ".", Metric::Logical, 2);
        assert_eq!(count, 4);
        assert_eq!(names(ids), ["report-2025.pdf", "Report.PDF"]);

        // Only under the given root; an empty query finds nothing.
        let sub = m.find_dir(&["Отчёты"]);
        assert_eq!(m.search(sub, "pdf", Metric::Logical, 10).0, 1);
        assert_eq!(m.search(0, "", Metric::Logical, 10), (0, Vec::new()));
    }

    #[test]
    fn find_dir_by_path() {
        let raw = RawDir {
            name: "root".into(),
            files: vec![file("sub", 5)],
            subdirs: vec![RawDir {
                name: "sub".into(),
                subdirs: vec![RawDir { name: "deep".into(), ..Default::default() }],
                files: vec![file("f", 1)],
                ..Default::default()
            }],
            ..Default::default()
        };
        let m = Model::from_raw(raw, "X:\\".into(), 4096);
        let sub = m.children(0).find(|&c| m.node(c).is_dir).unwrap();
        let deep = m.children(sub).find(|&c| m.node(c).is_dir).unwrap();
        assert_eq!(m.rel_path(deep), vec!["sub", "deep"]);
        assert!(m.rel_path(0).is_empty());
        assert_eq!(m.find_dir(&m.rel_path(deep)), deep);
        assert_eq!(m.find_dir(&[]), 0);
        // A vanished directory falls back to its closest ancestor.
        assert_eq!(m.find_dir(&["sub", "gone", "x"]), sub);
        // Files are never matched, even with the same name.
        assert_eq!(m.find_dir(&["sub", "f"]), sub);
        assert_eq!(m.find_dir(&["nope"]), 0);
    }
}
