pub mod about;
pub mod chart;
pub mod errors;
pub mod files;
pub mod search;
pub mod toolbar;
pub mod tree;

use egui::{Button, Color32, RichText, Ui};

/// Colour of destructive actions.
pub const DANGER: Color32 = Color32::from_rgb(200, 40, 40);

/// What an item's context menu asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemCommand {
    OpenInExplorer(u32),
    Properties(u32),
    /// Ask, then move to the Recycle Bin.
    Delete(u32),
}

/// Context menu of item `id`, the same in the chart, the tree and the
/// lists. The scan root (id 0) cannot be deleted.
pub fn item_menu(ui: &mut Ui, id: u32) -> Option<ItemCommand> {
    let mut command = None;
    if ui.button("Open in Explorer").clicked() {
        command = Some(ItemCommand::OpenInExplorer(id));
    }
    if ui.button("Properties").clicked() {
        command = Some(ItemCommand::Properties(id));
    }
    ui.separator();
    let delete = Button::new(RichText::new("Delete…").color(DANGER));
    if ui
        .add_enabled(id != 0, delete)
        .on_hover_text("Move to the Recycle Bin")
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
