// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Translation, the KDE way: one gettext catalog, `kabacus`, looked up
//! through KI18n. QML calls KI18n's `i18n()`, `i18nc()`, `i18np()` and
//! `i18ncp()` directly; Rust uses the macros here, which the gui sends
//! through the same KI18n lookup ([`set_translator`], from the bridge).
//! Without the gui (the model and its tests run without Qt) they render
//! the English source text, so tests read exactly what the code says.
//!
//! Placeholders are KI18n's: `%1`, `%2`, … in argument order, and a plural's
//! count is always `%1`. Translators may reorder them; that's why it isn't
//! `format!`. `cargo run -p xtask -- i18n-extract` collects every literal
//! passed to these macros (and to the QML functions) into `po/kabacus.pot`,
//! so the text must be a string literal, never a variable.

use std::sync::OnceLock;

/// Looks up `singular` (or `plural`, by `count`) in `context` and fills in
/// its placeholders: `count` first, when there is one, then `args`.
pub type Translator =
    fn(context: &str, singular: &str, plural: &str, count: Option<i64>, args: &[String]) -> String;

static TRANSLATOR: OnceLock<Translator> = OnceLock::new();

/// Send every translation through `translator` from now on. Only the
/// first call counts: the gui sets it once at startup.
pub fn set_translator(translator: Translator) {
    let _ = TRANSLATOR.set(translator);
}

/// What the macros call.
pub fn translate(
    context: &str,
    singular: &str,
    plural: &str,
    count: Option<i64>,
    args: &[String],
) -> String {
    match TRANSLATOR.get() {
        Some(translator) => translator(context, singular, plural, count, args),
        None => english(singular, plural, count, args),
    }
}

/// The untranslated text with its placeholders filled in: what KI18n
/// shows when there's no translation, English plural rules included.
pub fn english(singular: &str, plural: &str, count: Option<i64>, args: &[String]) -> String {
    let text = match count {
        Some(n) if n != 1 && !plural.is_empty() => plural,
        _ => singular,
    };
    let all: Vec<String> = count
        .map(|n| n.to_string())
        .into_iter()
        .chain(args.iter().cloned())
        .collect();
    substitute(text, &all)
}

/// Replaces `%1`, `%2`, … with `args[0]`, `args[1]`, …; a `%` not followed
/// by the number of an argument stays as it is.
pub fn substitute(text: &str, args: &[String]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        match after[..digits].parse::<usize>() {
            Ok(n) if n >= 1 && n <= args.len() => {
                out.push_str(&args[n - 1]);
                rest = &after[digits..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The macros' arguments as text, for [`translate`].
#[doc(hidden)]
#[macro_export]
macro_rules! i18n_args {
    ($($arg:expr),*) => {{
        // One argument would be `&[x.to_string()]`, which clippy reads as
        // cloning a reference into a slice.
        #[allow(clippy::cloned_ref_to_slice_refs)]
        let args: [String; $crate::i18n_args!(@count $($arg)*)] = [$(($arg).to_string()),*];
        args
    }};
    (@count) => { 0 };
    (@count $head:tt $($tail:tt)*) => { 1 + $crate::i18n_args!(@count $($tail)*) };
}

/// `i18n!("Can't reach %1", address)`: translated text with its
/// placeholders filled in.
#[macro_export]
macro_rules! i18n {
    ($text:literal $(, $arg:expr)* $(,)?) => {
        $crate::i18n::translate("", $text, "", None, &$crate::i18n_args!($($arg),*))
    };
}

/// `i18nc!("context", "text", args…)`: [`i18n!`] with a context that tells
/// translators where the text appears, and tells apart texts that are the
/// same in English but not in every language.
#[macro_export]
macro_rules! i18nc {
    ($context:literal, $text:literal $(, $arg:expr)* $(,)?) => {
        $crate::i18n::translate($context, $text, "", None, &$crate::i18n_args!($($arg),*))
    };
}

/// `i18np!("%1 transaction", "%1 transactions", n, args…)`: the form for
/// `n` in the user's language (which may have more than two); `n` is `%1`.
#[macro_export]
macro_rules! i18np {
    ($singular:literal, $plural:literal, $count:expr $(, $arg:expr)* $(,)?) => {
        $crate::i18n::translate(
            "",
            $singular,
            $plural,
            Some(($count) as i64),
            &$crate::i18n_args!($($arg),*),
        )
    };
}

/// [`i18np!`] with a context.
#[macro_export]
macro_rules! i18ncp {
    ($context:literal, $singular:literal, $plural:literal, $count:expr $(, $arg:expr)* $(,)?) => {
        $crate::i18n::translate(
            $context,
            $singular,
            $plural,
            Some(($count) as i64),
            &$crate::i18n_args!($($arg),*),
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_are_filled_in_by_number_not_position() {
        let args = ["Checking".to_string(), "Gifts".to_string()];
        assert_eq!(
            substitute("Move from %1 to %2", &args),
            "Move from Checking to Gifts"
        );
        // A translation may put them in another order.
        assert_eq!(substitute("%2 ← %1", &args), "Gifts ← Checking");
        assert_eq!(substitute("%1, %1", &args[..1]), "Checking, Checking");
    }

    #[test]
    fn a_percent_that_is_not_a_placeholder_stays() {
        let args = ["7".to_string()];
        assert_eq!(substitute("100% sure, %1", &args), "100% sure, 7");
        assert_eq!(substitute("%3 of %1", &args), "%3 of 7");
        assert_eq!(substitute("ends with %", &args), "ends with %");
    }

    #[test]
    fn without_the_gui_the_english_text_is_shown() {
        assert_eq!(
            crate::i18n!("Can't reach the server: %1", "timed out"),
            "Can't reach the server: timed out"
        );
        assert_eq!(crate::i18nc!("button", "Save"), "Save");
        assert_eq!(
            crate::i18np!("%1 transaction", "%1 transactions", 1usize),
            "1 transaction"
        );
        assert_eq!(
            crate::i18np!("%1 transaction on %2", "%1 transactions on %2", 3, "Visa"),
            "3 transactions on Visa"
        );
        assert_eq!(crate::i18ncp!("fund", "%1 fund", "%1 funds", 0), "0 funds");
    }
}
