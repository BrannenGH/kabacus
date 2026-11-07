// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The app's translations, as KDE apps keep them: one gettext catalog,
//! `kabacus`, with a template (`po/kabacus.pot`) and one `po/<lang>.po`
//! per language, compiled to `<lang>/LC_MESSAGES/kabacus.mo` for KI18n.
//!
//! - `i18n-extract` collects every string the app passes to KI18n (QML's
//!   `i18n()`, `i18nc()`, `i18np()`, `i18ncp()` and the Rust macros of the
//!   same names in `crates/kabacus-kde/src`) into the template, and merges
//!   it into every language's catalog, creating the catalogs of
//!   [`LANGUAGES`] that don't exist yet. A translation whose English text
//!   went away is kept at the end as obsolete (`#~`), never lost.
//!   `--check` changes nothing and fails if anything is out of date, if a
//!   message is untranslated (or fuzzy) in any catalog, or if a
//!   translation doesn't fit its English text (placeholders, plural
//!   forms).
//! - `i18n-build <dir>` compiles the catalogs into `<dir>`. Untranslated
//!   and fuzzy entries are left out, so KI18n shows the English text.
//!
//! This is done here rather than with xgettext so it runs wherever cargo
//! does (CI, every distribution's RPM build, the flatpak SDK) and knows
//! Rust's macros; the files are ordinary gettext ones for translators'
//! tools (Lokalize, poedit, msgfmt --check).

use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// The catalog's domain: KI18n's application domain (src/translation.h).
const DOMAIN: &str = "kabacus";

/// Where the strings are.
const QML_DIR: &str = "crates/kabacus-kde/qml";
const RUST_DIR: &str = "crates/kabacus-kde/src";
/// The macros' own definitions and tests.
const RUST_SKIP: &[&str] = &["i18n.rs"];

/// The languages there's a catalog for: KDE's translation teams', with
/// each team's plural rule (as in KDE's own catalogs), plus Esperanto and
/// Vietnamese, which KDE has teams for too.
pub const LANGUAGES: &[(&str, &str, &str)] = &[
    ("ar", "Arabic", "nplurals=6; plural=n==0 ? 0 : n==1 ? 1 : n==2 ? 2 : n%100>=3 && n%100<=10 ? 3 : n%100>=11 && n%100<=99 ? 4 : 5;"),
    ("ast", "Asturian", "nplurals=2; plural=n != 1;"),
    ("az", "Azerbaijani", "nplurals=2; plural=n != 1;"),
    ("be", "Belarusian", "nplurals=3; plural=n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2;"),
    ("bg", "Bulgarian", "nplurals=2; plural=n != 1;"),
    ("bs", "Bosnian", "nplurals=3; plural=n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2;"),
    ("ca", "Catalan", "nplurals=2; plural=n != 1;"),
    ("ca@valencia", "Valencian", "nplurals=2; plural=n != 1;"),
    ("cs", "Czech", "nplurals=3; plural=n==1 ? 0 : n>=2 && n<=4 ? 1 : 2;"),
    ("da", "Danish", "nplurals=2; plural=n != 1;"),
    ("de", "German", "nplurals=2; plural=n != 1;"),
    ("el", "Greek", "nplurals=2; plural=n != 1;"),
    ("en_GB", "British English", "nplurals=2; plural=n != 1;"),
    ("eo", "Esperanto", "nplurals=2; plural=n != 1;"),
    ("es", "Spanish", "nplurals=2; plural=n != 1;"),
    ("et", "Estonian", "nplurals=2; plural=n != 1;"),
    ("eu", "Basque", "nplurals=2; plural=n != 1;"),
    ("fa", "Persian", "nplurals=2; plural=n > 1;"),
    ("fi", "Finnish", "nplurals=2; plural=n != 1;"),
    ("fr", "French", "nplurals=2; plural=n > 1;"),
    ("ga", "Irish", "nplurals=5; plural=n==1 ? 0 : n==2 ? 1 : n<7 ? 2 : n<11 ? 3 : 4;"),
    ("gl", "Galician", "nplurals=2; plural=n != 1;"),
    ("he", "Hebrew", "nplurals=4; plural=n == 1 ? 0 : n == 2 ? 1 : n > 10 && n % 10 == 0 ? 2 : 3;"),
    ("hi", "Hindi", "nplurals=2; plural=n != 1;"),
    ("hu", "Hungarian", "nplurals=2; plural=n != 1;"),
    ("ia", "Interlingua", "nplurals=2; plural=n != 1;"),
    ("id", "Indonesian", "nplurals=2; plural=n != 1;"),
    ("is", "Icelandic", "nplurals=2; plural=n%10!=1 || n%100==11;"),
    ("it", "Italian", "nplurals=2; plural=n != 1;"),
    ("ja", "Japanese", "nplurals=1; plural=0;"),
    ("ka", "Georgian", "nplurals=2; plural=n != 1;"),
    ("kk", "Kazakh", "nplurals=1; plural=0;"),
    ("ko", "Korean", "nplurals=1; plural=0;"),
    ("lt", "Lithuanian", "nplurals=4; plural=n==1 ? 0 : n%10>=2 && (n%100<10 || n%100>=20) ? 1 : n%10==0 || (n%100>10 && n%100<20) ? 2 : 3;"),
    ("lv", "Latvian", "nplurals=3; plural=n%10==1 && n%100!=11 ? 0 : n != 0 ? 1 : 2;"),
    ("ml", "Malayalam", "nplurals=2; plural=n != 1;"),
    ("mr", "Marathi", "nplurals=2; plural=n != 1;"),
    ("nb", "Norwegian Bokmål", "nplurals=2; plural=n != 1;"),
    ("nds", "Low German", "nplurals=2; plural=n != 1;"),
    ("nl", "Dutch", "nplurals=2; plural=n != 1;"),
    ("nn", "Norwegian Nynorsk", "nplurals=2; plural=n != 1;"),
    ("oc", "Occitan", "nplurals=2; plural=n > 1;"),
    ("pa", "Punjabi", "nplurals=2; plural=n != 1;"),
    ("pl", "Polish", "nplurals=3; plural=n==1 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2;"),
    ("pt", "Portuguese", "nplurals=2; plural=n != 1;"),
    ("pt_BR", "Brazilian Portuguese", "nplurals=2; plural=n > 1;"),
    ("ro", "Romanian", "nplurals=3; plural=n==1 ? 0 : (n==0 || (n%100 > 0 && n%100 < 20)) ? 1 : 2;"),
    ("ru", "Russian", "nplurals=4; plural=n==1 ? 3 : n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2;"),
    ("sa", "Sanskrit", "nplurals=2; plural=n > 2;"),
    ("sk", "Slovak", "nplurals=3; plural=n==1 ? 0 : n>=2 && n<=4 ? 1 : 2;"),
    ("sl", "Slovenian", "nplurals=4; plural=n%100==1 ? 1 : n%100==2 ? 2 : n%100==3 || n%100==4 ? 3 : 0;"),
    ("sq", "Albanian", "nplurals=2; plural=n != 1;"),
    ("sr", "Serbian", "nplurals=4; plural=n==1 ? 3 : n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2;"),
    ("sr@ijekavian", "Serbian (Ijekavian)", "nplurals=4; plural=n==1 ? 3 : n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2;"),
    ("sr@ijekavianlatin", "Serbian (Ijekavian, Latin)", "nplurals=4; plural=n==1 ? 3 : n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2;"),
    ("sr@latin", "Serbian (Latin)", "nplurals=4; plural=n==1 ? 3 : n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2;"),
    ("sv", "Swedish", "nplurals=2; plural=n != 1;"),
    ("ta", "Tamil", "nplurals=2; plural=n != 1;"),
    ("tg", "Tajik", "nplurals=2; plural=n != 1;"),
    ("th", "Thai", "nplurals=1; plural=0;"),
    ("tr", "Turkish", "nplurals=2; plural=n > 1;"),
    ("ug", "Uyghur", "nplurals=1; plural=0;"),
    ("uk", "Ukrainian", "nplurals=4; plural=n==1 ? 3 : n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2;"),
    ("vi", "Vietnamese", "nplurals=1; plural=0;"),
    ("zh_CN", "Chinese (Simplified)", "nplurals=1; plural=0;"),
    ("zh_TW", "Chinese (Traditional)", "nplurals=1; plural=0;"),
];

