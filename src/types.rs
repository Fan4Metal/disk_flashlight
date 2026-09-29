//! File types: the extension of every file, totals by type under a folder,
//! and which types get a colour of their own on the chart.

use std::collections::HashMap;

use crate::model::{Metric, Model};

/// The type of a folder (folders have none).
pub const NOT_A_FILE: u16 = u16::MAX;
/// The type of files without an extension.
pub const NO_EXTENSION: u16 = 0;
/// Number of types, the largest of the whole scan, that get a colour of
/// their own; the others share one.
pub const COLOURED: usize = 12;
/// Rank of a type without a colour of its own.
pub const OTHER: u8 = u8::MAX;
/// Longer "extensions" are part of a name with dots rather than a type.
const MAX_EXT_LEN: usize = 16;

/// Extension of a file `name`, without the dot, as written: `mp4` for
/// `film.MP4`, `gz` for `a.tar.gz`; `None` for `.gitignore`, `name.`,
/// and a last part with a space or longer than 16 characters (`v1.0 final`).
pub fn extension(name: &str) -> Option<&str> {
    let dot = name.rfind('.')?;
    let ext = &name[dot + 1..];
    (dot > 0 && !ext.is_empty() && ext.chars().count() <= MAX_EXT_LEN && !ext.contains(' '))
        .then_some(ext)
}

/// Totals of one type under a folder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TypeStat {
    pub ty: u16,
    /// Logical size and space on disk of the files.
    pub size: u64,
    pub alloc: u64,
    pub files: u64,
}

impl TypeStat {
    pub fn metric(&self, m: Metric) -> u64 {
        match m {
            Metric::Logical => self.size,
            Metric::Physical => self.alloc,
        }
    }
}

#[derive(Debug, Default)]
pub struct FileTypes {
    /// Per node: its type, `NOT_A_FILE` for folders.
    of: Vec<u16>,
    /// Extension of each type, lowercase, without the dot; `""` for
    /// `NO_EXTENSION`.
    names: Vec<Box<str>>,
    /// Per type: its place among the largest types of the whole scan by
    /// logical size (a stable choice, whatever the metric or the folder
    /// shown), or `OTHER` past `COLOURED`.
    rank: Vec<u8>,
}

impl FileTypes {
    /// Types of all files of `model`; extensions differing only in case
    /// are one type.
    pub fn build(model: &Model) -> Self {
        let mut index: HashMap<Box<str>, u16> = HashMap::new();
        let mut names: Vec<Box<str>> = vec!["".into()];
        let mut of = Vec::with_capacity(model.len());
        let mut lower = String::new();
        for id in 0..model.len() as u32 {
            if model.node(id).is_dir {
                of.push(NOT_A_FILE);
                continue;
            }
            let ty = match extension(model.name(id)) {
                None => NO_EXTENSION,
                Some(ext) => {
                    lower.clear();
                    if ext.is_ascii() {
                        lower.extend(ext.chars().map(|c| c.to_ascii_lowercase()));
                    } else {
                        lower.push_str(&ext.to_lowercase());
                    }
                    match index.get(lower.as_str()) {
                        Some(&ty) => ty,
                        // Tens of thousands of distinct extensions: the rest
                        // count as none.
                        None if names.len() >= NOT_A_FILE as usize => NO_EXTENSION,
                        None => {
                            let ty = names.len() as u16;
                            names.push(lower.as_str().into());
                            index.insert(lower.as_str().into(), ty);
                            ty
                        }
                    }
                }
            };
            of.push(ty);
        }
        let mut types = FileTypes {
            of,
            rank: vec![OTHER; names.len()],
            names,
        };
        if !model.is_empty() {
            let top = types.stats(model, 0, Metric::Logical);
            for (i, s) in top.iter().take(COLOURED).enumerate() {
                types.rank[s.ty as usize] = i as u8;
            }
        }
        types
    }

    /// Type of node `id`, `NOT_A_FILE` for a folder.
    #[inline]
    pub fn of(&self, id: u32) -> u16 {
        self.of.get(id as usize).copied().unwrap_or(NOT_A_FILE)
    }

    /// Extension of type `ty`, lowercase, without the dot (`""` for none).
    pub fn name(&self, ty: u16) -> &str {
        self.names.get(ty as usize).map_or("", |n| n)
    }

    /// Colour slot of type `ty` (see `rank`), `OTHER` if it has none.
    #[inline]
    pub fn rank(&self, ty: u16) -> u8 {
        self.rank.get(ty as usize).copied().unwrap_or(OTHER)
    }

    /// The types with a colour of their own, in the order of their slots.
    pub fn coloured(&self) -> Vec<u16> {
        let mut out = vec![NOT_A_FILE; COLOURED];
        for (ty, &r) in self.rank.iter().enumerate() {
            if (r as usize) < COLOURED {
                out[r as usize] = ty as u16;
            }
        }
        out.retain(|&t| t != NOT_A_FILE);
        out
    }

