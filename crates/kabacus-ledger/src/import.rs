// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Reading a journal tree into a book: what is read as is, what is raised
//! or ignored and reported, and what stops the import with an error naming
//! file and line.

use crate::amount::{parse_amount, style_from_example, AmountError, Style, UsageStyle};
use crate::balance::{infer, Written};
use crate::book::{Account, Book, Commodity};
use crate::check::{
    assertion_without_amount, check_located, unbalanced_failure, CheckFailure, FailureKind, Locate,
    Position,
};
use chrono::NaiveDate;
use kabacus_core::{
    AccountName, Amount, BalanceAssertion, Cost, CostKind, Money, Posting, PostingKind, Status,
    Transaction,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

/// Where import reads journal text from. It exists only because a journal
/// can `include` other files.
pub trait ImportSource {
    fn read(&self, path: &Path) -> io::Result<String>;
    /// The files matching a glob pattern, in a stable order.
    fn glob(&self, pattern: &Path) -> io::Result<Vec<PathBuf>>;
}

/// Journals on the filesystem.
pub struct FsSource;

impl ImportSource for FsSource {
    fn read(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn glob(&self, pattern: &Path) -> io::Result<Vec<PathBuf>> {
        let pattern = pattern.to_string_lossy();
        let mut found: Vec<PathBuf> = glob::glob(&pattern)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?
            .filter_map(Result::ok)
            .filter(|p| p.is_file())
            .collect();
        found.sort();
        Ok(found)
    }
}

/// Journals held in memory, by path.
#[derive(Debug, Clone, Default)]
pub struct MemorySource(pub BTreeMap<PathBuf, String>);

impl ImportSource for MemorySource {
    fn read(&self, path: &Path) -> io::Result<String> {
        self.0
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, path.display().to_string()))
    }

    fn glob(&self, pattern: &Path) -> io::Result<Vec<PathBuf>> {
        let pattern = glob::Pattern::new(&pattern.to_string_lossy())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e.to_string()))?;
        Ok(self
            .0
            .keys()
            .filter(|p| pattern.matches_path(p))
            .cloned()
            .collect())
    }
}

/// What import ignored or changed, so nothing is dropped silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Noted {
    Tag,
    PriceLine,
    AutomatedPostings,
    PeriodicTransaction,
    Comment,
    CommentBlock,
    PayeeDirective,
    TagDirective,
    Subdirective,
    SecondaryDate,
    PlacesRaised,
    AssertionSkipped,
    PostingWithoutAmount,
}

impl Noted {
    pub fn as_str(self) -> &'static str {
        match self {
            Noted::Tag => "tag",
            Noted::PriceLine => "price-line",
            Noted::AutomatedPostings => "automated-postings",
            Noted::PeriodicTransaction => "periodic-transaction",
            Noted::Comment => "comment",
            Noted::CommentBlock => "comment-block",
            Noted::PayeeDirective => "payee-directive",
            Noted::TagDirective => "tag-directive",
            Noted::Subdirective => "subdirective",
            Noted::SecondaryDate => "secondary-date",
            Noted::PlacesRaised => "places-raised",
            Noted::AssertionSkipped => "assertion-skipped",
            Noted::PostingWithoutAmount => "posting-without-amount",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportEntry {
    pub noted: Noted,
    pub file: String,
    pub line: usize,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub entries: Vec<ReportEntry>,
}

impl ImportReport {
    fn note(&mut self, noted: Noted, at: &Loc, detail: impl Into<String>) {
        self.entries.push(ReportEntry {
            noted,
            file: at.file.clone(),
            line: at.line,
            detail: detail.into(),
        });
    }

    /// How many of each were noted.
    pub fn counts(&self) -> BTreeMap<Noted, usize> {
        let mut counts = BTreeMap::new();
        for e in &self.entries {
            *counts.entry(e.noted).or_insert(0) += 1;
        }
        counts
    }
}

/// Why an import stopped: a line it can't carry on past, or a book that
/// fails the strict check.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    #[error("{file}:{line}: {message}")]
    Stopped {
        file: String,
        line: usize,
        message: String,
    },
    #[error("{}", .0.iter().map(CheckFailure::message).collect::<Vec<_>>().join("\n"))]
    Invalid(Vec<CheckFailure>),
}

impl ImportError {
    fn at(at: &Loc, message: impl Into<String>) -> ImportError {
        ImportError::Stopped {
            file: at.file.clone(),
            line: at.line,
            message: message.into(),
        }
    }

