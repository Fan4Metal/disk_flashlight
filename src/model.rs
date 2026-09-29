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
    /// Last write time in Unix seconds, 0 if unknown. For a directory, the
    /// newest time in its subtree (its own time if it is empty). Fits in
    /// the padding, so it costs no memory.
    pub modified: u32,
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
    /// Last write time of the folder itself (Unix seconds, 0 = unknown);
    /// `finalize` replaces it with the newest time in the subtree unless
    /// the folder is empty.
    pub modified: u32,
}

#[derive(Debug, Default)]
pub struct RawFile {
    pub name: String,
    pub size: u64,
    pub alloc: u64,
    /// Last write time in Unix seconds, 0 if unknown.
    pub modified: u32,
}

impl RawDir {
    /// Compute subtree aggregates bottom-up.
    pub fn finalize(&mut self) {
        let mut size = 0u64;
        let mut alloc = 0u64;
        let mut files = self.files.len() as u32;
        let mut dirs = self.subdirs.len() as u32;
        let mut newest = 0u32;
        for f in &self.files {
            size += f.size;
            alloc += f.alloc;
            newest = newest.max(f.modified);
        }
        for d in &mut self.subdirs {
            d.finalize();
            size += d.size;
            alloc += d.alloc;
            files += d.file_count;
            dirs += d.dir_count;
            newest = newest.max(d.modified);
        }
        self.size = size;
        self.alloc = alloc;
        self.file_count = files;
        self.dir_count = dirs;
        // The folder's own time changes when an entry is added, removed or
        // renamed, which says little about the age of its contents.
        if !self.files.is_empty() || !self.subdirs.is_empty() {
            self.modified = newest;
        }
    }
}

/// Seconds between 1601-01-01 (FILETIME epoch) and 1970-01-01.
const FILETIME_UNIX_DIFF: u64 = 11_644_473_600;

/// A Windows FILETIME (100 ns ticks since 1601) as Unix seconds, saturated
/// to `u32`; times before 1970 and 0 (unset) become 0, "unknown".
pub fn unix_from_filetime(ft: u64) -> u32 {
    (ft / 10_000_000)
        .saturating_sub(FILETIME_UNIX_DIFF)
        .min(u32::MAX as u64) as u32
}

/// The current time in Unix seconds.
pub fn unix_now() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs().min(u32::MAX as u64) as u32)
}

