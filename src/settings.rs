//! Settings kept between runs in eframe's storage
//! (`%APPDATA%\Disk Flashlight\data\app.ron`, or [`PORTABLE_FILE`] next to
//! the exe, see [`location`]). Window size and position and egui's own state
//! (panel widths) are saved by eframe itself.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::i18n::LangChoice;
use crate::layout::{DEFAULT_RINGS, MAX_RINGS, MIN_RINGS};
use crate::model::{ItemKind, Metric};
use crate::render::ColorMode;
use crate::ui::SideView;
use crate::ui::files::ListSort;

const METRIC: &str = "metric";
const THEME: &str = "theme";
const LANGUAGE: &str = "language";
const COLOR_MODE: &str = "color_mode";
const RINGS: &str = "rings";
const FOLLOW_IN_TREE: &str = "follow_in_tree";
const LAST_PATH: &str = "last_path";
const RECENT_PATHS: &str = "recent_paths";
const SIDE_VIEW: &str = "side_view";
const SEARCH_WHOLE_WORD: &str = "search_whole_word";
const SEARCH_KIND: &str = "search_kind";
const SEARCH_SORT: &str = "search_sort";
const FILES_SORT: &str = "files_sort";
const FILES_OLDER_THAN: &str = "files_older_than";
const CHECK_UPDATES: &str = "check_updates";
const LAST_UPDATE_CHECK: &str = "last_update_check";

/// Settings file that makes a copy portable: when it lies next to the exe
/// (even empty) and can be written, the settings are kept in it instead of
/// the user profile. The portable zip ships an empty one.
pub const PORTABLE_FILE: &str = "disk_flashlight.ron";

/// Where the settings of this run are kept.
#[derive(Debug)]
pub struct Location {
    /// The settings file, `None` if there is nowhere to keep it.
    pub file: Option<PathBuf>,
    /// `file` is [`PORTABLE_FILE`] next to the exe.
    pub portable: bool,
}

impl Location {
    /// Whether settings of an earlier run are there (a portable file starts
    /// empty).
    pub fn has_saved(&self) -> bool {
        self.file
            .as_ref()
            .is_some_and(|f| f.metadata().is_ok_and(|m| m.len() > 0))
    }
}

/// Where the settings are kept, decided once per run.
pub fn location() -> &'static Location {
    static LOCATION: OnceLock<Location> = OnceLock::new();
    LOCATION.get_or_init(|| {
        let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
        match exe_dir.and_then(|d| portable_file_in(&d)) {
            Some(file) => Location { file: Some(file), portable: true },
            None => Location {
                file: eframe::storage_dir(crate::APP_ID).map(|d| d.join("app.ron")),
                portable: false,
            },
        }
    })
}

/// [`PORTABLE_FILE`] in `dir` if it is there and can be written; a copy on
/// a read-only medium keeps its settings in the profile instead of losing
/// them.
fn portable_file_in(dir: &Path) -> Option<PathBuf> {
    let file = dir.join(PORTABLE_FILE);
    // Opening for appending checks the permission without changing the file.
    std::fs::OpenOptions::new().append(true).open(&file).ok().map(|_| file)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Light, dark or as in Windows.
    pub theme: egui::ThemePreference,
    /// Interface language, or the one of Windows.
    pub language: LangChoice,
    pub metric: Metric,
    pub color_mode: ColorMode,
    /// Number of rings of the chart.
    pub rings: usize,
    pub follow_in_tree: bool,
    /// What the left panel shows.
    pub side_view: SideView,
    /// Search matches whole words only.
    pub search_whole_word: bool,
    /// Search lists files, folders or both.
    pub search_kind: ItemKind,
    /// Order of the search results.
    pub search_sort: ListSort,
    /// Order of the Largest files list.
    pub files_sort: ListSort,
    /// Largest files lists only files older than this many years (0: all).
    pub files_older_than: u32,
    /// Path of the last scan, offered in the path field on the next start.
    pub last_path: Option<String>,
    /// Folders scanned recently, newest first (the drive picker).
    pub recent: Vec<String>,
    /// Ask GitHub for a newer release at start-up (off unless enabled).
    pub check_updates: bool,
    /// Unix seconds of the last update check that got an answer.
    pub last_update_check: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: egui::ThemePreference::System,
            language: LangChoice::System,
            metric: Metric::Physical,
            color_mode: ColorMode::default(),
            rings: DEFAULT_RINGS,
            follow_in_tree: true,
            side_view: SideView::default(),
            search_whole_word: false,
            search_kind: ItemKind::All,
            search_sort: ListSort::Size,
            files_sort: ListSort::Size,
            files_older_than: 0,
            last_path: None,
            recent: Vec::new(),
            check_updates: false,
            last_update_check: 0,
        }
    }
}