    /// The failures, as the strict check reports them.
    pub fn failures(&self) -> Vec<CheckFailure> {
        match self {
            ImportError::Stopped {
                file,
                line,
                message,
            } => vec![CheckFailure {
                kind: FailureKind::Readable,
                position: Position::Line {
                    file: file.clone(),
                    line: *line,
                },
                sentence: message.clone(),
                details: String::new(),
            }],
            ImportError::Invalid(failures) => failures.clone(),
        }
    }
}

/// A book read from journals.
#[derive(Debug, Clone)]
pub struct ImportOutcome {
    pub book: Book,
    pub report: ImportReport,
    /// Each transaction's first line, by id.
    pub origins: HashMap<u64, (String, usize)>,
}

#[derive(Debug, Clone)]
struct Loc {
    file: String,
    line: usize,
}

#[derive(Debug, Clone)]
struct RawPosting {
    at: Loc,
    status: Status,
    kind: PostingKind,
    account: String,
    amount: Option<String>,
    cost: Option<(CostKind, String)>,
    assertion: Option<(String, String)>,
    comment: Vec<String>,
}

#[derive(Debug, Clone)]
struct RawTx {
    at: Loc,
    date: NaiveDate,
    date2: Option<NaiveDate>,
    status: Status,
    code: String,
    description: String,
    comment: Vec<String>,
    postings: Vec<RawPosting>,
}

#[derive(Debug, Default)]
struct Raw {
    transactions: Vec<RawTx>,
    /// Declared accounts in the order read, with their comments.
    accounts: Vec<(String, Vec<String>)>,
    /// Commodity directives in the order read: symbol and example style.
    commodities: Vec<(String, Option<Style>)>,
    report: ImportReport,
    files: Vec<PathBuf>,
    /// The files' names as positions give them, in read order.
    names: Vec<String>,
}

/// The block an indented line belongs to.
enum Block {
    None,
    Transaction,
    Account,
    Commodity,
    Ignored,
}

fn status_of(c: char) -> Option<Status> {
    match c {
        '*' => Some(Status::Cleared),
        '!' => Some(Status::Pending),
        _ => None,
    }
}

/// Split off a comment: the text after the first `;`.
fn split_comment(text: &str) -> (&str, Option<String>) {
    match text.find(';') {
        Some(i) => (&text[..i], Some(text[i + 1..].trim().to_string())),
        None => (text, None),
    }
}

fn parse_date(token: &str, at: &Loc) -> Result<NaiveDate, ImportError> {
    let parts: Vec<&str> = token.split(['-', '/', '.']).collect();
    if parts.len() == 2 {
        return Err(ImportError::at(at, "A date needs a year."));
    }
    let numbers: Option<Vec<u32>> = parts.iter().map(|p| p.parse().ok()).collect();
    match numbers.as_deref() {
        Some([y, m, d]) if parts[0].len() == 4 => NaiveDate::from_ymd_opt(*y as i32, *m, *d)
            .ok_or_else(|| ImportError::at(at, format!("{token} isn't a real date."))),
        _ => Err(ImportError::at(at, format!("{token} isn't a date."))),
    }
}

/// The account part of a posting ends at two spaces or a tab.
fn split_account(text: &str) -> (&str, &str) {
    let end = text
        .char_indices()
        .find(|&(i, c)| c == '\t' || (c == ' ' && text[i + 1..].starts_with(' ')))
        .map_or(text.len(), |(i, _)| i);
    (&text[..end], &text[end..])
}

/// Cut `text` at the first of `stops` outside double quotes.
fn cut_unquoted<'a>(text: &'a str, stops: &[char]) -> (&'a str, &'a str) {
    let mut quoted = false;
    for (i, c) in text.char_indices() {
        if c == '"' {
            quoted = !quoted;
        } else if !quoted && stops.contains(&c) {
            return (&text[..i], &text[i..]);
        }
    }
    (text, "")
}