// --- Messages -------------------------------------------------------------------------

/// A message: its context, English text and, for a plural, the English
/// plural.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    pub context: String,
    pub id: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    pub context: String,
    pub id: String,
    pub plural: Option<String>,
    /// One per plural form (one for a message without a plural).
    pub strs: Vec<String>,
    /// Translator comments (`# `), kept as written.
    pub comments: Vec<String>,
    /// The files it's used in (`#: file`).
    pub references: Vec<String>,
    pub flags: BTreeSet<String>,
    /// `#~`: its English text is no longer in the app.
    pub obsolete: bool,
}

impl Entry {
    fn key(&self) -> Key {
        Key {
            context: self.context.clone(),
            id: self.id.clone(),
        }
    }

    fn fuzzy(&self) -> bool {
        self.flags.contains("fuzzy")
    }

    fn translated(&self) -> bool {
        !self.strs.is_empty() && self.strs.iter().all(|s| !s.is_empty())
    }
}

// --- Extraction ------------------------------------------------------------------------

/// The KI18n calls and how many leading string arguments each takes:
/// context?, singular, plural?.
fn arity(name: &str) -> Option<(bool, bool)> {
    match name {
        "i18n" => Some((false, false)),
        "i18nc" => Some((true, false)),
        "i18np" => Some((false, true)),
        "i18ncp" => Some((true, true)),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Lang {
    Qml,
    Rust,
}

/// Every message in `text`, with the line it's on. A call whose text isn't
/// a string literal is an error: it couldn't be extracted, so it would
/// never be translated.
fn extract(text: &str, lang: Lang, file: &str) -> Result<Vec<(Entry, usize)>> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    let line_of = |at: usize| chars[..at].iter().filter(|c| **c == '\n').count() + 1;
    while i < chars.len() {
        let c = chars[i];
        // Comments.
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        // Strings and characters outside calls.
        if c == '"' || (lang == Lang::Qml && (c == '\'' || c == '`')) {
            i = string_literal(&chars, i, lang)
                .with_context(|| format!("{file}:{}", line_of(i)))?
                .1;
            continue;
        }
        if lang == Lang::Rust && c == '\'' {
            i = skip_char_or_lifetime(&chars, i);
            continue;
        }
        if lang == Lang::Rust && c == 'r' && !ident_before(&chars, i) {
            if let Some(end) = raw_string(&chars, i).map(|(_, end)| end) {
                i = end;
                continue;
            }
        }
        if is_ident_start(c) && !ident_before(&chars, i) {
            let start = i;
            while i < chars.len() && is_ident(chars[i]) {
                i += 1;
            }
            let name: String = chars[start..i].iter().collect();
            let Some((has_context, has_plural)) = arity(&name) else {
                continue;
            };
            // QML: not a method (`foo.i18n(`); Rust: the macro (`i18n!(`).
            if lang == Lang::Qml && start > 0 && chars[start - 1] == '.' {
                continue;
            }
            let mut j = i;
            if lang == Lang::Rust {
                if chars.get(j) != Some(&'!') {
                    continue;
                }
                j += 1;
            }
            j = skip_space(&chars, j);
            if chars.get(j) != Some(&'(') {
                continue;
            }
            j += 1;
            let line = line_of(start);
            let wanted = 1 + usize::from(has_context) + usize::from(has_plural);
            let mut args = vec![];
            for n in 0..wanted {
                j = skip_space(&chars, j);
                let literal = match chars.get(j) {
                    Some('"') => string_literal(&chars, j, lang),
                    Some('\'' | '`') if lang == Lang::Qml => string_literal(&chars, j, lang),
                    Some('r') if lang == Lang::Rust => {
                        raw_string(&chars, j).ok_or_else(|| anyhow::anyhow!("not a string literal"))
                    }
                    _ => Err(anyhow::anyhow!("not a string literal")),
                }
                .with_context(|| {
                    format!("{file}:{line}: {name}'s text must be a string literal")
                })?;
                args.push(literal.0);
                j = skip_space(&chars, literal.1);
                if n + 1 < wanted {
                    if chars.get(j) != Some(&',') {
                        bail!("{file}:{line}: {name} needs {wanted} string literals");
                    }
                    j += 1;
                }
            }
            let mut args = args.into_iter();
            let context = if has_context {
                args.next().unwrap_or_default()
            } else {
                String::new()
            };
            let id = args.next().unwrap_or_default();
            let plural = if has_plural { args.next() } else { None };
            if id.is_empty() {
                bail!("{file}:{line}: an empty message");
            }
            out.push((
                Entry {
                    context,
                    id,
                    plural,
                    ..Default::default()
                },
                line,
            ));
            i = j;
            continue;
        }
        i += 1;
    }
    Ok(out)
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn ident_before(chars: &[char], i: usize) -> bool {
    i > 0 && is_ident(chars[i - 1])
}

fn skip_space(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    i
}

/// Rust: `'x'`, `'\n'` or a lifetime (`'a`), from the quote; returns
/// where it ends.
fn skip_char_or_lifetime(chars: &[char], i: usize) -> usize {
    if chars.get(i + 1) == Some(&'\\') {
        let mut j = i + 2;
        while j < chars.len() && chars[j] != '\'' {
            j += 1;
        }
        return j + 1;
    }
    if chars.get(i + 2) == Some(&'\'') {
        return i + 3;
    }
    i + 1
}

/// `r"…"` or `r#"…"#`, from the `r`: its text and where it ends.
fn raw_string(chars: &[char], i: usize) -> Option<(String, usize)> {
    let mut j = i + 1;
    let mut hashes = 0;
    while chars.get(j) == Some(&'#') {
        hashes += 1;
        j += 1;
    }
    if chars.get(j) != Some(&'"') {
        return None;
    }
    j += 1;
    let start = j;
    loop {
        if j >= chars.len() {
            return None;
        }
        if chars[j] == '"' && (0..hashes).all(|h| chars.get(j + 1 + h) == Some(&'#')) {
            return Some((chars[start..j].iter().collect(), j + 1 + hashes));
        }
        j += 1;
    }
}

/// A quoted string from its opening quote: its text, escapes resolved,
/// and where it ends.
fn string_literal(chars: &[char], i: usize, lang: Lang) -> Result<(String, usize)> {
    let quote = chars[i];
    let mut out = String::new();
    let mut j = i + 1;
    while j < chars.len() {
        let c = chars[j];
        if c == quote {
            return Ok((out, j + 1));
        }
        if c != '\\' {
            out.push(c);
            j += 1;
            continue;
        }
        let Some(&e) = chars.get(j + 1) else {
            break;
        };
        j += 2;
        match e {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '0' => out.push('\0'),
            '\\' | '"' | '\'' | '`' => out.push(e),
            // A line continuation: the newline and the next line's
            // indentation aren't part of the text (Rust), or just the
            // newline (JavaScript).
            '\n' => {
                if lang == Lang::Rust {
                    j = skip_space(chars, j);
                }
            }
            'u' => {
                let (hex, next) = if chars.get(j) == Some(&'{') {
                    let end = (j..chars.len())
                        .find(|&k| chars[k] == '}')
                        .context("an unterminated \\u{…}")?;
                    (chars[j + 1..end].iter().collect::<String>(), end + 1)
                } else {
                    (chars[j..(j + 4).min(chars.len())].iter().collect(), j + 4)
                };
                let code = u32::from_str_radix(&hex, 16).context("a bad \\u escape")?;
                out.push(char::from_u32(code).context("a bad \\u escape")?);
                j = next;
            }
            other => bail!("an escape this extractor doesn't know: \\{other}"),
        }
    }
    bail!("an unterminated string")
}

/// Every message in the app, in the order the files and lines come,
/// each once with all the places it's used.
fn collect(root: &Path) -> Result<Vec<Entry>> {
    let mut files = vec![];
    for (dir, ext, lang) in [(QML_DIR, "qml", Lang::Qml), (RUST_DIR, "rs", Lang::Rust)] {
        let mut found: Vec<PathBuf> = std::fs::read_dir(root.join(dir))
            .with_context(|| format!("reading {dir}"))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == ext))
            .filter(|p| {
                !RUST_SKIP
                    .iter()
                    .any(|s| lang == Lang::Rust && p.file_name().is_some_and(|n| n == *s))
            })
            .collect();
        found.sort();
        files.extend(found.into_iter().map(|p| (p, lang)));
    }
    let mut entries: Vec<Entry> = vec![];
    let mut index: HashMap<Key, usize> = HashMap::new();
    for (path, lang) in files {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let text = std::fs::read_to_string(&path).with_context(|| format!("reading {rel}"))?;
        for (entry, line) in extract(&text, lang, &rel)? {
            // Files, not lines: with lines, every edit to a page would
            // change every language's catalog.
            let reference = rel.clone();
            match index.get(&entry.key()) {
                Some(&at) => {
                    let known = &mut entries[at];
                    if known.plural != entry.plural {
                        bail!(
                            "{rel}:{line}: {:?} is used both with and without a plural (or with two)",
                            entry.id
                        );
                    }
                    if !known.references.contains(&reference) {
                        known.references.push(reference);
                    }
                }
                None => {
                    index.insert(entry.key(), entries.len());
                    let mut entry = entry;
                    entry.references.push(reference);
                    entry.flags.insert("kde-format".to_string());
                    entries.push(entry);
                }
            }
        }
    }
    let web_path = "apps/kabacus-ui/src/messages.js";
    let web = std::fs::read_to_string(root.join(web_path))?;
    let start = web.find('[').context("missing web message array")?;
    let end = web.rfind(']').context("missing web message array end")?;
    // The vocabulary is a JSON-compatible array; trailing commas are permitted in JS.
    let array = web[start..=end].replace(",\n]", "\n]");
    let messages: Vec<String> = serde_json::from_str(&array)?;
    for id in messages {
        let mut entry = Entry {
            id,
            ..Entry::default()
        };
        if let Some(&at) = index.get(&entry.key()) {
            entries[at].references.push(web_path.into());
        } else {
            entry.references.push(web_path.into());
            entry.flags.insert("kde-format".into());
            index.insert(entry.key(), entries.len());
            entries.push(entry);
        }
    }
    Ok(entries)
}

