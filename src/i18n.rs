//! Interface language: English or Russian, taken from the language of the
//! Windows interface unless chosen in the About window.
//!
//! Strings stay next to the code that shows them, both languages together:
//! `tr!("Rescan", "Обновить")` gives the one of the current language (and
//! evaluates only that one, so `format!` arms cost nothing extra). A
//! missing translation does not compile.

use std::sync::atomic::{AtomicU8, Ordering::Relaxed};

use crate::format::thousands;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lang {
    #[default]
    En,
    Ru,
}

/// The language as kept in the settings: a fixed one, or the system's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LangChoice {
    #[default]
    System,
    En,
    Ru,
}

impl LangChoice {
    pub fn resolve(self) -> Lang {
        match self {
            LangChoice::System => system_lang(),
            LangChoice::En => Lang::En,
            LangChoice::Ru => Lang::Ru,
        }
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

/// The language the interface is shown in now.
pub fn lang() -> Lang {
    match LANG.load(Relaxed) {
        1 => Lang::Ru,
        _ => Lang::En,
    }
}

pub fn set_lang(lang: Lang) {
    LANG.store(lang as u8, Relaxed);
}

/// Russian when Windows shows its interface in Russian, else English.
pub fn system_lang() -> Lang {
    if crate::scan::win::ui_language_is_russian() {
        Lang::Ru
    } else {
        Lang::En
    }
}

/// The English or the Russian expression, by the current language.
#[macro_export]
macro_rules! tr {
    ($en:expr, $ru:expr $(,)?) => {
        match $crate::i18n::lang() {
            $crate::i18n::Lang::En => $en,
            $crate::i18n::Lang::Ru => $ru,
        }
    };
}

/// The Russian form of a word for `n` things: `one` for 1, 21, 101…,
/// `few` for 2–4, 22–24…, `many` for 0, 5–20, 25–30… (11–14 included).
pub fn ru_plural<'a>(n: u64, one: &'a str, few: &'a str, many: &'a str) -> &'a str {
    match (n % 10, n % 100) {
        (_, 11..=14) => many,
        (1, _) => one,
        (2..=4, _) => few,
        _ => many,
    }
}

/// `n` with the word for it in the current language: `1 file`, `3 files`,
/// `3 файла`, `5 файлов`; the number with thin-space thousands.
pub fn count(n: u64, en: [&str; 2], ru: [&str; 3]) -> String {
    count_in(lang(), n, en, ru)
}

pub fn count_in(lang: Lang, n: u64, en: [&str; 2], ru: [&str; 3]) -> String {
    let word = match lang {
        Lang::En => {
            if n == 1 {
                en[0]
            } else {
                en[1]
            }
        }
        Lang::Ru => ru_plural(n, ru[0], ru[1], ru[2]),
    };
    format!("{} {word}", thousands(n))
}

/// `count` for files.
pub fn files(n: u64) -> String {
    count(n, ["file", "files"], ["файл", "файла", "файлов"])
}

/// `count` for folders.
pub fn folders(n: u64) -> String {
    count(n, ["folder", "folders"], ["папка", "папки", "папок"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn russian_plurals() {
        let f = |n| ru_plural(n, "файл", "файла", "файлов");
        assert_eq!(f(1), "файл");
        assert_eq!(f(21), "файл");
        assert_eq!(f(101), "файл");
        assert_eq!(f(2), "файла");
        assert_eq!(f(24), "файла");
        assert_eq!(f(0), "файлов");
        assert_eq!(f(5), "файлов");
        assert_eq!(f(11), "файлов");
        assert_eq!(f(12), "файлов");
        assert_eq!(f(114), "файлов");
        assert_eq!(f(111), "файлов");
    }

    #[test]
    fn counts_in_both_languages() {
        let en = ["file", "files"];
        let ru = ["файл", "файла", "файлов"];
        assert_eq!(count_in(Lang::En, 1, en, ru), "1 file");
        assert_eq!(count_in(Lang::En, 0, en, ru), "0 files");
        assert_eq!(count_in(Lang::Ru, 3, en, ru), "3 файла");
        assert_eq!(count_in(Lang::Ru, 1000, en, ru), "1\u{2009}000 файлов");
    }
}