fn parse_posting(line: &str, at: Loc) -> Result<RawPosting, ImportError> {
    let mut rest = line.trim_start();
    let mut status = Status::Unmarked;
    if let Some(s) = rest.chars().next().and_then(status_of) {
        status = s;
        rest = rest[1..].trim_start();
    }
    let (account, rest) = split_account(rest);
    let (kind, account) = if account.starts_with('(') && account.ends_with(')') {
        (PostingKind::Virtual, &account[1..account.len() - 1])
    } else if account.starts_with('[') && account.ends_with(']') {
        (PostingKind::BalancedVirtual, &account[1..account.len() - 1])
    } else {
        (PostingKind::Real, account)
    };
    if account.trim().is_empty() {
        return Err(ImportError::at(&at, "This posting has no account."));
    }
    let (amount, rest) = cut_unquoted(rest, &['@', '=', ';']);
    let mut posting = RawPosting {
        at,
        status,
        kind,
        account: account.trim().to_string(),
        amount: Some(amount.trim().to_string()).filter(|a| !a.is_empty()),
        cost: None,
        assertion: None,
        comment: Vec::new(),
    };
    let mut rest = rest;
    if let Some(after) = rest.strip_prefix('@') {
        let (kind, after) = match after.strip_prefix('@') {
            Some(after) => (CostKind::Total, after),
            None => (CostKind::Unit, after),
        };
        let (cost, after) = cut_unquoted(after, &['=', ';']);
        posting.cost = Some((kind, cost.trim().to_string()));
        rest = after;
    }
    if rest.starts_with('=') {
        let op_len = ["==*", "==", "=*", "="]
            .iter()
            .find(|op| rest.starts_with(*op))
            .map_or(1, |op| op.len());
        let op = rest[..op_len].to_string();
        let (asserted, after) = cut_unquoted(&rest[op_len..], &[';']);
        posting.assertion = Some((op, asserted.trim().to_string()));
        rest = after;
    }
    if let Some(comment) = rest.strip_prefix(';') {
        posting.comment.push(comment.trim().to_string());
    } else if !rest.trim().is_empty() {
        return Err(ImportError::at(
            &posting.at,
            format!("{} isn't a posting.", line.trim()),
        ));
    }
    Ok(posting)
}

fn parse_header(line: &str, at: Loc) -> Result<RawTx, ImportError> {
    let token_end = line
        .find(|c: char| c.is_whitespace() || c == ';')
        .unwrap_or(line.len());
    let (dates, rest) = line.split_at(token_end);
    let (date, date2) = match dates.split_once('=') {
        Some((d, d2)) => (parse_date(d, &at)?, Some(parse_date(d2, &at)?)),
        None => (parse_date(dates, &at)?, None),
    };
    let (rest, comment) = split_comment(rest);
    let mut rest = rest.trim_start();
    let mut status = Status::Unmarked;
    if let Some(s) = rest.chars().next().and_then(status_of) {
        status = s;
        rest = rest[1..].trim_start();
    }
    let mut code = String::new();
    if let Some(after) = rest.strip_prefix('(') {
        if let Some(end) = after.find(')') {
            code = after[..end].trim().to_string();
            rest = after[end + 1..].trim_start();
        }
    }
    Ok(RawTx {
        at,
        date,
        date2,
        status,
        code,
        description: rest.trim().to_string(),
        comment: comment.into_iter().collect(),
        postings: Vec::new(),
    })
}

/// The first word of a directive line.
fn keyword(line: &str) -> &str {
    line.split_whitespace().next().unwrap_or("")
}

fn has_glob(path: &str) -> bool {
    path.contains(['*', '?', '['])
}

