//! Path field of the toolbar, like Explorer's address bar: it shows the
//! current path, turns into a text field on a click, offers the matching
//! folders below while a path is typed, and Enter scans it.

use std::path::{Path, PathBuf};

use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditState;
use egui::{Id, Key, Modifiers, RichText, Ui};

use crate::scan::win::Drive;

/// At most this many folders are offered.
const MAX_SUGGESTIONS: usize = 12;

#[derive(Default)]
pub struct PathField {
    /// Typing a path while a scan result is shown (with none, the field is
    /// always editable).
    editing: bool,
    /// Focus the field and select its text on the next frame.
    focus: bool,
    /// Folder whose subfolders are in `names`, as typed (with the
    /// trailing `\`), so it is read once per folder rather than per key.
    listed: Option<String>,
    names: Vec<String>,
    /// Suggestion picked with the arrow keys.
    selected: Option<usize>,
    /// The pointer was on the suggestion list last frame: a click there
    /// takes the focus from the field but must not close the list.
    list_hovered: bool,
}

impl PathField {
    /// `text` is the path being typed; `current` the path of the chart's
    /// centre, shown when not editing; `remote` tells network paths, which
    /// are not listed (a server that is away could stall the window).
    /// Returns a path to scan.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        text: &mut String,
        current: Option<String>,
        drives: &[Drive],
        remote: impl Fn(&Path) -> bool,
    ) -> Option<PathBuf> {
        if let Some(current) = &current
            && !self.editing
        {
            let label = ui
                .add(
                    egui::Label::new(RichText::new(current).monospace())
                        .truncate()
                        .sense(egui::Sense::click()),
                )
                .on_hover_cursor(egui::CursorIcon::Text)
                .on_hover_text(tr!(
                    "Click to type another path to scan",
                    "Щёлкните, чтобы ввести другой путь для сканирования"
                ));
            if label.clicked() {
                self.editing = true;
                self.focus = true;
                *text = current.clone();
            }
            return None;
        }

        let id = ui.id().with("path_field");
        let focused = ui.memory(|m| m.has_focus(id));
        let suggestions = if focused || self.list_hovered {
            self.suggestions(text, drives, &remote)
        } else {
            Vec::new()
        };
        let mut accept: Option<String> = None;
        // Keys for the list are taken before the field sees them: arrows
        // move in the list, Tab or Enter on a picked item takes it.
        if focused && !suggestions.is_empty() {
            let n = suggestions.len();
            let (down, up, tab) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowDown),
                    i.consume_key(Modifiers::NONE, Key::ArrowUp),
                    i.consume_key(Modifiers::NONE, Key::Tab),
                )
            });
            if down {
                self.selected = Some(self.selected.map_or(0, |s| (s + 1) % n));
            }
            if up {
                self.selected = Some(self.selected.map_or(n - 1, |s| (s + n - 1) % n));
            }
            if tab {
                accept = Some(suggestions[self.selected.unwrap_or(0)].clone());
            }
            if let Some(s) = self.selected
                && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter))
            {
                accept = suggestions.get(s).cloned();
            }
        }

        let mut go = false;
        let field = ui
            .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                go = ui.button(tr!("Scan", "Сканировать")).clicked();
                ui.add(
                    egui::TextEdit::singleline(text)
                        .id(id)
                        .desired_width(ui.available_width())
                        .hint_text(tr!(r"Path to scan, e.g. D:\Projects", r"Путь для сканирования, например D:\Projects")),
                )
            })
            .inner;
        if field.changed() {
            self.selected = None;
        }
        go |= field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));

        // The list below the field.
        if !suggestions.is_empty() && (focused || self.list_hovered) {
            let area = egui::Area::new(id.with("suggestions"))
                .order(egui::Order::Foreground)
                .fixed_pos(field.rect.left_bottom() + egui::vec2(0.0, 2.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(field.rect.width());
                        for (i, s) in suggestions.iter().enumerate() {
                            let row = ui.selectable_label(
                                self.selected == Some(i),
                                RichText::new(s).monospace(),
                            );
                            if row.clicked() {
                                accept = Some(s.clone());
                            }
                        }
                    });
                });
            self.list_hovered = area.response.contains_pointer();
        } else {
            self.list_hovered = false;
        }

        if let Some(path) = accept {
            // Into the folder: typing goes on after its backslash.
            *text = format!("{}\\", path.trim_end_matches('\\'));
            self.selected = None;
            self.list_hovered = false;
            place_cursor(ui.ctx(), id, text.chars().count(), false);
            field.request_focus();
        } else if std::mem::take(&mut self.focus) {
            place_cursor(ui.ctx(), id, text.chars().count(), true);
            field.request_focus();
        } else if field.lost_focus() && !self.list_hovered && !go {
            // Esc or a click elsewhere: back to showing the current path.
            self.editing = false;
        }

        let path = text.trim();
        if go && !path.is_empty() {
            self.editing = false;
            self.selected = None;
            return Some(PathBuf::from(path));
        }
        None
    }

    /// Full paths offered for `text`: drives while no folder is typed yet,
    /// then the subfolders of the typed folder whose names start with what
    /// follows its last backslash.
    fn suggestions(&mut self, text: &str, drives: &[Drive], remote: &impl Fn(&Path) -> bool) -> Vec<String> {
        let text = text.trim_start();
        if text.is_empty() {
            return Vec::new();
        }
        let Some(cut) = text.rfind(['\\', '/']) else {
            return drives
                .iter()
                .filter(|d| starts_with_ignore_case(&d.root, text))
                .map(|d| d.root.clone())
                .take(MAX_SUGGESTIONS)
                .collect();
        };
        let (folder, prefix) = text.split_at(cut + 1);
        if self.listed.as_deref() != Some(folder) {
            self.names = if remote(Path::new(folder)) {
                Vec::new()
            } else {
                list_folders(folder)
            };
            self.listed = Some(folder.to_string());
            self.selected = None;
        }
        self.names
            .iter()
            .filter(|n| starts_with_ignore_case(n, prefix))
            .map(|n| format!("{folder}{n}"))
            .take(MAX_SUGGESTIONS)
            .collect()
    }
}

