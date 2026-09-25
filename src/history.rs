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
}