struct Reader<'a> {
    src: &'a dyn ImportSource,
    raw: Raw,
    stack: Vec<PathBuf>,
}

impl Reader<'_> {
    fn read_file(
        &mut self,
        path: &Path,
        name: &str,
        from: Option<&Loc>,
    ) -> Result<(), ImportError> {
        if self.stack.iter().any(|p| p == path) {
            let at = from.cloned().unwrap_or(Loc {
                file: name.to_string(),
                line: 1,
            });
            return Err(ImportError::at(
                &at,
                format!("Including {name} here forms a cycle."),
            ));
        }
        let text = self.src.read(path).map_err(|_| match from {
            Some(at) => ImportError::at(at, format!("No file matches {name}.")),
            None => ImportError::Stopped {
                file: name.to_string(),
                line: 0,
                message: format!("{name} can't be read."),
            },
        })?;
        self.stack.push(path.to_path_buf());
        self.raw.files.push(path.to_path_buf());
        self.raw.names.push(name.to_string());
        let result = self.read_text(path, name, &text);
        self.stack.pop();
        result
    }

    fn include(&mut self, path: &Path, target: &str, at: &Loc) -> Result<(), ImportError> {
        let dir = path.parent().unwrap_or(Path::new(""));
        let wanted = dir.join(target);
        let files = if has_glob(target) {
            self.src.glob(&wanted).unwrap_or_default()
        } else {
            vec![wanted]
        };
        if files.is_empty() {
            return Err(ImportError::at(at, format!("No file matches {target}.")));
        }
        let base = Path::new(&at.file).parent().unwrap_or(Path::new(""));
        for file in files {
            let relative = file.strip_prefix(dir).unwrap_or(&file);
            let name = base.join(relative).to_string_lossy().to_string();
            self.read_file(&file, &name, Some(at))?;
        }
        Ok(())
    }

    fn read_text(&mut self, path: &Path, name: &str, text: &str) -> Result<(), ImportError> {
        let mut block = Block::None;
        let mut lines = text.lines().enumerate().peekable();
        while let Some((n, raw_line)) = lines.next() {
            let line = raw_line.trim_end_matches('\r');
            let at = Loc {
                file: name.to_string(),
                line: n + 1,
            };
            if line.trim().is_empty() {
                block = Block::None;
                continue;
            }
            let indented = line.starts_with([' ', '\t']);
            if indented {
                let body = line.trim_start();
                match block {
                    Block::Transaction => {
                        let tx = self.raw.transactions.last_mut().expect("in a transaction");
                        if let Some(comment) = body.strip_prefix(';') {
                            let comment = comment.trim().to_string();
                            match tx.postings.last_mut() {
                                Some(p) => {
                                    if p.comment.is_empty() {
                                        p.comment.push(String::new());
                                    }
                                    p.comment.push(comment);
                                }
                                None => {
                                    if tx.comment.is_empty() {
                                        tx.comment.push(String::new());
                                    }
                                    tx.comment.push(comment);
                                }
                            }
                        } else {
                            tx.postings.push(parse_posting(line, at)?);
                        }
                    }
                    Block::Account => {
                        let (_, comments) = self.raw.accounts.last_mut().expect("in an account");
                        match body.strip_prefix(';') {
                            Some(comment) => {
                                if comments.is_empty() {
                                    comments.push(String::new());
                                }
                                comments.push(comment.trim().to_string());
                            }
                            None => self.raw.report.note(Noted::Subdirective, &at, body),
                        }
                    }
                    Block::Commodity => {
                        if let Some(example) = body.strip_prefix("format") {
                            let (example, _) = split_comment(example);
                            let (_, style) = style_from_example(example)
                                .map_err(|e| amount_error(&at, e, true))?;
                            if let Some((_, current)) = self.raw.commodities.last_mut() {
                                *current = Some(style);
                            }
                        } else if !body.starts_with(';') {
                            self.raw.report.note(Noted::Subdirective, &at, body);
                        }
                    }
                    Block::Ignored => {}
                    Block::None => {
                        if body.starts_with([';', '#']) {
                            self.raw.report.note(Noted::Comment, &at, body);
                        } else {
                            return Err(ImportError::at(
                                &at,
                                "This line isn't a transaction or a supported directive.",
                            ));
                        }
                    }
                }
                continue;
            }
            block = Block::None;
            let first = line.chars().next().expect("not empty");
            if first.is_ascii_digit() {
                self.raw.transactions.push(parse_header(line, at)?);
                block = Block::Transaction;
                continue;
            }
            if matches!(first, ';' | '#' | '*') {
                self.raw.report.note(Noted::Comment, &at, line);
                continue;
            }
            let word = keyword(line);
            let rest = line[word.len()..].trim_start();
            match word {
                "include" => {
                    let (target, _) = split_comment(rest);
                    self.include(path, target.trim(), &at)?;
                }
                "account" => {
                    let (account, after) = split_account(rest);
                    let account = account.trim();
                    if account.is_empty() {
                        return Err(ImportError::at(
                            &at,
                            "This account directive names no account.",
                        ));
                    }
                    let after = after.trim();
                    let mut comments = Vec::new();
                    if let Some(comment) = after.strip_prefix(';') {
                        comments.push(comment.trim().to_string());
                    } else if !after.is_empty() {
                        return Err(ImportError::at(
                            &at,
                            format!("{} isn't an account directive.", line.trim()),
                        ));
                    }
                    self.raw.accounts.push((account.to_string(), comments));
                    block = Block::Account;
                }
                "commodity" => {
                    let (what, _) = split_comment(rest);
                    let what = what.trim();
                    if what.chars().any(|c| c.is_ascii_digit()) {
                        let (symbol, style) =
                            style_from_example(what).map_err(|e| amount_error(&at, e, true))?;
                        self.raw.commodities.push((symbol, Some(style)));
                    } else {
                        let symbol = what.trim_matches('"').to_string();
                        if symbol.is_empty() {
                            return Err(ImportError::at(
                                &at,
                                "This commodity directive names no commodity.",
                            ));
                        }
                        self.raw.commodities.push((symbol, None));
                    }
                    block = Block::Commodity;
                }
                "P" => {
                    self.raw.report.note(Noted::PriceLine, &at, line);
                }
                "payee" => {
                    self.raw.report.note(Noted::PayeeDirective, &at, line);
                    block = Block::Ignored;
                }
                "tag" => {
                    self.raw.report.note(Noted::TagDirective, &at, line);
                    block = Block::Ignored;
                }
                "comment" => {
                    self.raw.report.note(Noted::CommentBlock, &at, line);
                    for (_, l) in lines.by_ref() {
                        if l.trim_end_matches('\r').trim() == "end comment" {
                            break;
                        }
                    }
                }
                _ if first == '=' => {
                    self.raw.report.note(Noted::AutomatedPostings, &at, line);
                    block = Block::Ignored;
                }
                _ if first == '~' => {
                    self.raw.report.note(Noted::PeriodicTransaction, &at, line);
                    block = Block::Ignored;
                }
                "alias" | "apply" | "end" | "decimal-mark" | "D" | "Y" | "year" => {
                    let what = if word == "apply" || word == "end" {
                        line.split_whitespace()
                            .take(3)
                            .collect::<Vec<_>>()
                            .join(" ")
                    } else {
                        word.to_string()
                    };
                    return Err(ImportError::at(&at, format!("{what} isn't supported.")));
                }
                _ if word.starts_with('Y') && word[1..].chars().all(|c| c.is_ascii_digit()) => {
                    return Err(ImportError::at(&at, "Y isn't supported."));
                }
                _ => {
                    return Err(ImportError::at(
                        &at,
                        "This line isn't a transaction or a supported directive.",
                    ));
                }
            }
        }
        Ok(())
    }
}

