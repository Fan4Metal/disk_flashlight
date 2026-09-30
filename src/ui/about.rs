//! "About" dialog: icon, name, version, a short description, links and the
//! interface language.

use egui::{Align, Layout, TextureHandle, Ui};

use crate::VERSION;
use crate::i18n::{LangChoice, set_lang, system_lang};
use crate::update::{Status, Updates, release_url, version_of};

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const LICENSE: &str = env!("CARGO_PKG_LICENSE");
/// Cargo joins several authors with `:`.
const AUTHORS: &str = env!("CARGO_PKG_AUTHORS");
/// Displayed icon size in points.
const ICON_SIZE: f32 = 64.0;
/// Width of the language list, enough for its longest entry.
const LANG_WIDTH: f32 = 220.0;

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
    pub fn show(&mut self, ctx: &egui::Context, updates: &mut Updates) {
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
                let authors = AUTHORS.replace(':', ", ");
                ui.label(tr!(format!("Author: {authors}"), format!("Автор: {authors}")));
                ui.hyperlink_to(tr!("Homepage", "Сайт проекта"), REPOSITORY)
                    .on_hover_text(REPOSITORY);
                ui.weak(tr!(format!("{LICENSE} License"), format!("Лицензия {LICENSE}")));
                let stored = crate::settings::location();
                let place = if stored.portable {
                    tr!("Settings: next to the program (portable)", "Настройки: рядом с программой (портативно)")
                } else {
                    tr!("Settings: in the user profile", "Настройки: в профиле пользователя")
                };
                let place = ui.weak(place);
                match &stored.file {
                    Some(file) => place.on_hover_text(file.display().to_string()),
                    None => place.on_hover_text(tr!("Settings are not saved", "Настройки не сохраняются")),
                };
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                // Label over the list, both centred like the rest; a fixed
                // width keeps the list from jumping when the language changes.
                ui.weak(tr!("Interface language", "Язык интерфейса"));
                let mut choice = self.lang;
                // A combo box lays itself out left to right, ignoring the
                // centring: indent it by hand (`width` is its outer width).
                ui.horizontal(|ui| {
                    ui.add_space(((ui.available_width() - LANG_WIDTH) / 2.0).max(0.0));
                    egui::ComboBox::from_id_salt("language")
                        .selected_text(lang_label(choice))
                        .width(LANG_WIDTH)
                        .show_ui(ui, |ui| {
                            for c in [LangChoice::System, LangChoice::En, LangChoice::Ru] {
                                ui.selectable_value(&mut choice, c, lang_label(c));
                            }
                        });
                });
                if choice != self.lang {
                    self.lang = choice;
                    set_lang(choice.resolve());
                }
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                update_section(ui, updates);
                ui.add_space(8.0);
            });
            close_button(ui)
        });
        if modal.inner || modal.should_close() {
            self.open = false;
        }
    }
}

/// The update check: the start-up option, a button to check now and the
/// outcome.
fn update_section(ui: &mut Ui, updates: &mut Updates) {
    let before = updates.enabled;
    ui.checkbox(
        &mut updates.enabled,
        tr!("Check for updates at start-up (once a day)", "Проверять обновления при запуске (раз в сутки)"),
    )
    .on_hover_text(tr!(
        "Asks api.github.com for the latest release; nothing else is sent, \
         and nothing is downloaded",
        "Запрашивает у api.github.com последний выпуск; больше ничего не \
         отправляется и не скачивается"
    ));
    if updates.enabled && !before {
        updates.start_if_due(ui.ctx());
    }
    let checking = updates.status == Status::Checking;
    if ui
        .add_enabled(!checking, egui::Button::new(tr!("Check now", "Проверить сейчас")))
        .clicked()
    {
        updates.start(ui.ctx());
    }
    match &updates.status {
        Status::Unknown => {}
        Status::Checking => {
            ui.horizontal(|ui| {
                // Centred by hand, as the language list above.
                ui.add_space(((ui.available_width() - 110.0) / 2.0).max(0.0));
                ui.add(egui::Spinner::new());
                ui.weak(tr!("Checking…", "Проверка…"));
            });
        }
        Status::UpToDate => {
            ui.weak(tr!("This is the latest version", "Установлена последняя версия"));
        }
        Status::Newer(tag) => {
            let version = version_of(tag);
            ui.hyperlink_to(
                tr!(format!("Version {version} is available"), format!("Доступна версия {version}")),
                release_url(tag),
            );
        }
        Status::Failed(e) => {
            ui.weak(tr!("Could not check for updates", "Не удалось проверить обновления"))
                .on_hover_text(e);
        }
    }
}

fn close_button(ui: &mut Ui) -> bool {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.button(tr!("Close", "Закрыть")).clicked()
    })
    .inner
}