// --- PO files --------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct Catalog {
    /// The header entry's comments and fields (msgid "").
    pub header_comments: Vec<String>,
    pub header: String,
    pub entries: Vec<Entry>,
}

impl Catalog {
    fn header_field(&self, name: &str) -> Option<&str> {
        self.header.lines().find_map(|l| {
            l.strip_prefix(name)
                .and_then(|r| r.strip_prefix(':'))
                .map(str::trim)
        })
    }

    /// The number of plural forms the header's rule has.
    fn nplurals(&self) -> Result<usize> {
        let rule = self
            .header_field("Plural-Forms")
            .context("the header has no Plural-Forms")?;
        let n = rule
            .split(';')
            .find_map(|p| p.trim().strip_prefix("nplurals="))
            .context("Plural-Forms has no nplurals")?;
        n.trim().parse().context("nplurals isn't a number")
    }
}

fn unquote(s: &str) -> Result<String> {
    let s = s.trim();
    let inner = s
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .with_context(|| format!("not a quoted string: {s}"))?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            other => bail!("an unknown escape \\{}", other.unwrap_or(' ')),
        }
    }
    Ok(out)
}

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A `.po` or `.pot` file.
pub fn parse_po(text: &str) -> Result<Catalog> {
    #[derive(Default)]
    struct Raw {
        comments: Vec<String>,
        references: Vec<String>,
        flags: BTreeSet<String>,
        obsolete: bool,
        context: Option<String>,
        id: Option<String>,
        plural: Option<String>,
        strs: BTreeMap<usize, String>,
    }
    let mut catalog = Catalog::default();
    let mut raws: Vec<Raw> = vec![];
    let mut cur = Raw::default();
    // Which string a continuation line adds to.
    #[derive(Clone, Copy)]
    enum Field {
        Context,
        Id,
        Plural,
        Str(usize),
    }
    let mut field: Option<Field> = None;
    let mut started = false;
    let flush = |cur: &mut Raw, raws: &mut Vec<Raw>, started: &mut bool| {
        if *started {
            raws.push(std::mem::take(cur));
        }
        *started = false;
    };
    for (n, line) in text.lines().enumerate() {
        let lineno = n + 1;
        let line = line.trim_end();
        if line.is_empty() {
            flush(&mut cur, &mut raws, &mut started);
            field = None;
            continue;
        }
        let (obsolete, body) = match line.strip_prefix("#~") {
            Some(rest) => (true, rest.trim_start()),
            None => (false, line),
        };
        if !obsolete && body.starts_with('#') {
            // A comment starts a new entry once the last one has text.
            if cur.id.is_some() {
                flush(&mut cur, &mut raws, &mut started);
                field = None;
            }
            started = true;
            if let Some(r) = body.strip_prefix("#:") {
                cur.references
                    .extend(r.split_whitespace().map(str::to_string));
            } else if let Some(f) = body.strip_prefix("#,") {
                cur.flags.extend(
                    f.split(',')
                        .map(|f| f.trim().to_string())
                        .filter(|f| !f.is_empty()),
                );
            } else if body.starts_with("#.") || body.starts_with("#|") {
                // Extracted comments and previous strings are regenerated.
            } else {
                cur.comments.push(body.to_string());
            }
            continue;
        }
        if body.starts_with('"') {
            let s = unquote(body).with_context(|| format!("line {lineno}"))?;
            match field {
                Some(Field::Context) => cur.context.get_or_insert_with(String::new).push_str(&s),
                Some(Field::Id) => cur.id.get_or_insert_with(String::new).push_str(&s),
                Some(Field::Plural) => cur.plural.get_or_insert_with(String::new).push_str(&s),
                Some(Field::Str(i)) => cur.strs.entry(i).or_default().push_str(&s),
                None => bail!("line {lineno}: a string with no keyword before it"),
            }
            continue;
        }
        let (keyword, rest) = body
            .split_once(char::is_whitespace)
            .with_context(|| format!("line {lineno}: can't read {body:?}"))?;
        // A keyword after a complete entry starts the next one.
        if matches!(keyword, "msgctxt" | "msgid") && cur.id.is_some() {
            flush(&mut cur, &mut raws, &mut started);
        }
        started = true;
        cur.obsolete |= obsolete;
        let s = unquote(rest).with_context(|| format!("line {lineno}"))?;
        field = Some(match keyword {
            "msgctxt" => {
                cur.context = Some(s);
                Field::Context
            }
            "msgid" => {
                cur.id = Some(s);
                Field::Id
            }
            "msgid_plural" => {
                cur.plural = Some(s);
                Field::Plural
            }
            "msgstr" => {
                cur.strs.insert(0, s);
                Field::Str(0)
            }
            k => {
                let i = k
                    .strip_prefix("msgstr[")
                    .and_then(|r| r.strip_suffix(']'))
                    .and_then(|i| i.parse().ok())
                    .with_context(|| format!("line {lineno}: unknown keyword {k}"))?;
                cur.strs.insert(i, s);
                Field::Str(i)
            }
        });
    }
    flush(&mut cur, &mut raws, &mut started);
    for raw in raws {
        let Some(id) = raw.id else {
            // Comments with no message: the file's opening comments.
            catalog.header_comments.extend(raw.comments);
            continue;
        };
        if id.is_empty() && raw.context.is_none() {
            catalog.header_comments.extend(raw.comments);
            catalog.header = raw.strs.get(&0).cloned().unwrap_or_default();
            continue;
        }
        let count = raw.strs.keys().max().map(|m| m + 1).unwrap_or(0);
        let strs = (0..count)
            .map(|i| raw.strs.get(&i).cloned().unwrap_or_default())
            .collect();
        catalog.entries.push(Entry {
            context: raw.context.unwrap_or_default(),
            id,
            plural: raw.plural,
            strs,
            comments: raw.comments,
            references: raw.references,
            flags: raw.flags,
            obsolete: raw.obsolete,
        });
    }
    Ok(catalog)
}