fn amount_error(at: &Loc, e: AmountError, directive: bool) -> ImportError {
    let message = match e {
        AmountError::TooManyPlaces(text) => {
            format!("{text} has more than 255 decimal places.")
        }
        AmountError::BadSeparator(text) if directive => {
            format!("The example amount {text} needs a decimal mark, like 1000.00 or 1000.")
        }
        AmountError::BadSeparator(text) => format!("{text} uses a digit separator wrongly."),
        AmountError::NoNumber(text) | AmountError::Unreadable(text) => {
            format!("{text} isn't an amount.")
        }
    };
    ImportError::at(at, message)
}

/// The journals the book whose main file is `main` is made of, in read
/// order.
pub fn files(src: &dyn ImportSource, main: &Path) -> Result<Vec<PathBuf>, ImportError> {
    Ok(read_raw(src, main)?.files)
}

fn read_raw(src: &dyn ImportSource, main: &Path) -> Result<Raw, ImportError> {
    let mut reader = Reader {
        src,
        raw: Raw::default(),
        stack: Vec::new(),
    };
    let name = main
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| main.display().to_string());
    reader.read_file(main, &name, None)?;
    Ok(reader.raw)
}

/// Positions by the line each transaction and posting was read from.
struct ByLine<'a> {
    origins: &'a HashMap<u64, (String, usize)>,
    postings: &'a HashMap<u64, Vec<usize>>,
}