    /// Totals of every type present under `root`, largest by `metric`
    /// first (then by type, so the order is stable).
    pub fn stats(&self, model: &Model, root: u32, metric: Metric) -> Vec<TypeStat> {
        let mut by_type = vec![TypeStat::default(); self.names.len()];
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            for c in model.children(dir) {
                let n = model.node(c);
                if n.is_dir {
                    stack.push(c);
                } else {
                    let s = &mut by_type[self.of(c) as usize];
                    s.size += n.size;
                    s.alloc += n.alloc;
                    s.files += 1;
                }
            }
        }
        let mut out: Vec<TypeStat> = by_type
            .into_iter()
            .enumerate()
            .filter(|(_, s)| s.files > 0)
            .map(|(ty, s)| TypeStat { ty: ty as u16, ..s })
            .collect();
        out.sort_by_key(|s| (std::cmp::Reverse(s.metric(metric)), s.ty));
        out
    }

    /// Per node: bytes by `metric` of the files of type `ty` in it (its own
    /// size for such a file, the sum for a folder), for colouring the chart
    /// like search matches.
    pub fn hits(&self, model: &Model, ty: u16, metric: Metric) -> Vec<u64> {
        let mut hits = vec![0u64; model.len()];
        for (id, &t) in self.of.iter().enumerate() {
            if t == ty {
                hits[id] = model.node(id as u32).metric(metric);
            }
        }
        // A parent is always packed before its children: summing from the
        // last node back reaches every folder after all of its contents.
        for id in (1..model.len()).rev() {
            if hits[id] > 0 {
                let p = model.node(id as u32).parent as usize;
                hits[p] += hits[id];
            }
        }
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RawDir, RawFile};

    fn f(name: &str, size: u64) -> RawFile {
        RawFile {
            name: name.into(),
            size,
            alloc: size * 2,
            modified: 0,
        }
    }

    #[test]
    fn extensions() {
        assert_eq!(extension("film.MP4"), Some("MP4"));
        assert_eq!(extension("a.tar.gz"), Some("gz"));
        assert_eq!(extension("фото.jpeg"), Some("jpeg"));
        assert_eq!(extension(".gitignore"), None);
        assert_eq!(extension("README"), None);
        assert_eq!(extension("name."), None);
        assert_eq!(extension("Report v1.0 final"), None);
        assert_eq!(extension("x.aaaaaaaaaaaaaaaaa"), None);
        assert_eq!(extension("x.файл"), Some("файл"));
    }

    fn model() -> Model {
        let raw = RawDir {
            name: "root".into(),
            files: vec![f("a.MP4", 100), f("b.txt", 5), f("Makefile", 7)],
            subdirs: vec![RawDir {
                name: "sub".into(),
                files: vec![f("c.mp4", 50), f("d.TXT", 1), f("e.Txt", 2)],
                subdirs: vec![RawDir {
                    name: "deep".into(),
                    files: vec![f("g.txt", 3)],
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        Model::from_raw(raw, "X:\\".into(), 1)
    }

    #[test]
    fn totals_by_type_ignore_case() {
        let m = model();
        let t = &m.types;
        let stats = t.stats(&m, 0, Metric::Logical);
        let named: Vec<(&str, u64, u64)> = stats.iter().map(|s| (t.name(s.ty), s.size, s.files)).collect();
        assert_eq!(named, [("mp4", 150, 2), ("txt", 11, 4), ("", 7, 1)]);
        assert_eq!(stats[0].alloc, 300);
        // Under a folder only its files count.
        let sub = m.find_dir(&["sub"]);
        let named: Vec<(&str, u64)> = t.stats(&m, sub, Metric::Logical).iter().map(|s| (t.name(s.ty), s.size)).collect();
        assert_eq!(named, [("mp4", 50), ("txt", 6)]);
        // Folders have no type; ranks follow the size over the whole scan.
        assert_eq!(t.of(sub), NOT_A_FILE);
        let mp4 = stats[0].ty;
        assert_eq!((t.rank(mp4), t.rank(stats[1].ty), t.rank(NO_EXTENSION)), (0, 1, 2));
        assert_eq!(t.coloured(), [mp4, stats[1].ty, NO_EXTENSION]);
    }

    #[test]
    fn largest_files_of_one_type() {
        let m = model();
        let txt = m.types.stats(&m, 0, Metric::Logical)[1].ty;
        let top = |root, limit| -> Vec<&str> {
            m.largest_files_where(root, Metric::Logical, limit, |id, _| m.types.of(id) == txt)
                .into_iter()
                .map(|id| m.name(id))
                .collect()
        };
        assert_eq!(top(0, 10), ["b.txt", "g.txt", "e.Txt", "d.TXT"]);
        assert_eq!(top(0, 2), ["b.txt", "g.txt"]);
        assert_eq!(top(m.find_dir(&["sub"]), 10), ["g.txt", "e.Txt", "d.TXT"]);
    }

    #[test]
    fn hits_add_up_to_the_folders() {
        let m = model();
        let t = &m.types;
        let txt = t.stats(&m, 0, Metric::Logical)[1].ty;
        let hits = t.hits(&m, txt, Metric::Logical);
        assert_eq!(hits[0], 11);
        assert_eq!(hits[m.find_dir(&["sub"]) as usize], 6);
        assert_eq!(hits[m.find_dir(&["sub", "deep"]) as usize], 3);
        // Brute force: every node's share is the sum of the matching files
        // at or below it.
        for id in 0..m.len() as u32 {
            let mut want = 0;
            let mut stack = vec![id];
            while let Some(n) = stack.pop() {
                if m.node(n).is_dir {
                    stack.extend(m.children(n));
                } else if t.of(n) == txt {
                    want += m.node(n).size;
                }
            }
            assert_eq!(hits[id as usize], want, "node {}", m.name(id));
        }
    }
}
