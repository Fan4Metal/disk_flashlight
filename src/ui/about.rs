//! "About" dialog: icon, name, version, a short description and links.

use egui::{Align, Layout, TextureHandle, TextureOptions, Ui};

use crate::VERSION;

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const LICENSE: &str = env!("CARGO_PKG_LICENSE");
/// Displayed icon size in points.
const ICON_SIZE: f32 = 64.0;

#[derive(Default)]
pub struct AboutDialog {
    pub open: bool,
    /// Rasterised on first show, at the display's pixel density.
    icon: Option<TextureHandle>,
}

impl AboutDialog {
    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let icon = self
            .icon
            .get_or_insert_with(|| {
                let px = (ICON_SIZE * ctx.pixels_per_point()).round() as u32;
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [px as usize, px as usize],
                    &crate::icon::rgba(px),
                );
                ctx.load_texture("about_icon", image, TextureOptions::LINEAR)
            })
            .clone();

        let modal = egui::Modal::new(egui::Id::new("about")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.vertical_centered(|ui| {
                ui.add_space(4.0);
                ui.image((icon.id(), egui::vec2(ICON_SIZE, ICON_SIZE)));
                ui.add_space(6.0);
                ui.heading("Disk Flashlight");
                ui.label(format!("Version {VERSION}"));
                ui.add_space(8.0);
                ui.label("Disk space analyzer for Windows with a sunburst chart.");
                ui.add_space(8.0);
                ui.hyperlink_to("Homepage", REPOSITORY)
                    .on_hover_text(REPOSITORY);
                ui.weak(format!("{LICENSE} License"));
                ui.add_space(8.0);
            });
            close_button(ui)
        });
        if modal.inner || modal.should_close() {
            self.open = false;
        }
    }
}

fn close_button(ui: &mut Ui) -> bool {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.button("Close").clicked()
    })
    .inner
}
