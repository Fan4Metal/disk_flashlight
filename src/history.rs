//! Browser-style navigation history over node ids.

use crate::model::NO_NODE;

#[derive(Debug, Default)]
pub struct History {
    pub root: u32,
    back: Vec<u32>,
    fwd: Vec<u32>,
}

impl History {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Translate every id through `f` (e.g. into a rescanned model), dropping
    /// entries that collapse onto their neighbour or onto the new root.
    pub fn remap(&mut self, mut f: impl FnMut(u32) -> u32) {
        self.root = f(self.root);
        for stack in [&mut self.back, &mut self.fwd] {
            for id in stack.iter_mut() {
                *id = f(*id);
            }
            stack.dedup();
            if stack.last() == Some(&self.root) {
                stack.pop();
            }
        }
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.fwd.is_empty()
    }

    /// Jump to `id`, recording the current root. Returns `true` if the root
    /// actually changed.
    pub fn navigate(&mut self, id: u32) -> bool {
        if id == self.root || id == NO_NODE {
            return false;
        }
        self.back.push(self.root);
        self.fwd.clear();
        self.root = id;
        true
    }

    pub fn back(&mut self) -> bool {
        let Some(prev) = self.back.pop() else {
            return false;
        };
        self.fwd.push(self.root);
        self.root = prev;
        true
    }

    pub fn forward(&mut self) -> bool {
        let Some(next) = self.fwd.pop() else {
            return false;
        };
        self.back.push(self.root);
        self.root = next;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigate_back_forward() {
        let mut h = History::default();
        assert!(!h.navigate(0)); // already there
        assert!(!h.navigate(NO_NODE));
        assert!(h.navigate(5));
        assert!(h.navigate(9));
        assert_eq!(h.root, 9);
        assert!(h.can_back() && !h.can_forward());
        assert!(h.back());
        assert_eq!(h.root, 5);
        assert!(h.can_forward());
        assert!(h.back());
        assert_eq!(h.root, 0);
        assert!(!h.back());
        assert!(h.forward());
        assert_eq!(h.root, 5);
        // A new navigation drops the forward stack.
        assert!(h.navigate(7));
        assert!(!h.can_forward());
        assert!(h.back());
        assert_eq!(h.root, 5);
        h.reset();
        assert_eq!(h.root, 0);
        assert!(!h.can_back());
    }

    #[test]
    fn remap_collapses_duplicates() {
        let mut h = History::default();
        for id in [1, 2, 3, 4] {
            h.navigate(id);
        }
        h.back(); // back: 0 1 2, root 3, fwd: 4
        // 2 and 3 vanished and fall back to 1; 4 becomes 40.
        h.remap(|id| match id {
            2 | 3 => 1,
            4 => 40,
            x => x,
        });
        assert_eq!(h.root, 1);
        assert_eq!(h.back, vec![0]);
        assert_eq!(h.fwd, vec![40]);
    }
}