impl Settings {
    /// Read settings; missing or unknown values keep their defaults.
    pub fn load(storage: &dyn eframe::Storage) -> Self {
        let d = Self::default();
        let get = |key| storage.get_string(key);
        Self {
            theme: match get(THEME).as_deref() {
                Some("system") => egui::ThemePreference::System,
                Some("light") => egui::ThemePreference::Light,
                Some("dark") => egui::ThemePreference::Dark,
                _ => d.theme,
            },
            language: match get(LANGUAGE).as_deref() {
                Some("system") => LangChoice::System,
                Some("en") => LangChoice::En,
                Some("ru") => LangChoice::Ru,
                _ => d.language,
            },
            metric: match get(METRIC).as_deref() {
                Some("physical") => Metric::Physical,
                Some("logical") => Metric::Logical,
                _ => d.metric,
            },
            color_mode: match get(COLOR_MODE).as_deref() {
                Some("size") => ColorMode::Size,
                Some("depth") => ColorMode::Depth,
                Some("age") => ColorMode::Age,
                Some("type") => ColorMode::Type,
                _ => d.color_mode,
            },
            rings: get(RINGS)
                .and_then(|v| v.parse().ok())
                .filter(|n| (MIN_RINGS..=MAX_RINGS).contains(n))
                .unwrap_or(d.rings),
            follow_in_tree: match get(FOLLOW_IN_TREE).as_deref() {
                Some("true") => true,
                Some("false") => false,
                _ => d.follow_in_tree,
            },
            side_view: match get(SIDE_VIEW).as_deref() {
                Some("folders") => SideView::Folders,
                Some("largest_files") => SideView::LargestFiles,
                Some("search") => SideView::Search,
                Some("types") => SideView::Types,
                _ => d.side_view,
            },
            search_whole_word: match get(SEARCH_WHOLE_WORD).as_deref() {
                Some("true") => true,
                Some("false") => false,
                _ => d.search_whole_word,
            },
            search_kind: match get(SEARCH_KIND).as_deref() {
                Some("all") => ItemKind::All,
                Some("files") => ItemKind::Files,
                Some("folders") => ItemKind::Folders,
                _ => d.search_kind,
            },
            search_sort: get(SEARCH_SORT).as_deref().and_then(sort_from).unwrap_or(d.search_sort),
            files_sort: get(FILES_SORT).as_deref().and_then(sort_from).unwrap_or(d.files_sort),
            files_older_than: get(FILES_OLDER_THAN)
                .and_then(|v| v.parse().ok())
                .filter(|&y: &u32| y <= 100)
                .unwrap_or(d.files_older_than),
            last_path: get(LAST_PATH).filter(|p| !p.is_empty()),
            // One path per line: Windows paths cannot hold a line break.
            recent: get(RECENT_PATHS)
                .map(|v| {
                    v.lines()
                        .filter(|p| !p.trim().is_empty())
                        .take(crate::app::MAX_RECENT)
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default(),
            check_updates: match get(CHECK_UPDATES).as_deref() {
                Some("true") => true,
                Some("false") => false,
                _ => d.check_updates,
            },
            last_update_check: get(LAST_UPDATE_CHECK)
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.last_update_check),
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
            ColorMode::Age => "age",
            ColorMode::Type => "type",
        };
        storage.set_string(METRIC, metric.into());
        let theme = match self.theme {
            egui::ThemePreference::System => "system",
            egui::ThemePreference::Light => "light",
            egui::ThemePreference::Dark => "dark",
        };
        storage.set_string(THEME, theme.into());
        let language = match self.language {
            LangChoice::System => "system",
            LangChoice::En => "en",
            LangChoice::Ru => "ru",
        };
        storage.set_string(LANGUAGE, language.into());
        storage.set_string(COLOR_MODE, color_mode.into());
        storage.set_string(RINGS, self.rings.to_string());
        let side_view = match self.side_view {
            SideView::Folders => "folders",
            SideView::LargestFiles => "largest_files",
            SideView::Search => "search",
            SideView::Types => "types",
        };
        storage.set_string(FOLLOW_IN_TREE, self.follow_in_tree.to_string());
        storage.set_string(SIDE_VIEW, side_view.into());
        storage.set_string(SEARCH_WHOLE_WORD, self.search_whole_word.to_string());
        let search_kind = match self.search_kind {
            ItemKind::All => "all",
            ItemKind::Files => "files",
            ItemKind::Folders => "folders",
        };
        storage.set_string(SEARCH_KIND, search_kind.into());
        storage.set_string(SEARCH_SORT, sort_name(self.search_sort).into());
        storage.set_string(FILES_SORT, sort_name(self.files_sort).into());
        storage.set_string(FILES_OLDER_THAN, self.files_older_than.to_string());
        storage.set_string(LAST_PATH, self.last_path.clone().unwrap_or_default());
        storage.set_string(RECENT_PATHS, self.recent.join("\n"));
        storage.set_string(CHECK_UPDATES, self.check_updates.to_string());
        storage.set_string(LAST_UPDATE_CHECK, self.last_update_check.to_string());
    }
}

fn sort_name(sort: ListSort) -> &'static str {
    match sort {
        ListSort::Size => "size",
        ListSort::Oldest => "oldest",
        ListSort::Newest => "newest",
    }
}