/// The catalog as a `.po` file, the same way every time.
pub fn write_po(catalog: &Catalog) -> String {
    let mut out = String::new();
    for c in &catalog.header_comments {
        out.push_str(c);
        out.push('\n');
    }
    out.push_str("msgid \"\"\nmsgstr \"\"\n");
    for line in catalog.header.split_inclusive('\n') {
        out.push_str(&quote(line));
        out.push('\n');
    }
    for e in &catalog.entries {
        out.push('\n');
        let p = if e.obsolete { "#~ " } else { "" };
        for c in &e.comments {
            out.push_str(c);
            out.push('\n');
        }
        if !e.obsolete && !e.references.is_empty() {
            out.push_str(&format!("#: {}\n", e.references.join(" ")));
        }
        if !e.flags.is_empty() {
            let flags: Vec<&str> = e.flags.iter().map(String::as_str).collect();
            out.push_str(&format!("#, {}\n", flags.join(", ")));
        }
        if !e.context.is_empty() {
            out.push_str(&format!("{p}msgctxt {}\n", quote(&e.context)));
        }
        out.push_str(&format!("{p}msgid {}\n", quote(&e.id)));
        match &e.plural {
            Some(plural) => {
                out.push_str(&format!("{p}msgid_plural {}\n", quote(plural)));
                for (i, s) in e.strs.iter().enumerate() {
                    out.push_str(&format!("{p}msgstr[{i}] {}\n", quote(s)));
                }
            }
            None => {
                let s = e.strs.first().map(String::as_str).unwrap_or("");
                out.push_str(&format!("{p}msgstr {}\n", quote(s)));
            }
        }
    }
    out
}