#[derive(Debug, Default)]
pub struct Model {
    nodes: Vec<Node>,
    names: String,
    /// Root path as scanned, e.g. `C:\` or `D:\Projects`.
    pub root_path: String,
    pub cluster_size: u64,
    /// When the scan finished (Unix seconds): ages are counted from here,
    /// so that they do not drift while the results are looked at.
    pub scanned_at: u32,
    /// Type (extension) of every file, built with the model.
    pub types: crate::types::FileTypes,
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
            scanned_at: unix_now(),
            types: Default::default(),
        };
        m.push_dir(&root, NO_NODE);
        m.pack_children(&root, 0);
        m.types = crate::types::FileTypes::build(&m);
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
            modified: d.modified,
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
            modified: f.modified,
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

    /// A copy without `id` and everything under it, the sizes and counts
    /// above it reduced accordingly: the tree after deleting it. Ids change;
    /// map them by path (`rel_path` / `find_dir`). `id` must not be the root.
    pub fn without(&self, id: u32) -> Model {
        debug_assert_ne!(id, 0, "the root cannot be removed");
        let mut m = Model::from_raw(self.raw_dir(0, id), self.root_path.clone(), self.cluster_size);
        m.scanned_at = self.scanned_at;
        m
    }

    /// The subtree of `dir` as a scanner would produce it, leaving out
    /// `skip`. Directories before files, each in stored order, so that the
    /// stable sort in `pack_children` keeps the order of equal sizes.
    fn raw_dir(&self, dir: u32, skip: u32) -> RawDir {
        // A folder left empty keeps the newest time of what it held.
        let mut d = RawDir {
            name: self.name(dir).to_string(),
            modified: self.node(dir).modified,
            ..Default::default()
        };
        for c in self.children(dir) {
            let n = self.node(c);
            if c == skip {
                continue;
            } else if n.is_dir {
                d.subdirs.push(self.raw_dir(c, skip));
            } else {
                d.files.push(RawFile {
                    name: self.name(c).to_string(),
                    size: n.size,
                    alloc: n.alloc,
                    modified: n.modified,
                });
            }
        }
        d
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

    /// The `limit` largest files under `root` by `metric`, largest first;
    /// with `before`, only files last modified before that time (Unix
    /// seconds; files without a known time are left out). Directories no
    /// larger than the smallest file kept so far are not entered: nothing
    /// inside them can make the list. Among files of equal size the choice
    /// is deterministic but otherwise unspecified.
    pub fn largest_files(&self, root: u32, metric: Metric, limit: usize, before: Option<u32>) -> Vec<u32> {
        let old_enough = |t: u32| before.is_none_or(|b| t != 0 && t < b);
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
                } else if old_enough(n.modified) {
                    best.push(m, c);
                }
            }
        }
        best.into_ids()
    }

    /// Files and directories under `root` whose names match `query`, with
    /// the `limit` largest of them by `metric`.
    ///
    /// The query is split into terms at spaces (`"..."` keeps a phrase
    /// together), and a name must match every term, ignoring case; terms
    /// joined by `|` or `;` are alternatives, one of which must match
    /// (`a|b c` is "a or b, and c"; `*.mp4;*.mkv` as in Explorer). A term
    /// is a part of the name, or with `*` (any run of characters) or `?`
    /// (one character) a mask for the whole name, as in Explorer: `*.py`
    /// finds names ending in `.py`.
    ///
    /// With `whole_word`, a part must not continue a word on either side
    /// (letters, digits and `_` make words). Only ends of the term that are
    /// word characters are checked, so `.py` finds `a.py` and `a.py.bak` but
    /// not `a.pyd`. Masks are not affected.
    ///
    /// `kind` limits matches to files or folders; the others are still
    /// searched through but neither listed nor counted.
    pub fn search(
        &self,
        root: u32,
        query: &str,
        metric: Metric,
        limit: usize,
        whole_word: bool,
        kind: ItemKind,
    ) -> Found {
        let groups = Term::parse(query);
        if groups.is_empty() {
            return Found::default();
        }
        let mut lowered = Lowered::default();
        self.find(root, metric, limit, |name, is_dir| {
            if !kind.allows(is_dir) {
                return false;
            }
            lowered.reset();
            groups
                .iter()
                .all(|any| any.iter().any(|t| t.matches(name, whole_word, &mut lowered)))
        })
    }

    /// Everything under `root` whose name `matches`, for [`Self::search`].
    fn find(&self, root: u32, metric: Metric, limit: usize, mut matches: impl FnMut(&str, bool) -> bool) -> Found {
        let mut best = TopN::new(limit);
        let (mut count, mut total) = (0, 0);
        let mut hits = vec![0u64; self.nodes.len()];
        // A folder is pushed with whether it or one of its ancestors matched.
        let mut stack = vec![(root, false)];
        while let Some((dir, inside)) = stack.pop() {
            for c in self.children(dir) {
                let n = self.node(c);
                let m = n.metric(metric);
                let hit = matches(self.name(c), n.is_dir);
                if hit || inside {
                    hits[c as usize] = m;
                }
                if hit {
                    count += 1;
                    best.push(m, c);
                    if !inside {
                        total += m;
                        let mut p = n.parent;
                        while p != NO_NODE {
                            hits[p as usize] += m;
                            p = self.node(p).parent;
                        }
                    }
                }
                if n.is_dir {
                    stack.push((c, inside || hit));
                }
            }
        }
        Found {
            count,
            total,
            ids: best.into_ids(),
            hits,
        }
    }
}

/// Which items [`Model::search`] lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ItemKind {
    #[default]
    All,
    Files,
    Folders,
}