impl Locate for ByLine<'_> {
    fn transaction(&self, tx: &Transaction) -> Position {
        let (file, line) = self.origins.get(&tx.id).cloned().unwrap_or_default();
        Position::Line { file, line }
    }

    fn posting(&self, tx: &Transaction, posting: usize) -> Position {
        let (file, line) = self.origins.get(&tx.id).cloned().unwrap_or_default();
        let line = self
            .postings
            .get(&tx.id)
            .and_then(|lines| lines.get(posting).copied())
            .unwrap_or(line);
        Position::Line { file, line }
    }
}

/// A book read but not yet checked.
pub struct Read {
    pub book: Book,
    pub report: ImportReport,
    pub origins: HashMap<u64, (String, usize)>,
    posting_lines: HashMap<u64, Vec<usize>>,
    /// Failures found while reading: transactions that can't be balanced
    /// (they are left out of the book) and assertions without an amount.
    pub failures: Vec<CheckFailure>,
    files: Vec<String>,
}

impl Read {
    /// The strict check, positioned by file and line, with the failures
    /// found while reading.
    pub fn check(&self) -> Vec<CheckFailure> {
        let mut all = self.failures.clone();
        let unbalanced = all.iter().any(|f| f.kind == FailureKind::Balanced);
        all.extend(
            check_located(
                &self.book,
                &ByLine {
                    origins: &self.origins,
                    postings: &self.posting_lines,
                },
            )
            .into_iter()
            .map(|mut f| {
                if unbalanced && f.kind == FailureKind::Assertion {
                    f.details
                        .push_str("; transactions that don't balance are left out");
                }
                f
            }),
        );
        // By kind; within a kind, by file and line, except assertions,
        // which are by date and account.
        let order = |f: &CheckFailure| match (&f.position, f.kind) {
            (Position::Line { file, line }, kind) if kind != FailureKind::Assertion => (
                self.files
                    .iter()
                    .position(|n| n == file)
                    .unwrap_or(usize::MAX),
                *line,
            ),
            _ => (0, 0),
        };
        all.sort_by_key(|f| (f.kind, order(f)));
        all
    }
}