const POT_HEADER: &str = "Project-Id-Version: kabacus\n\
Report-Msgid-Bugs-To: https://github.com/BrannenGH/kabacus/issues\n\
MIME-Version: 1.0\n\
Content-Type: text/plain; charset=UTF-8\n\
Content-Transfer-Encoding: 8bit\n\
Plural-Forms: nplurals=INTEGER; plural=EXPRESSION;\n";

/// The template: every message, untranslated.
fn template(entries: &[Entry]) -> Catalog {
    Catalog {
        header_comments: vec![
            "# The messages of Kabacus, the KDE app, for translating.".to_string(),
            "# Generated by `cargo run -p xtask -- i18n-extract`; don't edit by hand.".to_string(),
        ],
        header: POT_HEADER.to_string(),
        entries: entries
            .iter()
            .map(|e| Entry {
                strs: vec![String::new(); if e.plural.is_some() { 2 } else { 1 }],
                ..e.clone()
            })
            .collect(),
    }
}

/// A new language's catalog.
fn new_catalog(lang: &str, name: &str, plural_forms: &str) -> Catalog {
    Catalog {
        header_comments: vec![
            format!("# {name} translation of Kabacus."),
            "# This file is distributed under the same license as the kabacus package.".to_string(),
        ],
        header: format!(
            "Project-Id-Version: kabacus\n\
             Report-Msgid-Bugs-To: https://github.com/BrannenGH/kabacus/issues\n\
             Language-Team: {name}\n\
             Language: {lang}\n\
             MIME-Version: 1.0\n\
             Content-Type: text/plain; charset=UTF-8\n\
             Content-Transfer-Encoding: 8bit\n\
             Plural-Forms: {plural_forms}\n"
        ),
        entries: vec![],
    }
}

/// `po` brought up to date with the template's messages: their
/// translations kept, new ones added untranslated, gone ones obsolete.
fn merge(po: &Catalog, messages: &[Entry]) -> Result<Catalog> {
    let nplurals = po.nplurals()?;
    let mut old: HashMap<Key, &Entry> = po.entries.iter().map(|e| (e.key(), e)).collect();
    let mut entries = vec![];
    for m in messages {
        let forms = if m.plural.is_some() { nplurals } else { 1 };
        let mut entry = Entry {
            strs: vec![String::new(); forms],
            ..m.clone()
        };
        if let Some(prev) = old.remove(&m.key()) {
            // Kept only if it's still the same kind of message.
            if prev.plural.is_some() == m.plural.is_some() {
                entry.strs = prev.strs.clone();
                entry.strs.resize(forms, String::new());
                entry.comments = prev.comments.clone();
                if prev.fuzzy() {
                    entry.flags.insert("fuzzy".to_string());
                }
                // A changed English plural needs a look.
                if prev.plural != m.plural && prev.translated() {
                    entry.flags.insert("fuzzy".to_string());
                }
            }
        }
        entries.push(entry);
    }
    // What's left was translated once: kept, obsolete, at the end.
    let mut gone: Vec<Entry> = po
        .entries
        .iter()
        .filter(|e| old.contains_key(&e.key()) && e.translated())
        .map(|e| Entry {
            obsolete: true,
            references: vec![],
            ..e.clone()
        })
        .collect();
    entries.append(&mut gone);
    Ok(Catalog {
        header_comments: po.header_comments.clone(),
        header: po.header.clone(),
        entries,
    })
}

/// `%1`, `%2`, … in `text`.
fn placeholders(text: &str) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    let mut rest = text;
    while let Some(at) = rest.find('%') {
        let after = &rest[at + 1..];
        let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(n) = digits.parse::<u32>() {
            if n >= 1 {
                out.insert(n);
            }
        }
        rest = after;
    }
    out
}

/// What's wrong with a catalog's translations, one line each.
/// The message the app reads its layout direction from
/// (crates/kabacus-kde/src/translation.rs): context and text.
const DIRECTION: (&str, &str) = (
    "Which way this language is written: translate as RTL if right to left, LTR otherwise",
    "LTR",
);

