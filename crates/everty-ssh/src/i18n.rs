use crate::config::AppLanguage;

/// Returns localized text slice based on active application language.
#[inline(always)]
pub fn tr<'a>(lang: AppLanguage, ru: &'a str, en: &'a str) -> &'a str {
    match lang {
        AppLanguage::Ru => ru,
        AppLanguage::En => en,
    }
}
