//! "About" dialog: icon, name, version, a short description, links and the
//! interface language.

use egui::{Align, Layout, TextureHandle, Ui};

use crate::VERSION;
use crate::i18n::{LangChoice, set_lang, system_lang};

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const LICENSE: &str = env!("CARGO_PKG_LICENSE");
/// Displayed icon size in points.
const ICON_SIZE: f32 = 64.0;

#[derive(Default)]
pub struct AboutDialog {
    pub open: bool,
    /// Interface language (kept in the settings).
    pub lang: LangChoice,
    /// Rasterised on first show, at the display's pixel density.
    icon: Option<TextureHandle>,
}

/// Name of a language choice; the languages in their own language, so
/// that they can be found whatever the interface shows.
fn lang_label(choice: LangChoice) -> String {
    match choice {
        LangChoice::System => {
            let own = match system_lang() {
                crate::i18n::Lang::En => "English",
                crate::i18n::Lang::Ru => "Русский",
            };
            tr!(format!("As in Windows ({own})"), format!("Как в Windows ({own})"))
        }
        LangChoice::En => "English".into(),
        LangChoice::Ru => "Русский".into(),
    }
}

impl AboutDialog {
    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let icon = super::app_icon(ctx, ICON_SIZE, &mut self.icon);

        let modal = egui::Modal::new(egui::Id::new("about")).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.vertical_centered(|ui| {
                ui.add_space(4.0);
                ui.image((icon.id(), egui::vec2(ICON_SIZE, ICON_SIZE)));
                ui.add_space(6.0);
                ui.heading("Disk Flashlight");
                ui.label(tr!(format!("Version {VERSION}"), format!("Версия {VERSION}")));
                ui.add_space(8.0);
                ui.label(tr!(
                    "Disk space analyzer for Windows with a sunburst chart.",
                    "Анализатор занятого места на дисках Windows с круговой диаграммой."
                ));
                ui.add_space(8.0);
                ui.hyperlink_to(tr!("Homepage", "Сайт проекта"), REPOSITORY)
                    .on_hover_text(REPOSITORY);
                ui.weak(tr!(format!("{LICENSE} License"), format!("Лицензия {LICENSE}")));
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label(tr!("Language", "Язык"));
                    let mut choice = self.lang;
                    egui::ComboBox::from_id_salt("language")
                        .selected_text(lang_label(choice))
                        .show_ui(ui, |ui| {
                            for c in [LangChoice::System, LangChoice::En, LangChoice::Ru] {
                                ui.selectable_value(&mut choice, c, lang_label(c));
                            }
                        });
                    if choice != self.lang {
                        self.lang = choice;
                        set_lang(choice.resolve());
                    }
                });
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
        ui.button(tr!("Close", "Закрыть")).clicked()
    })
    .inner
}