/// The catalogs whose language is written right to left. Their DIRECTION
/// must say RTL, or the app wouldn't mirror for them; every other catalog
/// must leave it LTR, or it would mirror for a left-to-right language.
const RIGHT_TO_LEFT: &[&str] = &["ar", "fa", "he", "ug"];

fn problems(lang: &str, po: &Catalog) -> Vec<String> {
    let mut out = vec![];
    let nplurals = match po.nplurals() {
        Ok(n) => n,
        Err(e) => return vec![format!("{lang}: {e}")],
    };
    if let Some(&(_, _, rule)) = LANGUAGES.iter().find(|(l, _, _)| *l == lang) {
        if po.header_field("Plural-Forms") != Some(rule) {
            out.push(format!("{lang}: Plural-Forms should be {rule:?}"));
        }
    }
    let direction = po
        .entries
        .iter()
        .find(|e| !e.obsolete && e.context == DIRECTION.0 && e.id == DIRECTION.1)
        .and_then(|e| e.strs.first())
        .map(String::as_str)
        .unwrap_or("");
    if RIGHT_TO_LEFT.contains(&lang) {
        if direction != "RTL" {
            out.push(format!(
                "{lang}: is written right to left, so translate {:?} as \"RTL\"",
                DIRECTION.1
            ));
        }
    } else if !matches!(direction, "" | "LTR") {
        out.push(format!(
            "{lang}: is written left to right, so translate {:?} as \"LTR\"",
            DIRECTION.1
        ));
    }
    for e in po.entries.iter().filter(|e| !e.obsolete) {
        let what = format!("{lang}: {:?}", e.id);
        match &e.plural {
            None => {
                let s = e.strs.first().map(String::as_str).unwrap_or("");
                if !s.is_empty() && placeholders(s) != placeholders(&e.id) {
                    out.push(format!("{what}: the translation's placeholders differ"));
                }
            }
            Some(plural) => {
                if e.strs.len() != nplurals {
                    out.push(format!("{what}: needs {nplurals} plural forms"));
                }
                // A form may leave out the count (%1), as KI18n allows.
                let mut want = placeholders(plural);
                want.extend(placeholders(&e.id));
                want.insert(1);
                for s in e.strs.iter().filter(|s| !s.is_empty()) {
                    let mut got = placeholders(s);
                    got.insert(1);
                    if got != want {
                        out.push(format!("{what}: a plural form's placeholders differ"));
                    }
                }
            }
        }
        // The app is translated as it grows: a message added or reworded
        // is translated in every catalog in the same change, so no
        // language falls back to English for new features. A fuzzy entry
        // isn't used (KI18n shows the English), so it counts as missing.
        if e.fuzzy() {
            out.push(format!(
                "{what}: is marked fuzzy; check the translation and drop the flag"
            ));
        } else if !e.translated() {
            out.push(format!("{what}: untranslated"));
        }
    }
    out
}

fn po_dir(root: &Path) -> PathBuf {
    root.join("po")
}

/// Every `po/<lang>.po`, by language.
fn catalogs(root: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let mut out = BTreeMap::new();
    let dir = po_dir(root);
    if !dir.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(&dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "po") {
            let lang = path
                .file_stem()
                .and_then(|s| s.to_str())
                .context("a catalog's name isn't UTF-8")?
                .to_string();
            out.insert(lang, path);
        }
    }
    Ok(out)
}

/// `cargo xtask i18n-extract [--check]`.
pub fn extract_all(root: &Path, check: bool) -> Result<()> {
    let messages = collect(root)?;
    let mut wanted: BTreeMap<PathBuf, String> = BTreeMap::new();
    wanted.insert(
        po_dir(root).join(format!("{DOMAIN}.pot")),
        write_po(&template(&messages)),
    );
    let existing = catalogs(root)?;
    let mut problems_found = vec![];
    for (lang, name, rule) in LANGUAGES {
        if !existing.contains_key(*lang) {
            let po = merge(&new_catalog(lang, name, rule), &messages)?;
            wanted.insert(po_dir(root).join(format!("{lang}.po")), write_po(&po));
        }
    }
    for (lang, path) in &existing {
        let text = std::fs::read_to_string(path)?;
        let po = parse_po(&text).with_context(|| format!("reading {}", path.display()))?;
        let merged = merge(&po, &messages).with_context(|| format!("merging {lang}"))?;
        problems_found.extend(problems(lang, &merged));
        wanted.insert(path.clone(), write_po(&merged));
    }
    let mut stale = vec![];
    for (path, text) in &wanted {
        let current = std::fs::read_to_string(path).unwrap_or_default();
        if current != *text {
            stale.push(
                path.strip_prefix(root)
                    .unwrap_or(path)
                    .display()
                    .to_string(),
            );
            if !check {
                std::fs::create_dir_all(path.parent().context("no parent")?)?;
                std::fs::write(path, text)?;
            }
        }
    }
    if !problems_found.is_empty() {
        bail!(
            "translations that don't fit:\n{}",
            problems_found.join("\n")
        );
    }
    if check && !stale.is_empty() {
        bail!(
            "out of date (run `cargo run -p xtask -- i18n-extract`):\n{}",
            stale.join("\n")
        );
    }
    let total = messages.len();
    for lang in catalogs(root)?.keys() {
        if let Ok(text) = std::fs::read_to_string(po_dir(root).join(format!("{lang}.po"))) {
            if let Ok(po) = parse_po(&text) {
                let done = po
                    .entries
                    .iter()
                    .filter(|e| !e.obsolete && !e.fuzzy() && e.translated())
                    .count();
                if check {
                    continue;
                }
                println!("{lang}: {done}/{total}");
            }
        }
    }
    Ok(())
}

// --- MO files --------------------------------------------------------------------------

