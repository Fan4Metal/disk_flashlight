//! Settings kept between runs in eframe's storage
//! (`%APPDATA%\Disk Flashlight\data\app.ron`). Window size and position and
//! egui's own state (panel widths) are saved by eframe itself.

use crate::model::Metric;
use crate::render::ColorMode;
use crate::ui::SideView;

const METRIC: &str = "metric";
const COLOR_MODE: &str = "color_mode";
const FOLLOW_IN_TREE: &str = "follow_in_tree";
const LAST_PATH: &str = "last_path";
const SIDE_VIEW: &str = "side_view";

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub metric: Metric,
    pub color_mode: ColorMode,
    pub follow_in_tree: bool,
    /// What the left panel shows.
    pub side_view: SideView,
    /// Path of the last scan, offered in the path field on the next start.
    pub last_path: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            metric: Metric::Physical,
            color_mode: ColorMode::default(),
            follow_in_tree: true,
            side_view: SideView::default(),
            last_path: None,
        }
    }
}

impl Settings {
    /// Read settings; missing or unknown values keep their defaults.
    pub fn load(storage: &dyn eframe::Storage) -> Self {
        let d = Self::default();
        let get = |key| storage.get_string(key);
        Self {
            metric: match get(METRIC).as_deref() {
                Some("physical") => Metric::Physical,
                Some("logical") => Metric::Logical,
                _ => d.metric,
            },
            color_mode: match get(COLOR_MODE).as_deref() {
                Some("size") => ColorMode::Size,
                Some("depth") => ColorMode::Depth,
                _ => d.color_mode,
            },
            follow_in_tree: match get(FOLLOW_IN_TREE).as_deref() {
                Some("true") => true,
                Some("false") => false,
                _ => d.follow_in_tree,
            },
            side_view: match get(SIDE_VIEW).as_deref() {
                Some("folders") => SideView::Folders,
                Some("largest_files") => SideView::LargestFiles,
                Some("search") => SideView::Search,
                _ => d.side_view,
            },
            last_path: get(LAST_PATH).filter(|p| !p.is_empty()),
        }
    }

    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        let metric = match self.metric {
            Metric::Physical => "physical",
            Metric::Logical => "logical",
        };
        let color_mode = match self.color_mode {
            ColorMode::Size => "size",
            ColorMode::Depth => "depth",
        };
        storage.set_string(METRIC, metric.into());
        storage.set_string(COLOR_MODE, color_mode.into());
        let side_view = match self.side_view {
            SideView::Folders => "folders",
            SideView::LargestFiles => "largest_files",
            SideView::Search => "search",
        };
        storage.set_string(FOLLOW_IN_TREE, self.follow_in_tree.to_string());
        storage.set_string(SIDE_VIEW, side_view.into());
        storage.set_string(LAST_PATH, self.last_path.clone().unwrap_or_default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::Storage;
    use std::collections::HashMap;

    #[derive(Default)]
    struct MemStorage(HashMap<String, String>);

    impl Storage for MemStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.into(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }

    #[test]
    fn round_trip() {
        let s = Settings {
            metric: Metric::Logical,
            color_mode: ColorMode::Depth,
            follow_in_tree: false,
            side_view: SideView::LargestFiles,
            last_path: Some(r"D:\Projects".into()),
        };
        let mut storage = MemStorage::default();
        s.save(&mut storage);
        assert_eq!(Settings::load(&storage), s);
    }

    #[test]
    fn missing_or_unknown_values_use_defaults() {
        assert_eq!(Settings::load(&MemStorage::default()), Settings::default());
        let mut storage = MemStorage::default();
        storage.set_string(METRIC, "bogus".into());
        storage.set_string(FOLLOW_IN_TREE, "maybe".into());
        storage.set_string(LAST_PATH, String::new());
        assert_eq!(Settings::load(&storage), Settings::default());
    }
}