impl ItemKind {
    pub fn allows(self, is_dir: bool) -> bool {
        match self {
            ItemKind::All => true,
            ItemKind::Files => !is_dir,
            ItemKind::Folders => is_dir,
        }
    }
}

/// What [`Model::search`] found.
#[derive(Debug, Default, PartialEq)]
pub struct Found {
    /// Number of matches.
    pub count: usize,
    /// Size of the matches by the metric, each byte once: matches inside a
    /// matching folder are already part of its size.
    pub total: u64,
    /// The largest matches, largest first.
    pub ids: Vec<u32>,
    /// Per node: bytes of it that match, each once. A match and everything
    /// inside it have their whole size, a folder above matches the sum of
    /// those below it, anything else 0 (empty when the query is).
    pub hits: Vec<u64>,
}

/// Whether `text` matches the mask `pat` as a whole, where `*` stands for
/// any run of characters and `?` for one; `eq(p, c)` compares a mask
/// character with a text one. On a mismatch after `*` only the last `*` is
/// retried one character further, which is enough and keeps it linear in
/// practice.
fn wildcard<T: Copy + PartialEq + From<u8>>(pat: &[T], text: &[T], eq: impl Fn(T, T) -> bool) -> bool {
    let (star, any) = (T::from(b'*'), T::from(b'?'));
    let same = |p: T, c: T| p == any || eq(p, c);
    // The part after the last `*` has a fixed length, so it must match the
    // end of the text: checked first, this rejects most names for `*.ext`.
    let (pat, text) = match pat.iter().rposition(|&c| c == star) {
        Some(last) => {
            let tail = &pat[last + 1..];
            let Some(cut) = text.len().checked_sub(tail.len()) else {
                return false;
            };
            if !tail.iter().zip(&text[cut..]).all(|(&p, &c)| same(p, c)) {
                return false;
            }
            (&pat[..=last], &text[..cut])
        }
        None => (pat, text),
    };
    let (mut p, mut t) = (0, 0);
    let mut retry = None; // (mask position after the last `*`, text position)
    while t < text.len() {
        if p < pat.len() && pat[p] == star {
            p += 1;
            retry = Some((p, t));
        } else if p < pat.len() && same(pat[p], text[t]) {
            p += 1;
            t += 1;
        } else if let Some((rp, rt)) = retry {
            p = rp;
            t = rt + 1;
            retry = Some((rp, t));
        } else {
            return false;
        }
    }
    pat[p..].iter().all(|&c| c == star)
}

/// Whether every term of `query` is a mask, so that whole words do not
/// apply to it.
pub fn masks_only(query: &str) -> bool {
    let groups = Term::parse(query);
    !groups.is_empty() && groups.iter().flatten().all(|t| matches!(t, Term::Mask { .. }))
}