/// Read a journal tree into a book, without the strict check.
pub fn read(src: &dyn ImportSource, main: &Path) -> Result<Read, ImportError> {
    let mut raw = read_raw(src, main)?;
    let mut report = std::mem::take(&mut raw.report);

    // Directives apply wherever they are, so the decimal marks are known
    // before any amount is read. A later directive wins.
    let mut declared: BTreeMap<String, Option<Style>> = BTreeMap::new();
    for (symbol, style) in &raw.commodities {
        let entry = declared.entry(symbol.clone()).or_insert(None);
        if style.is_some() {
            *entry = style.clone();
        }
    }
    let marks: HashMap<String, char> = declared
        .iter()
        .filter_map(|(s, style)| style.as_ref().map(|st| (s.clone(), st.decimal_mark)))
        .collect();
    let mut usage: BTreeMap<String, UsageStyle> = BTreeMap::new();
    let mut read_amount = |text: &str, at: &Loc| -> Result<Amount, ImportError> {
        let parsed = parse_amount(text, |symbol| marks.get(symbol).copied())
            .map_err(|e| amount_error(at, e, false))?;
        usage
            .entry(parsed.amount.commodity.clone())
            .or_default()
            .add(&parsed.observed);
        Ok(parsed.amount)
    };

    let mut book = Book::default();
    for (name, comments) in &raw.accounts {
        let entry = book
            .accounts
            .entry(name.clone())
            .or_insert_with(|| Account {
                name: AccountName::new(name.clone()),
                tags: String::new(),
            });
        let text = comments.join("\n");
        if !text.trim().is_empty() {
            if entry.tags.is_empty() {
                entry.tags = text;
            } else {
                entry.tags = format!("{}\n{}", entry.tags, text.trim_start_matches('\n'));
            }
        }
    }

    let mut origins = HashMap::new();
    let mut failures = Vec::new();
    let mut pending: Vec<(Transaction, Vec<Written>, &RawTx)> = Vec::new();
    for (n, raw_tx) in raw.transactions.iter().enumerate() {
        let id = n as u64 + 1;
        origins.insert(id, (raw_tx.at.file.clone(), raw_tx.at.line));
        let mut comment = raw_tx.comment.clone();
        if let Some(date2) = raw_tx.date2 {
            let note = format!("date2:{}", date2.format("%Y-%m-%d"));
            report.note(Noted::SecondaryDate, &raw_tx.at, &note);
            if comment.is_empty() {
                comment.push(note);
            } else if comment[0].trim().is_empty() {
                comment[0] = note;
            } else {
                comment[0] = format!("{}, {note}", comment[0]);
            }
        }
        let comment = comment.join("\n");
        let tags = kabacus_core::declared_tags(&comment);
        for (name, _) in &tags {
            report.note(Noted::Tag, &raw_tx.at, name);
        }
        let header = Transaction {
            id,
            date: raw_tx.date,
            status: raw_tx.status,
            code: raw_tx.code.clone(),
            description: raw_tx.description.clone(),
            comment,
            tags,
            postings: Vec::new(),
        };
        let mut written = Vec::new();
        let mut assertion_problem = None;
        for rp in &raw_tx.postings {
            let amount = rp
                .amount
                .as_deref()
                .map(|t| read_amount(t, &rp.at))
                .transpose()?;
            let cost = match &rp.cost {
                Some((kind, text)) => Some(Cost {
                    kind: *kind,
                    amount: read_amount(text, &rp.at)?,
                }),
                None => None,
            };
            let balance_assertion = match &rp.assertion {
                Some((op, text)) => Some(BalanceAssertion {
                    amount: read_amount(text, &rp.at)?,
                    total: op.starts_with("=="),
                    inclusive: op.ends_with('*'),
                }),
                None => None,
            };
            if balance_assertion.is_some() && amount.is_none() && assertion_problem.is_none() {
                assertion_problem = Some(assertion_without_amount(
                    &rp.account,
                    Position::Line {
                        file: rp.at.file.clone(),
                        line: rp.at.line,
                    },
                ));
            }
            let comment = rp.comment.join("\n");
            let tags = kabacus_core::declared_tags(&comment);
            for (name, _) in &tags {
                report.note(Noted::Tag, &rp.at, name);
            }
            written.push(Written {
                has_amount: amount.is_some(),
                posting: Posting {
                    account: AccountName::new(rp.account.clone()),
                    amount: amount.unwrap_or_else(|| Amount::new("", Money::zero())),
                    status: rp.status,
                    kind: rp.kind,
                    comment,
                    tags,
                    cost,
                    balance_assertion,
                },
            });
        }
        match assertion_problem {
            Some(problem) => failures.push(problem),
            None => pending.push((header, written, raw_tx)),
        }
    }

    for (symbol, style) in declared {
        let style = style.unwrap_or_else(|| {
            usage
                .get(&symbol)
                .map(|u| u.style(&symbol))
                .unwrap_or_else(|| Style::default_for(&symbol))
        });
        book.commodities
            .insert(symbol.clone(), Commodity { symbol, style });
    }
    let styles = book.styles();

    let mut posting_lines = HashMap::new();
    for (header, written, raw_tx) in pending {
        match infer(&written) {
            Ok(filled) => {
                let mut postings = Vec::new();
                let mut lines = Vec::new();
                for ((w, rp), ps) in written.iter().zip(&raw_tx.postings).zip(filled) {
                    if ps.is_empty() {
                        report.note(
                            Noted::PostingWithoutAmount,
                            &rp.at,
                            format!("{} has nothing to take its amount from", w.posting.account),
                        );
                    }
                    for p in ps {
                        lines.push(rp.at.line);
                        postings.push(p);
                    }
                }
                posting_lines.insert(header.id, lines);
                book.transactions.push(Transaction { postings, ..header });
            }
            Err(problem) => failures.push(unbalanced_failure(
                &header,
                &problem,
                Position::Line {
                    file: raw_tx.at.file.clone(),
                    line: raw_tx.at.line,
                },
                &styles,
            )),
        }
    }

    // More places than a commodity's directive shows raise its places.
    let raised = crate::check::places_needed(&book, &book.transactions);
    for (symbol, needed) in raised {
        let commodity = book.commodities.get_mut(&symbol).expect("declared");
        commodity.style.places = Some(needed.places);
        let (file, line) = origins
            .get(&needed.transaction)
            .cloned()
            .unwrap_or_default();
        let at = Loc { file, line };
        let (places, amount, account) = (needed.places, needed.amount, needed.account);
        let shown = crate::render::render_money(&amount, &book.styles());
        report.note(
            Noted::PlacesRaised,
            &at,
            format!("{symbol} now shows {places} decimal places because of {shown} on {account}."),
        );
    }

    Ok(Read {
        book,
        report,
        origins,
        posting_lines,
        failures,
        files: raw.names,
    })
}