fn sort_from(name: &str) -> Option<ListSort> {
    match name {
        "size" => Some(ListSort::Size),
        "oldest" => Some(ListSort::Oldest),
        "newest" => Some(ListSort::Newest),
        _ => None,
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
            theme: egui::ThemePreference::Dark,
            language: LangChoice::Ru,
            metric: Metric::Logical,
            color_mode: ColorMode::Age,
            rings: 10,
            follow_in_tree: false,
            side_view: SideView::LargestFiles,
            search_whole_word: true,
            search_kind: ItemKind::Folders,
            search_sort: ListSort::Newest,
            files_sort: ListSort::Oldest,
            files_older_than: 5,
            last_path: Some(r"D:\Projects".into()),
            recent: vec![r"D:\Projects".into(), r"\\server\share\Фото".into()],
            check_updates: true,
            last_update_check: 1_790_000_000,
        };
        let mut storage = MemStorage::default();
        s.save(&mut storage);
        assert_eq!(Settings::load(&storage), s);
    }

    #[test]
    fn portable_file_only_when_present_and_writable() {
        let dir = std::env::temp_dir().join(format!("df_portable_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(PORTABLE_FILE);
        assert_eq!(portable_file_in(&dir), None);
        std::fs::write(&file, "").unwrap();
        assert_eq!(portable_file_in(&dir), Some(file.clone()));
        let mut perm = std::fs::metadata(&file).unwrap().permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(&file, perm.clone()).unwrap();
        assert_eq!(portable_file_in(&dir), None);
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        std::fs::set_permissions(&file, perm).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_or_unknown_values_use_defaults() {
        assert_eq!(Settings::load(&MemStorage::default()), Settings::default());
        let mut storage = MemStorage::default();
        storage.set_string(METRIC, "bogus".into());
        storage.set_string(FOLLOW_IN_TREE, "maybe".into());
        storage.set_string(RINGS, "40".into());
        storage.set_string(FILES_SORT, "sideways".into());
        storage.set_string(FILES_OLDER_THAN, "-1".into());
        storage.set_string(LAST_PATH, String::new());
        storage.set_string(CHECK_UPDATES, "yes".into());
        storage.set_string(LAST_UPDATE_CHECK, "soon".into());
        assert_eq!(Settings::load(&storage), Settings::default());
    }
}