/// Subfolder names of `folder`, sorted ignoring case; none if it cannot be
/// read.
fn list_folders(folder: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

fn starts_with_ignore_case(name: &str, prefix: &str) -> bool {
    let mut name = name.chars().flat_map(char::to_lowercase);
    prefix
        .chars()
        .flat_map(char::to_lowercase)
        .all(|p| name.next() == Some(p))
}

/// Put the cursor of text field `id` at the end of its `len` characters,
/// or select them all.
fn place_cursor(ctx: &egui::Context, id: Id, len: usize, select_all: bool) {
    let mut state = TextEditState::load(ctx, id).unwrap_or_default();
    let range = if select_all {
        CCursorRange::two(CCursor::new(0), CCursor::new(len))
    } else {
        CCursorRange::one(CCursor::new(len))
    };
    state.cursor.set_char_range(Some(range));
    state.store(ctx, id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_ignores_case() {
        assert!(starts_with_ignore_case("Projects", "pro"));
        assert!(starts_with_ignore_case("Отчёты", "ОТЧ"));
        assert!(starts_with_ignore_case("abc", ""));
        assert!(!starts_with_ignore_case("ab", "abc"));
        assert!(!starts_with_ignore_case("Projects", "roj"));
    }

    #[test]
    fn folders_of_a_typed_path() {
        let dir = std::env::temp_dir().join(format!("df_path_field_{}", std::process::id()));
        for sub in ["Alpha", "alps", "Beta"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        std::fs::write(dir.join("almanac.txt"), b"").unwrap();
        let mut f = PathField::default();
        let base = format!("{}\\", dir.display());
        let found = f.suggestions(&format!("{base}AL"), &[], &|_| false);
        // Folders only, sorted ignoring case, as full paths.
        assert_eq!(found, [format!("{base}Alpha"), format!("{base}alps")]);
        // Network paths are not listed.
        assert!(f.suggestions(r"\\server\share\", &[], &|_| true).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