/// Import a journal tree: read it, skip (and report) any assertion that
/// doesn't hold at the end of its day, and refuse a book that fails any
/// other part of the strict check.
pub fn import(src: &dyn ImportSource, main: &Path) -> Result<ImportOutcome, ImportError> {
    let mut read = read(src, main)?;
    let failures = read.check();
    let (assertions, others): (Vec<CheckFailure>, Vec<CheckFailure>) = failures
        .into_iter()
        .partition(|f| f.kind == FailureKind::Assertion && read.failures.iter().all(|r| r != f));
    if !others.is_empty() {
        return Err(ImportError::Invalid(others));
    }
    let skipped: HashSet<(String, usize)> = assertions
        .iter()
        .filter_map(|f| match &f.position {
            Position::Line { file, line } => Some((file.clone(), *line)),
            _ => None,
        })
        .collect();
    for tx in &mut read.book.transactions {
        let lines = read.posting_lines.get(&tx.id).cloned().unwrap_or_default();
        let file = read
            .origins
            .get(&tx.id)
            .map(|o| o.0.clone())
            .unwrap_or_default();
        for (i, p) in tx.postings.iter_mut().enumerate() {
            let line = lines.get(i).copied().unwrap_or(0);
            if p.balance_assertion.is_some() && skipped.contains(&(file.clone(), line)) {
                p.balance_assertion = None;
            }
        }
    }
    for f in assertions {
        if let Position::Line { file, line } = &f.position {
            read.report.entries.push(ReportEntry {
                noted: Noted::AssertionSkipped,
                file: file.clone(),
                line: *line,
                detail: f.sentence.clone(),
            });
        }
    }
    Ok(ImportOutcome {
        book: read.book,
        report: read.report,
        origins: read.origins,
    })
}

#[cfg(test)]
mod tests;