/// A compiled catalog, as GNU gettext (and so KI18n) reads it.
pub fn write_mo(po: &Catalog) -> Vec<u8> {
    let mut pairs: Vec<(Vec<u8>, Vec<u8>)> = vec![(vec![], po.header.as_bytes().to_vec())];
    for e in &po.entries {
        if e.obsolete || e.fuzzy() || !e.translated() {
            continue;
        }
        let mut key = vec![];
        if !e.context.is_empty() {
            key.extend_from_slice(e.context.as_bytes());
            key.push(4);
        }
        key.extend_from_slice(e.id.as_bytes());
        if let Some(plural) = &e.plural {
            key.push(0);
            key.extend_from_slice(plural.as_bytes());
        }
        let value = e.strs.join("\0").into_bytes();
        pairs.push((key, value));
    }
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    let n = pairs.len() as u32;
    let header_len = 28u32;
    let originals_at = header_len;
    let translations_at = originals_at + n * 8;
    let mut data_at = translations_at + n * 8;
    let mut originals = vec![];
    let mut translations = vec![];
    let mut data = vec![];
    for (key, _) in &pairs {
        originals.push((key.len() as u32, data_at));
        data.extend_from_slice(key);
        data.push(0);
        data_at += key.len() as u32 + 1;
    }
    for (_, value) in &pairs {
        translations.push((value.len() as u32, data_at));
        data.extend_from_slice(value);
        data.push(0);
        data_at += value.len() as u32 + 1;
    }
    let mut out = vec![];
    for word in [
        0x9504_12de_u32,
        0,
        n,
        originals_at,
        translations_at,
        0,
        data_at,
    ] {
        out.extend_from_slice(&word.to_le_bytes());
    }
    for (len, at) in originals.into_iter().chain(translations) {
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&at.to_le_bytes());
    }
    out.extend_from_slice(&data);
    out
}

