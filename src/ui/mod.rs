pub mod about;
pub mod chart;
pub mod errors;
pub mod files;
pub mod path_field;
pub mod search;
pub mod toolbar;
pub mod tree;

use egui::{Button, Color32, RichText, TextureHandle, TextureOptions, Ui};

/// The app icon as a texture `points` wide, rasterised at the display's
/// pixel density and kept in `cache`; rasterised again when that density
/// changes (the window moved to a monitor with another scale).
pub fn app_icon(ctx: &egui::Context, points: f32, cache: &mut Option<TextureHandle>) -> TextureHandle {
    let px = (points * ctx.pixels_per_point()).round() as usize;
    if cache.as_ref().is_none_or(|t| t.size() != [px, px]) {
        let image = egui::ColorImage::from_rgba_unmultiplied([px, px], &crate::icon::rgba(px as u32));
        *cache = Some(ctx.load_texture(format!("app_icon_{px}"), image, TextureOptions::LINEAR));
    }
    cache.clone().expect("set above")
}

/// Colour of destructive actions: the fill of their buttons.
pub const DANGER: Color32 = Color32::from_rgb(200, 40, 40);

/// Text colour of a destructive action, readable on either theme.
pub fn danger_text(ui: &Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(240, 110, 110)
    } else {
        DANGER
    }
}

/// What an item's context menu asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemCommand {
    OpenInExplorer(u32),
    Properties(u32),
    /// Put the full path on the clipboard.
    CopyPath(u32),
    /// Ask, then move to the Recycle Bin.
    Delete(u32),
}

/// Context menu of item `id`, the same in the chart, the tree and the
/// lists. The scan root (id 0) cannot be deleted.
pub fn item_menu(ui: &mut Ui, id: u32) -> Option<ItemCommand> {
    let mut command = None;
    if ui.button(tr!("Open in Explorer", "Открыть в Проводнике")).clicked() {
        command = Some(ItemCommand::OpenInExplorer(id));
    }
    if ui.button(tr!("Properties", "Свойства")).clicked() {
        command = Some(ItemCommand::Properties(id));
    }
    if ui
        .add(Button::new(tr!("Copy path", "Копировать путь")).shortcut_text("Ctrl+C"))
        .clicked()
    {
        command = Some(ItemCommand::CopyPath(id));
    }
    ui.separator();
    let delete = Button::new(RichText::new(tr!("Delete…", "Удалить…")).color(danger_text(ui)));
    if ui
        .add_enabled(id != 0, delete)
        .on_hover_text(tr!("Move to the Recycle Bin", "Переместить в корзину"))
        .clicked()
    {
        command = Some(ItemCommand::Delete(id));
    }
    if command.is_some() {
        ui.close();
    }
    command
}

/// What the left panel shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SideView {
    #[default]
    Folders,
    LargestFiles,
    Search,
}