/// Byte ranges of `name` matched by the parts of `query` (masks cover the
/// whole name and are left out), sorted and merged: every occurrence of
/// every part, as [`Model::search`] would find it, for showing in the list.
pub fn match_ranges(query: &str, whole_word: bool, name: &str) -> Vec<Range<usize>> {
    // Lowercase characters of the name, each with the byte range of the
    // original character it comes from.
    let mut low: Vec<(char, Range<usize>)> = Vec::new();
    for (at, c) in name.char_indices() {
        for l in c.to_lowercase() {
            low.push((l, at..at + c.len_utf8()));
        }
    }
    let mut found: Vec<Range<usize>> = Vec::new();
    for term in Term::parse(query).iter().flatten() {
        let Term::Part {
            needle,
            check_start,
            check_end,
        } = term
        else {
            continue;
        };
        let pat: Vec<char> = needle.chars().collect();
        for (i, w) in low.windows(pat.len()).enumerate() {
            if !w.iter().zip(&pat).all(|((l, _), p)| l == p) {
                continue;
            }
            let word_at = |j: Option<usize>| j.and_then(|j| low.get(j)).is_some_and(|(l, _)| is_word_char(*l));
            if whole_word && (*check_start && word_at(i.checked_sub(1)) || *check_end && word_at(Some(i + pat.len()))) {
                continue;
            }
            found.push(w[0].1.start..w[pat.len() - 1].1.end);
        }
    }
    found.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for r in found {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

/// One term of a search query, lowercase.
enum Term {
    /// Part of a name. `check_start` / `check_end`: whether that end is a
    /// word character, which whole-word matching then checks.
    Part {
        needle: String,
        check_start: bool,
        check_end: bool,
    },
    /// Mask for the whole name, also as characters for non-ASCII names.
    Mask { pat: String, chars: Vec<char> },
}

impl Term {
    /// Split `query` at whitespace, `|` and `;` outside `"..."` into groups
    /// that must all match, each a list of alternatives: terms joined by `|`
    /// or `;`. Empty terms and stray separators are dropped.
    fn parse(query: &str) -> Vec<Vec<Term>> {
        let mut groups: Vec<Vec<Term>> = Vec::new();
        let mut cur = String::new();
        let mut quoted = false;
        // The next term joins the last group (after a `|`).
        let mut join = false;
        let mut flush = |cur: &mut String, join: &mut bool| {
            if !cur.is_empty() {
                let term = Term::new(cur.to_lowercase());
                match groups.last_mut() {
                    Some(any) if *join => any.push(term),
                    _ => groups.push(vec![term]),
                }
                cur.clear();
                *join = false;
            }
        };
        for c in query.chars() {
            if c == '"' {
                quoted = !quoted;
            } else if quoted || !(c.is_whitespace() || c == '|' || c == ';') {
                cur.push(c);
            } else {
                flush(&mut cur, &mut join);
                if c == '|' || c == ';' {
                    join = true;
                }
            }
        }
        flush(&mut cur, &mut join);
        groups
    }

    fn new(text: String) -> Self {
        if text.contains(['*', '?']) {
            Term::Mask {
                chars: text.chars().collect(),
                pat: text,
            }
        } else {
            Term::Part {
                check_start: text.chars().next().is_some_and(is_word_char),
                check_end: text.chars().next_back().is_some_and(is_word_char),
                needle: text,
            }
        }
    }

    fn matches(&self, name: &str, whole_word: bool, lowered: &mut Lowered) -> bool {
        match self {
            Term::Part {
                needle,
                check_start,
                check_end,
            } => {
                // Whether `hay[at..at + len]`, a match, stands as a whole word.
                let bounded = |hay: &str, at: usize| {
                    !(*check_start && hay[..at].chars().next_back().is_some_and(is_word_char)
                        || *check_end && hay[at + needle.len()..].chars().next().is_some_and(is_word_char))
                };
                if needle.is_ascii() {
                    // Compare bytes without lowercasing the name; other
                    // bytes of UTF-8 never equal ASCII ones, so any name
                    // works (and a match starts and ends on char boundaries).
                    let (h, n) = (name.as_bytes(), needle.as_bytes());
                    n.len() <= h.len()
                        && h.windows(n.len())
                            .enumerate()
                            .any(|(at, w)| w.eq_ignore_ascii_case(n) && (!whole_word || bounded(name, at)))
                } else if name.is_ascii() {
                    false // lowercases to ASCII, so it cannot hold the non-ASCII needle
                } else {
                    let hay = lowered.text(name);
                    if whole_word {
                        hay.match_indices(needle.as_str()).any(|(at, _)| bounded(hay, at))
                    } else {
                        hay.contains(needle.as_str())
                    }
                }
            }
            Term::Mask { pat, chars } => {
                if pat.is_ascii() && name.is_ascii() {
                    wildcard(pat.as_bytes(), name.as_bytes(), |p, c| p == c.to_ascii_lowercase())
                } else {
                    wildcard(chars, lowered.chars(name), |p, c| p == c)
                }
            }
        }
    }
}

/// Lowercase forms of the name being searched, built on first use and shared
/// by the terms.
#[derive(Default)]
struct Lowered {
    text: String,
    chars: Vec<char>,
    has_text: bool,
    has_chars: bool,
}

impl Lowered {
    /// Start on a new name.
    fn reset(&mut self) {
        self.has_text = false;
        self.has_chars = false;
    }

    fn text(&mut self, name: &str) -> &str {
        if !self.has_text {
            self.text.clear();
            self.text.extend(name.chars().flat_map(char::to_lowercase));
            self.has_text = true;
        }
        &self.text
    }

    fn chars(&mut self, name: &str) -> &[char] {
        if !self.has_chars {
            self.chars.clear();
            self.chars.extend(name.chars().flat_map(char::to_lowercase));
            self.has_chars = true;
        }
        &self.chars
    }
}

/// Characters that make up words for [`Model::search`]'s whole-word mode.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
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
        RawFile { name: name.into(), size, alloc: size, modified: 0 }
    }

    fn dated(name: &str, size: u64, modified: u32) -> RawFile {
        RawFile { modified, ..file(name, size) }
    }

    #[test]
    fn node_stays_small() {
        // The time sits in what was padding; a million nodes stay 48 MB.
        assert_eq!(size_of::<Node>(), 48);
    }

    #[test]
    fn filetime_to_unix() {
        assert_eq!(unix_from_filetime(0), 0);
        assert_eq!(unix_from_filetime(116_444_736_000_000_000), 0);
        // 2024-01-01 00:00:00 UTC.
        assert_eq!(unix_from_filetime(133_485_408_000_000_000), 1_704_067_200);
        assert_eq!(unix_from_filetime(u64::MAX), u32::MAX);
    }

    #[test]
    fn folders_take_the_newest_time_inside() {
        let raw = RawDir {
            name: "root".into(),
            modified: 999,
            files: vec![dated("old", 1, 100)],
            subdirs: vec![
                RawDir {
                    name: "fresh".into(),
                    modified: 5,
                    files: vec![dated("a", 1, 300), dated("b", 1, 200)],
                    ..Default::default()
                },
                RawDir { name: "empty".into(), modified: 250, ..Default::default() },
            ],
            ..Default::default()
        };
        let m = Model::from_raw(raw, "X:\\".into(), 1);
        let id = |name: &str| m.children(0).find(|&c| m.name(c) == name).unwrap();
        // The folder's own time (5, 999) gives way to its contents.
        assert_eq!(m.node(id("fresh")).modified, 300);
        assert_eq!(m.node(id("empty")).modified, 250);
        assert_eq!(m.node(id("old")).modified, 100);
        assert_eq!(m.node(0).modified, 300);
        // Deleting the newest file makes the folder older.
        let a = m.children(id("fresh")).find(|&c| m.name(c) == "a").unwrap();
        let m2 = m.without(a);
        assert_eq!(m2.node(m2.find_dir(&["fresh"])).modified, 200);
        assert_eq!(m2.node(0).modified, 250);
        assert_eq!(m2.scanned_at, m.scanned_at);
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
                    let modified = rnd(100) as u32;
                    RawFile { name: format!("f{i}"), size, alloc: size.div_ceil(4096) * 4096, modified }
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
                    for before in [None, Some(50)] {
                        // Time 0 is "unknown" and never passes the filter.
                        let old = |i: u32| before.is_none_or(|b| (1..b).contains(&m.node(i).modified));
                        let mut all: Vec<u64> = (0..m.len() as u32)
                            .filter(|&i| !m.node(i).is_dir && under(root, i) && old(i))
                            .map(|i| m.node(i).metric(metric))
                            .collect();
                        all.sort_unstable_by(|a, b| b.cmp(a));
                        all.truncate(limit);
                        let got = m.largest_files(root, metric, limit, before);
                        assert!(got.iter().all(|&i| !m.node(i).is_dir && under(root, i) && old(i)));
                        let sizes: Vec<u64> = got.iter().map(|&i| m.node(i).metric(metric)).collect();
                        assert_eq!(sizes, all, "root {root}, {metric:?}, limit {limit}, before {before:?}");
                    }
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

        let f = m.search(0, "REPORT", Metric::Logical, 10, false, ItemKind::All);
        assert_eq!((f.count, f.total), (2, 120));
        assert_eq!(names(f.ids), ["report-2025.pdf", "Report.PDF"]);

        // Cyrillic, case-insensitive: the folder and a file inside it, which
        // the total does not count twice.
        let f = m.search(0, "отчё", Metric::Logical, 10, false, ItemKind::All);
        assert_eq!((f.count, f.total), (2, 100));
        let sub = m.find_dir(&["Отчёты"]);
        // Everything in the matching folder is covered, the root partly.
        assert_eq!(f.hits[0], 100);
        assert!(m.children(sub).all(|c| f.hits[c as usize] == m.node(c).size));
        assert_eq!(names(f.ids), ["Отчёты", "ОТЧЁТ.docx"]);

        let f = m.search(0, "REPORT", Metric::Logical, 10, false, ItemKind::All);
        assert_eq!((f.hits[0], f.hits[sub as usize]), (120, 70));

        // Files only: the folder no longer matches, so only the file counts.
        let f = m.search(0, "отчё", Metric::Logical, 10, false, ItemKind::Files);
        assert_eq!((f.count, f.total, f.hits[0]), (1, 30, 30));
        assert_eq!(names(f.ids), ["ОТЧЁТ.docx"]);
        // Folders only: the folder covers the file inside it, unlisted.
        let f = m.search(0, "отчё", Metric::Logical, 10, false, ItemKind::Folders);
        assert_eq!((f.count, f.total, f.hits[0]), (1, 100, 100));
        assert_eq!(names(f.ids), ["Отчёты"]);
        assert!(m.children(sub).all(|c| f.hits[c as usize] == m.node(c).size));

        // The limit keeps the largest, the count and total stay complete.
        let f = m.search(0, ".", Metric::Logical, 2, false, ItemKind::All);
        assert_eq!((f.count, f.total), (4, 155));
        assert_eq!(names(f.ids), ["report-2025.pdf", "Report.PDF"]);

        // Only under the given root; an empty query finds nothing.
        let sub = m.find_dir(&["Отчёты"]);
        assert_eq!(m.search(sub, "pdf", Metric::Logical, 10, false, ItemKind::All).count, 1);
        assert_eq!(m.search(0, "", Metric::Logical, 10, false, ItemKind::All), Found::default());
    }

    #[test]
    fn search_whole_words() {
        let raw = RawDir {
            name: "root".into(),
            files: ["a.py", "b.pyd", "c.py.bak", "my_py.txt", "py", "pyd.py", "Отчёт 2025.doc", "Отчёты.doc"]
                .into_iter()
                .map(|n| file(n, 1))
                .collect(),
            ..Default::default()
        };
        let m = Model::from_raw(raw, "X:\\".into(), 1);
        let found = |q: &str| {
            let mut names: Vec<String> = m
                .search(0, q, Metric::Logical, 10, true, ItemKind::All)
                .ids
                .into_iter()
                .map(|i| m.name(i).to_string())
                .collect();
            names.sort();
            names
        };
        // A leading dot is not checked, the trailing "y" is.
        assert_eq!(found(".PY"), ["a.py", "c.py.bak", "pyd.py"]);
        // "_" is a word character; the name itself may be the word; in
        // "pyd.py" the second occurrence counts.
        assert_eq!(found("py"), ["a.py", "c.py.bak", "py", "pyd.py"]);
        // Non-ASCII names.
        assert_eq!(found("отчёт"), ["Отчёт 2025.doc"]);
        assert_eq!(m.search(0, "отчёт", Metric::Logical, 10, false, ItemKind::All).count, 2);
    }

    #[test]
    fn search_masks() {
        let raw = RawDir {
            name: "root".into(),
            files: ["a.py", "b.pyd", "c.py.bak", "Script.PY", "отчёт.py", "abab.txt", "x.txt"]
                .into_iter()
                .map(|n| file(n, 1))
                .collect(),
            ..Default::default()
        };
        let m = Model::from_raw(raw, "X:\\".into(), 1);
        let found = |q: &str, whole_word| {
            let mut names: Vec<String> = m
                .search(0, q, Metric::Logical, 10, whole_word, ItemKind::All)
                .ids
                .into_iter()
                .map(|i| m.name(i).to_string())
                .collect();
            names.sort();
            names
        };
        // The mask covers the whole name, ignoring case, in any script.
        assert_eq!(found("*.py", false), ["Script.PY", "a.py", "отчёт.py"]);
        assert_eq!(found("?.py*", false), ["a.py", "b.pyd", "c.py.bak"]);
        assert_eq!(found("ОТЧ?Т.*", false), ["отчёт.py"]);
        assert_eq!(found("*.p", false), Vec::<String>::new());
        // A `*` retried after a partial match.
        assert_eq!(found("*ab*.txt", false), ["abab.txt"]);
        assert_eq!(found("*", false).len(), 7);
        // Whole words do not apply to masks.
        assert_eq!(found("*.py", true), found("*.py", false));
    }

    #[test]
    fn search_all_terms() {
        let raw = RawDir {
            name: "root".into(),
            files: ["Big Fish (2003).mp4", "fish big.mp4", "Big Fish.srt", "Отчёт 2025.pdf", "report 2025.pdf"]
                .into_iter()
                .map(|n| file(n, 1))
                .collect(),
            ..Default::default()
        };
        let m = Model::from_raw(raw, "X:\\".into(), 1);
        let found = |q: &str, whole_word| {
            let mut names: Vec<String> = m
                .search(0, q, Metric::Logical, 10, whole_word, ItemKind::All)
                .ids
                .into_iter()
                .map(|i| m.name(i).to_string())
                .collect();
            names.sort();
            names
        };
        // Every term, in any order; masks and parts mix.
        assert_eq!(found("FISH  big", false), ["Big Fish (2003).mp4", "Big Fish.srt", "fish big.mp4"]);
        assert_eq!(found("*.mp4 big fish", false), ["Big Fish (2003).mp4", "fish big.mp4"]);
        assert_eq!(found("2025 отчёт", false), ["Отчёт 2025.pdf"]);
        // Quotes keep a phrase; an unclosed one runs to the end.
        assert_eq!(found("\"big fish\"", false), ["Big Fish (2003).mp4", "Big Fish.srt"]);
        assert_eq!(found("mp4 \"big fish", false), ["Big Fish (2003).mp4"]);
        // Whole words apply to each part.
        assert_eq!(found("fish 200", true), Vec::<String>::new());
        assert_eq!(found("fish 2003", true), ["Big Fish (2003).mp4"]);
        assert_eq!(m.search(0, "  \"\" ", Metric::Logical, 10, false, ItemKind::All), Found::default());
        assert!(masks_only("*.mp4 ?.srt") && !masks_only("*.mp4 fish") && !masks_only(""));
        assert!(!masks_only("*.mp4|fish"));
    }

    #[test]
    fn search_alternatives() {
        let raw = RawDir {
            name: "root".into(),
            files: ["a 2010.mp4", "b 2010.mkv", "c 2011.mkv", "d 2010.avi", "e|f.txt"]
                .into_iter()
                .map(|n| file(n, 1))
                .collect(),
            ..Default::default()
        };
        let m = Model::from_raw(raw, "X:\\".into(), 1);
        let found = |q: &str| {
            let mut names: Vec<String> = m
                .search(0, q, Metric::Logical, 10, false, ItemKind::All)
                .ids
                .into_iter()
                .map(|i| m.name(i).to_string())
                .collect();
            names.sort();
            names
        };
        assert_eq!(found("mp4|mkv"), ["a 2010.mp4", "b 2010.mkv", "c 2011.mkv"]);
        assert_eq!(found("mp4 | MKV"), found("mp4|mkv"));
        assert_eq!(found("*.mp4;*.mkv"), found("mp4|mkv"));
        assert_eq!(found("mp4; mkv|avi 2010"), ["a 2010.mp4", "b 2010.mkv", "d 2010.avi"]);
        // `|` binds tighter than a space.
        assert_eq!(found("*.mp4|*.mkv 2010"), ["a 2010.mp4", "b 2010.mkv"]);
        assert_eq!(found("2010 avi|mp4|"), ["a 2010.mp4", "d 2010.avi"]);
        // Stray `|` is ignored, a quoted one is literal.
        assert_eq!(found("| avi"), ["d 2010.avi"]);
        assert_eq!(found("\"e|f\""), ["e|f.txt"]);
    }

    #[test]
    fn match_ranges_for_display() {
        let m = |q: &str, whole_word, name: &str| -> Vec<(usize, usize)> {
            match_ranges(q, whole_word, name).into_iter().map(|r| (r.start, r.end)).collect()
        };
        assert_eq!(m("report 2025", false, "Report-2025.pdf"), [(0, 6), (7, 11)]);
        // Byte ranges of the original characters, whatever their case.
        assert_eq!(m("отчёт", false, "Мой ОТЧЁТ.doc"), [(7, 17)]);
        // Every occurrence; whole words skip those inside a word.
        assert_eq!(m("py", false, "pyd.py"), [(0, 2), (4, 6)]);
        assert_eq!(m("py", true, "pyd.py"), [(4, 6)]);
        // Overlaps merge; alternatives and quoted phrases count.
        assert_eq!(m("ab bc", false, "abc"), [(0, 3)]);
        assert_eq!(m("mp4|mkv \"a b\"", false, "a b.mkv"), [(0, 3), (4, 7)]);
        // Masks are left out.
        assert!(m("*.mp4", false, "a.mp4").is_empty());
        assert!(m("", false, "a").is_empty());
    }

    #[test]
    fn wildcard_matching() {
        let w = |p: &str, t: &str| wildcard(p.as_bytes(), t.as_bytes(), |a, b| a == b);
        assert!(w("", ""));
        assert!(!w("", "a"));
        assert!(w("*", ""));
        assert!(w("**a**", "a"));
        assert!(w("a*b*c", "aXbYbZc"));
        assert!(!w("a*b*c", "aXbYbZ"));
        assert!(w("?", "a"));
        assert!(!w("?", ""));
        assert!(w("*a?c", "abcabc"));
        assert!(w("*.mp4", ".mp4") && !w("*.mp4", "mp4") && !w("*.mp4", "a.mp4x"));
        assert!(w("a*?c", "abc") && !w("a*?c", "ac"));
        assert!(w("*b*", "abc") && w("a*", "a"));
    }

    #[test]
    fn without_removes_a_subtree_and_updates_totals() {
        let raw = RawDir {
            name: "root".into(),
            files: vec![file("top.bin", 40)],
            subdirs: vec![
                RawDir {
                    name: "a".into(),
                    files: vec![file("a1", 30), file("a2", 20)],
                    subdirs: vec![RawDir {
                        name: "deep".into(),
                        files: vec![file("d1", 5)],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                RawDir {
                    name: "b".into(),
                    files: vec![file("b1", 50)],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let m = Model::from_raw(raw, "X:\\".into(), 1);
        let names = |m: &Model, id| m.children(id).map(|c| m.name(c).to_string()).collect::<Vec<_>>();
        assert_eq!(names(&m, 0), ["a", "b", "top.bin"]);

        // A file: its folder and the root shrink, and "a" drops below "b".
        let a = m.find_dir(&["a"]);
        let a1 = m.children(a).find(|&c| m.name(c) == "a1").unwrap();
        let n = m.without(a1);
        assert_eq!(n.len(), m.len() - 1);
        assert_eq!((n.node(0).size, n.node(0).files, n.node(0).dirs), (115, 4, 3));
        assert_eq!(names(&n, 0), ["b", "top.bin", "a"]);
        let na = n.find_dir(&["a"]);
        assert_eq!((n.node(na).size, n.node(na).files), (25, 2));
        assert_eq!(n.path(n.find_dir(&["a", "deep"])), "X:\\a\\deep");

        // A folder goes with everything inside it.
        let n = m.without(m.find_dir(&["a", "deep"]));
        assert_eq!((n.node(0).size, n.node(0).files, n.node(0).dirs), (140, 4, 2));
        assert_eq!(n.find_dir(&["a", "deep"]), n.find_dir(&["a"]));
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