/// `cargo xtask i18n-build <dir>`: `<dir>/<lang>/LC_MESSAGES/kabacus.mo`
/// for every catalog, as `share/locale` holds them.
pub fn build_all(root: &Path, out: &Path) -> Result<()> {
    for (lang, path) in catalogs(root)? {
        let text = std::fs::read_to_string(&path)?;
        let po = parse_po(&text).with_context(|| format!("reading {}", path.display()))?;
        let found = problems(&lang, &po);
        if !found.is_empty() {
            bail!("translations that don't fit:\n{}", found.join("\n"));
        }
        let dir = out.join(&lang).join("LC_MESSAGES");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(format!("{DOMAIN}.mo")), write_mo(&po))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(text: &str, lang: Lang) -> Vec<(String, String, Option<String>)> {
        extract(text, lang, "test")
            .unwrap()
            .into_iter()
            .map(|(e, _)| (e.context, e.id, e.plural))
            .collect()
    }

    #[test]
    fn qml_calls_are_found_with_their_context_and_plural() {
        let qml = r#"
            // i18n("not this: a comment")
            title: i18n("Overview")
            text: i18nc("what a card owes", 'Owes')
            text: i18np("%1 fund", "%1 funds", n)
            subtitle: i18ncp("fund", "%1 left", "%1 left", n, x)
            text: ledger.i18n("not this: a method")
            text: "i18n(\"not this: a string\")"
        "#;
        assert_eq!(
            ids(qml, Lang::Qml),
            [
                ("".into(), "Overview".into(), None),
                ("what a card owes".into(), "Owes".into(), None),
                ("".into(), "%1 fund".into(), Some("%1 funds".into())),
                ("fund".into(), "%1 left".into(), Some("%1 left".into())),
            ]
        );
    }

    #[test]
    fn rust_macros_are_found_past_lifetimes_chars_and_raw_strings() {
        let rust = r##"
            fn f<'a>(x: &'a str) -> char { '"' }
            /// i18n!("not this: a doc comment")
            let s = r#"i18n!("not this: a raw string")"#;
            let a = i18n!("Can't reach %1", x);
            let b = crate::i18nc!("%1 is a file", "Can't read \"%1\"", f);
            let c = i18np!(
                "%1 transaction",
                "%1 transactions \
                 in all",
                n
            );
            i18n_args!(x);
        "##;
        assert_eq!(
            ids(rust, Lang::Rust),
            [
                ("".into(), "Can't reach %1".into(), None),
                ("%1 is a file".into(), "Can't read \"%1\"".into(), None),
                (
                    "".into(),
                    "%1 transaction".into(),
                    Some("%1 transactions in all".into())
                ),
            ]
        );
    }

    #[test]
    fn a_text_that_isnt_a_literal_is_refused() {
        let err = extract("text: i18n(label)", Lang::Qml, "Form.qml").unwrap_err();
        assert!(format!("{err:#}").contains("Form.qml:1"), "{err:#}");
        assert!(extract("i18n!(message)", Lang::Rust, "x.rs").is_err());
    }

    fn eo(entries: &str) -> Catalog {
        let po = format!(
            "{}\n{entries}",
            write_po(&new_catalog(
                "eo",
                "Esperanto",
                "nplurals=2; plural=n != 1;"
            ))
        );
        parse_po(&po).unwrap()
    }

    #[test]
    fn each_catalog_says_which_way_its_language_is_written() {
        let direction = |text: &str| {
            format!(
                "msgctxt \"{}\"\nmsgid \"{}\"\nmsgstr \"{text}\"\n",
                DIRECTION.0, DIRECTION.1
            )
        };
        let wrong = |lang: &str, po: &Catalog| {
            problems(lang, po)
                .into_iter()
                .filter(|p| p.contains("written"))
                .count()
        };
        // Untranslated: fine left to right, not right to left.
        assert_eq!(wrong("eo", &eo(&direction(""))), 0);
        assert_eq!(wrong("he", &eo(&direction(""))), 1);
        assert_eq!(wrong("he", &eo(&direction("RTL"))), 0);
        assert_eq!(wrong("eo", &eo(&direction("RTL"))), 1);
        assert_eq!(wrong("eo", &eo(&direction("LTR"))), 0);
        // A translation of the word itself isn't a direction.
        assert_eq!(wrong("ar", &eo(&direction("من اليمين"))), 1);
    }

    #[test]
    fn a_catalog_reads_back_as_written() {
        let po = eo(concat!(
            "# a translator's note\n",
            "#: crates/kabacus-kde/qml/Main.qml:3\n",
            "#, kde-format\n",
            "msgctxt \"what a card owes\"\n",
            "msgid \"Owes\"\n",
            "msgstr \"Ŝuldas\"\n",
            "\n",
            "msgid \"%1 fund\"\n",
            "msgid_plural \"%1 funds\"\n",
            "msgstr[0] \"%1 fonduso\"\n",
            "msgstr[1] \"\"\n",
            "\"%1 fondusoj\"\n",
            "\n",
            "#~ msgid \"Gone\"\n",
            "#~ msgstr \"For\"\n",
        ));
        assert_eq!(po.header_field("Language"), Some("eo"));
        assert_eq!(po.nplurals().unwrap(), 2);
        assert_eq!(po.entries.len(), 3);
        assert_eq!(po.entries[0].context, "what a card owes");
        assert_eq!(po.entries[0].comments, ["# a translator's note"]);
        assert_eq!(po.entries[1].strs, ["%1 fonduso", "%1 fondusoj"]);
        assert!(po.entries[2].obsolete);
        let again = parse_po(&write_po(&po)).unwrap();
        assert_eq!(again.entries, po.entries);
        assert_eq!(again.header, po.header);
    }

    #[test]
    fn merging_keeps_translations_and_obsoletes_what_went_away() {
        let po = eo("msgid \"Funds\"\nmsgstr \"Fondusoj\"\n\nmsgid \"Old\"\nmsgstr \"Malnova\"\n");
        let messages = [
            Entry {
                id: "Cards".into(),
                ..Default::default()
            },
            Entry {
                id: "Funds".into(),
                ..Default::default()
            },
            Entry {
                id: "%1 fund".into(),
                plural: Some("%1 funds".into()),
                ..Default::default()
            },
        ];
        let merged = merge(&po, &messages).unwrap();
        let strs: Vec<(&str, Vec<String>, bool)> = merged
            .entries
            .iter()
            .map(|e| (e.id.as_str(), e.strs.clone(), e.obsolete))
            .collect();
        assert_eq!(
            strs,
            [
                ("Cards", vec!["".to_string()], false),
                ("Funds", vec!["Fondusoj".to_string()], false),
                ("%1 fund", vec!["".to_string(), "".to_string()], false),
                ("Old", vec!["Malnova".to_string()], true),
            ]
        );
    }

    #[test]
    fn a_translation_must_keep_its_placeholders() {
        let po = eo(concat!(
            "msgid \"Move %1 to %2\"\nmsgstr \"Movu %2 al %1\"\n\n",
            "msgid \"Pay %1\"\nmsgstr \"Pagu\"\n\n",
            "msgid \"%1 fund\"\nmsgid_plural \"%1 funds\"\nmsgstr[0] \"unu fonduso\"\nmsgstr[1] \"%1 fondusoj\"\n",
        ));
        let found = problems("eo", &po);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("Pay %1"), "{found:?}");
    }

    #[test]
    fn every_message_must_be_translated() {
        let po = eo(concat!(
            "msgid \"Cards\"\nmsgstr \"\"\n\n",
            "#, fuzzy\nmsgid \"Funds\"\nmsgstr \"Fondusoj\"\n\n",
            "msgid \"%1 fund\"\nmsgid_plural \"%1 funds\"\nmsgstr[0] \"\"\nmsgstr[1] \"\"\n\n",
            "msgid \"Loans\"\nmsgstr \"Pruntoj\"\n\n",
            "#~ msgid \"Gone\"\n#~ msgstr \"\"\n",
        ));
        let found = problems("eo", &po);
        assert_eq!(found.len(), 3, "{found:?}");
        for id in ["\"Cards\"", "\"Funds\"", "\"%1 fund\""] {
            assert!(found.iter().any(|p| p.contains(id)), "{id}: {found:?}");
        }
    }

    /// Reads a .mo the way gettext does: binary search isn't needed to
    /// check it, only the tables.
    fn lookup(mo: &[u8], key: &[u8]) -> Option<Vec<u8>> {
        let word = |at: usize| u32::from_le_bytes(mo[at..at + 4].try_into().unwrap()) as usize;
        assert_eq!(word(0), 0x9504_12de);
        let (n, originals, translations) = (word(8), word(12), word(16));
        (0..n).find_map(|i| {
            let (len, at) = (word(originals + i * 8), word(originals + i * 8 + 4));
            (&mo[at..at + len] == key).then(|| {
                let (len, at) = (word(translations + i * 8), word(translations + i * 8 + 4));
                mo[at..at + len].to_vec()
            })
        })
    }

    #[test]
    fn a_compiled_catalog_holds_translated_entries_only() {
        let po = eo(concat!(
            "msgctxt \"what a card owes\"\nmsgid \"Owes\"\nmsgstr \"Ŝuldas\"\n\n",
            "msgid \"%1 fund\"\nmsgid_plural \"%1 funds\"\nmsgstr[0] \"%1 fonduso\"\nmsgstr[1] \"%1 fondusoj\"\n\n",
            "msgid \"Cards\"\nmsgstr \"\"\n\n",
            "#, fuzzy\nmsgid \"Funds\"\nmsgstr \"Fondusoj\"\n",
        ));
        let mo = write_mo(&po);
        assert_eq!(
            lookup(&mo, "what a card owes\u{4}Owes".as_bytes()).unwrap(),
            "Ŝuldas".as_bytes()
        );
        assert_eq!(
            lookup(&mo, b"%1 fund\0%1 funds").unwrap(),
            b"%1 fonduso\0%1 fondusoj"
        );
        assert!(lookup(&mo, b"Cards").is_none());
        assert!(lookup(&mo, b"Funds").is_none());
        let header = String::from_utf8(lookup(&mo, b"").unwrap()).unwrap();
        assert!(header.contains("Plural-Forms: nplurals=2"), "{header}");
    }
}
