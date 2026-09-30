// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Reading and writing amounts, and commodity display styles. Implements
//! the specification's "Ledger spec: amounts, commodity styles and
//! precision".

use kabacus_core::{Amount, Money};
use num_bigint::BigInt;
use num_traits::{Signed, Zero};

/// The most decimal places an amount can have.
pub const MAX_PLACES: u32 = kabacus_core::money::MAX_PLACES;

/// Which side of the number a commodity symbol is written on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// How a commodity's amounts are shown. It never changes a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Style {
    pub side: Side,
    /// A space between the symbol and the number.
    pub spaced: bool,
    /// `.` or `,`.
    pub decimal_mark: char,
    pub group_mark: Option<char>,
    /// Digit group sizes from the right; the last one repeats.
    pub group_sizes: Vec<u8>,
    /// The places a `commodity` directive's example gives; `None` for a
    /// commodity declared without one.
    pub places: Option<u32>,
}

impl Style {
    /// A style for a commodity nothing has styled: a one-character symbol
    /// such as `$` on the left, a word such as `USD` on the right.
    pub fn default_for(symbol: &str) -> Style {
        let left = symbol.chars().count() == 1 && !symbol.chars().all(char::is_alphanumeric);
        Style {
            side: if left { Side::Left } else { Side::Right },
            spaced: !left && !symbol.is_empty(),
            decimal_mark: '.',
            group_mark: None,
            group_sizes: Vec::new(),
            places: None,
        }
    }
}

/// Why an amount couldn't be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AmountError {
    #[error("there is no number in {0:?}")]
    NoNumber(String),
    #[error("{0:?} uses a digit separator wrongly")]
    BadSeparator(String),
    #[error("{0:?} has more than 255 decimal places")]
    TooManyPlaces(String),
    #[error("{0:?} isn't an amount")]
    Unreadable(String),
}

/// What reading one amount saw about how it is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    /// `None` when there is no symbol.
    pub side: Option<Side>,
    pub spaced: bool,
    pub decimal_mark: Option<char>,
    pub group_mark: Option<char>,
    pub group_sizes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub amount: Amount,
    pub observed: Observed,
}

/// Characters that end an unquoted commodity symbol.
fn ends_symbol(c: char) -> bool {
    c.is_ascii_digit() || c.is_whitespace() || "-+.@*;\"{}=".contains(c)
}

/// Whether `symbol` must be written in double quotes.
pub fn needs_quotes(symbol: &str) -> bool {
    symbol.chars().any(|c| ends_symbol(c) || c == ',')
}

pub fn render_symbol(symbol: &str) -> String {
    if needs_quotes(symbol) {
        format!("\"{symbol}\"")
    } else {
        symbol.to_string()
    }
}

struct Cursor<'a> {
    chars: Vec<char>,
    at: usize,
    text: &'a str,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.chars.get(self.at + n).copied()
    }

    fn skip_spaces(&mut self) -> bool {
        let start = self.at;
        while self.peek().is_some_and(char::is_whitespace) {
            self.at += 1;
        }
        self.at > start
    }

    fn sign(&mut self) -> Option<bool> {
        match self.peek() {
            Some('-') => {
                self.at += 1;
                Some(true)
            }
            Some('+') => {
                self.at += 1;
                Some(false)
            }
            _ => None,
        }
    }

    fn symbol(&mut self) -> Result<Option<String>, AmountError> {
        match self.peek() {
            Some('"') => {
                let rest: String = self.chars[self.at + 1..].iter().collect();
                let Some(end) = rest.find('"') else {
                    return Err(AmountError::Unreadable(self.text.to_string()));
                };
                let symbol: String = rest[..end].to_string();
                self.at += 1 + symbol.chars().count() + 1;
                Ok(Some(symbol))
            }
            Some(c) if !ends_symbol(c) => {
                let start = self.at;
                while self.peek().is_some_and(|c| !ends_symbol(c)) {
                    self.at += 1;
                }
                Ok(Some(self.chars[start..self.at].iter().collect()))
            }
            _ => Ok(None),
        }
    }

    fn starts_number(&self) -> bool {
        match self.peek() {
            Some(c) if c.is_ascii_digit() => true,
            Some('.' | ',') => self.peek_at(1).is_some_and(|c| c.is_ascii_digit()),
            _ => false,
        }
    }

    /// Digit groups and the marks between them, then any exponent.
    fn number(&mut self) -> (Vec<String>, Vec<char>, Option<i64>) {
        let mut parts = vec![String::new()];
        let mut marks = Vec::new();
        loop {
            match self.peek() {
                Some(c) if c.is_ascii_digit() => {
                    parts.last_mut().expect("one part").push(c);
                    self.at += 1;
                }
                Some(c @ ('.' | ',' | '\'' | '_')) => {
                    marks.push(c);
                    parts.push(String::new());
                    self.at += 1;
                }
                Some(' ')
                    if parts.last().is_some_and(|p| !p.is_empty())
                        && self.peek_at(1).is_some_and(|c| c.is_ascii_digit()) =>
                {
                    marks.push(' ');
                    parts.push(String::new());
                    self.at += 1;
                }
                _ => break,
            }
        }
        let exponent = match (self.peek(), self.peek_at(1), self.peek_at(2)) {
            (Some('e' | 'E'), Some(d), _) if d.is_ascii_digit() => Some(1),
            (Some('e' | 'E'), Some('-' | '+'), Some(d)) if d.is_ascii_digit() => Some(2),
            _ => None,
        }
        .map(|skip| {
            let negative = self.peek_at(1) == Some('-');
            self.at += skip;
            let mut value: i64 = 0;
            while let Some(d) = self.peek().and_then(|c| c.to_digit(10)) {
                value = value.saturating_mul(10).saturating_add(d as i64);
                self.at += 1;
            }
            if negative {
                -value
            } else {
                value
            }
        });
        (parts, marks, exponent)
    }
}

/// Which mark, by index into `marks`, is the decimal mark.
fn decimal_index(
    text: &str,
    marks: &[char],
    known: Option<char>,
) -> Result<Option<usize>, AmountError> {
    let bad = || AmountError::BadSeparator(text.to_string());
    let count = |c: char| marks.iter().filter(|&&m| m == c).count();
    let index = match known {
        Some(d) => match count(d) {
            0 => None,
            1 => Some(marks.iter().position(|&m| m == d).expect("counted")),
            _ => return Err(bad()),
        },
        // Two kinds of mark: the last is the decimal mark. One kind: a lone
        // `.` or `,` is the decimal mark, even before exactly three digits;
        // repeated, it groups digits.
        None => match marks.last() {
            None => None,
            Some(&last) => {
                let decimal = matches!(last, '.' | ',') && count(last) == 1;
                if marks.iter().any(|&m| m != last) {
                    if !decimal {
                        return Err(bad());
                    }
                    Some(marks.len() - 1)
                } else {
                    decimal.then_some(marks.len() - 1)
                }
            }
        },
    };
    if let Some(i) = index {
        if i + 1 != marks.len() {
            return Err(bad());
        }
    }
    let groups = &marks[..index.unwrap_or(marks.len())];
    if groups.iter().any(|&m| m != groups[0]) {
        return Err(bad());
    }
    Ok(index)
}

/// Read one amount: a commodity and a number, in either order.
/// `decimal_mark` is the mark the commodity's directive settles, if any;
/// it is looked up by symbol, so the caller passes a function.
pub fn parse_amount(
    text: &str,
    decimal_mark: impl Fn(&str) -> Option<char>,
) -> Result<Parsed, AmountError> {
    let trimmed = text.trim();
    let mut cur = Cursor {
        chars: trimmed.chars().collect(),
        at: 0,
        text: trimmed,
    };
    let unreadable = || AmountError::Unreadable(trimmed.to_string());
    let mut negative = None;
    negative = negative.or(cur.sign());
    cur.skip_spaces();
    let mut side = None;
    let mut spaced = false;
    let mut symbol = None;
    if !cur.starts_number() {
        symbol = cur.symbol()?;
        if symbol.is_some() {
            side = Some(Side::Left);
            spaced = cur.skip_spaces();
            if let Some(s) = cur.sign() {
                if negative.is_some() {
                    return Err(unreadable());
                }
                negative = Some(s);
                cur.skip_spaces();
            }
        }
    }
    if !cur.starts_number() {
        return Err(AmountError::NoNumber(trimmed.to_string()));
    }
    let (parts, marks, exponent) = cur.number();
    if symbol.is_none() {
        let gap = cur.skip_spaces();
        if let Some(s) = cur.symbol()? {
            symbol = Some(s);
            side = Some(Side::Right);
            spaced = gap;
        }
    }
    cur.skip_spaces();
    if cur.peek().is_some() {
        return Err(unreadable());
    }
    let symbol = symbol.unwrap_or_default();
    let index = decimal_index(trimmed, &marks, decimal_mark(&symbol))?;
    let int_end = index.map_or(parts.len(), |i| i + 1);
    let (int_parts, frac) = (&parts[..int_end], index.map(|i| parts[i + 1].as_str()));
    // A group mark sits between digits.
    if int_parts.len() > 1 && int_parts.iter().any(String::is_empty) {
        return Err(AmountError::BadSeparator(trimmed.to_string()));
    }
    let group_sizes: Vec<u8> = int_parts[1..]
        .iter()
        .rev()
        .map(|p| p.len().min(255) as u8)
        .collect();
    let frac = frac.unwrap_or("");
    let digits: String = int_parts.concat() + frac;
    let mut mantissa: BigInt = if digits.is_empty() {
        BigInt::zero()
    } else {
        digits.parse().map_err(|_| unreadable())?
    };
    let mut places = frac.len() as i64;
    if let Some(e) = exponent {
        places -= e;
        if places < 0 {
            mantissa *= BigInt::from(10).pow((-places) as u32);
            places = 0;
        }
    }
    if places > MAX_PLACES as i64 {
        return Err(AmountError::TooManyPlaces(trimmed.to_string()));
    }
    if negative == Some(true) {
        mantissa = -mantissa;
    }
    Ok(Parsed {
        amount: Amount::new(symbol, Money::from_big(mantissa, places as u32)),
        observed: Observed {
            side,
            spaced,
            decimal_mark: index.map(|i| marks[i]),
            group_mark: (int_parts.len() > 1).then(|| marks[0]),
            group_sizes,
        },
    })
}

/// A `commodity` directive's example amount as a style. The example must
/// have a decimal mark.
pub fn style_from_example(text: &str) -> Result<(String, Style), AmountError> {
    let parsed = parse_amount(text, |_| None)?;
    let Some(decimal_mark) = parsed.observed.decimal_mark else {
        return Err(AmountError::BadSeparator(text.trim().to_string()));
    };
    let symbol = parsed.amount.commodity;
    let mut style = Style::default_for(&symbol);
    if let Some(side) = parsed.observed.side {
        style.side = side;
        style.spaced = parsed.observed.spaced;
    }
    style.decimal_mark = decimal_mark;
    style.group_mark = parsed.observed.group_mark;
    style.group_sizes = parsed.observed.group_sizes;
    style.places = Some(parsed.amount.money.places());
    Ok((symbol, style))
}

/// A commodity's style worked out from how its amounts are written: the
/// first sets side, spacing and mark; grouping comes from any that group.
#[derive(Debug, Default, Clone)]
pub struct UsageStyle {
    first: Option<Observed>,
    decimal_mark: Option<char>,
    group: Option<(char, Vec<u8>)>,
}

impl UsageStyle {
    pub fn add(&mut self, observed: &Observed) {
        if self.first.is_none() {
            self.first = Some(observed.clone());
        }
        if self.decimal_mark.is_none() {
            self.decimal_mark = observed.decimal_mark;
        }
        if self.group.is_none() {
            if let Some(mark) = observed.group_mark {
                self.group = Some((mark, observed.group_sizes.clone()));
            }
        }
    }

    pub fn style(&self, symbol: &str) -> Style {
        let mut style = Style::default_for(symbol);
        if let Some(first) = &self.first {
            if let Some(side) = first.side {
                style.side = side;
                style.spaced = first.spaced;
            }
        }
        if let Some(mark) = self.decimal_mark {
            style.decimal_mark = mark;
        }
        if let Some((mark, sizes)) = &self.group {
            if *mark != style.decimal_mark {
                style.group_mark = Some(*mark);
                style.group_sizes = sizes.clone();
            }
        }
        style
    }
}

fn group_digits(digits: &str, mark: char, sizes: &[u8]) -> String {
    if sizes.is_empty() || sizes.contains(&0) {
        return digits.to_string();
    }
    let mut groups: Vec<&str> = Vec::new();
    let mut end = digits.len();
    let mut i = 0;
    while end > 0 {
        let size = sizes[i.min(sizes.len() - 1)] as usize;
        let start = end.saturating_sub(size);
        groups.push(&digits[start..end]);
        end = start;
        i += 1;
    }
    groups.reverse();
    groups.join(&mark.to_string())
}

/// The number of `money` in `style`, unsigned, padded with zeros up to
/// the style's places.
pub fn render_number(money: &Money, style: &Style) -> String {
    let places = money.places().max(style.places.unwrap_or(0));
    let scaled = money.rescaled(places);
    let digits = scaled.mantissa().abs().to_string();
    let digits = format!("{digits:0>width$}", width = places as usize + 1);
    let (int, frac) = digits.split_at(digits.len() - places as usize);
    // Grouping only where a directive settles which mark is the decimal
    // mark: a lone group mark before three digits would otherwise read
    // as one.
    let int = match (style.group_mark, style.places) {
        (Some(mark), Some(_)) => group_digits(int, mark, &style.group_sizes),
        _ => int.to_string(),
    };
    if places == 0 {
        int
    } else {
        format!("{int}{}{frac}", style.decimal_mark)
    }
}

/// An amount as journal text in its commodity's style. A minus goes before
/// the symbol on the left and before the number on the right.
pub fn render_amount(amount: &Amount, style: &Style) -> String {
    let sign = if amount.money.is_negative() { "-" } else { "" };
    let number = render_number(&amount.money, style);
    if amount.commodity.is_empty() {
        return format!("{sign}{number}");
    }
    let symbol = render_symbol(&amount.commodity);
    let gap = if style.spaced { " " } else { "" };
    match style.side {
        Side::Left => format!("{sign}{symbol}{gap}{number}"),
        Side::Right => format!("{sign}{number}{gap}{symbol}"),
    }
}

/// A `commodity` directive's example for `style`: `$1,000.00`.
pub fn render_example(symbol: &str, style: &Style) -> String {
    let places = style.places.unwrap_or(0);
    let money = Money::from_big(BigInt::from(1000) * BigInt::from(10).pow(places), places);
    let mut number = render_number(&money, style);
    if places == 0 {
        number.push(style.decimal_mark);
    }
    if symbol.is_empty() {
        return number;
    }
    let symbol = render_symbol(symbol);
    let gap = if style.spaced { " " } else { "" };
    match style.side {
        Side::Left => format!("{symbol}{gap}{number}"),
        Side::Right => format!("{number}{gap}{symbol}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> (String, i128, u32) {
        read_with(text, None)
    }

    fn read_with(text: &str, mark: Option<char>) -> (String, i128, u32) {
        let p = parse_amount(text, |_| mark).unwrap();
        let m: i128 = p.amount.money.mantissa().try_into().unwrap();
        (p.amount.commodity, m, p.amount.money.places())
    }

    fn observed(text: &str) -> Observed {
        parse_amount(text, |_| None).unwrap().observed
    }

    #[test]
    fn symbols_on_either_side_with_or_without_a_space() {
        assert_eq!(read("$12.50"), ("$".into(), 1250, 2));
        assert_eq!(read("12.50 USD"), ("USD".into(), 1250, 2));
        assert_eq!(read("12.5USD"), ("USD".into(), 125, 1));
        assert_eq!(read("USD12.5"), ("USD".into(), 125, 1));
        assert_eq!(read("€5"), ("€".into(), 5, 0));
        assert_eq!(read("5€"), ("€".into(), 5, 0));
        assert_eq!(read("£1.50"), ("£".into(), 150, 2));
        assert_eq!(read("12 usd"), ("usd".into(), 12, 0));
        assert_eq!(read("12 U$D"), ("U$D".into(), 12, 0));
        assert!(observed("$ 12.50").spaced);
        assert_eq!(observed("12.50 $").side, Some(Side::Right));
        assert!(!observed("€5").spaced);
    }

    #[test]
    fn quoted_symbols() {
        assert_eq!(read("\"AAPL 2\" 3"), ("AAPL 2".into(), 3, 0));
        assert_eq!(read("3 \"AAPL 2\""), ("AAPL 2".into(), 3, 0));
        assert_eq!(observed("3 \"AAPL 2\"").side, Some(Side::Right));
    }

    #[test]
    fn signs_before_the_symbol_or_the_number() {
        for text in ["-$12.50", "$-12.50", "- $12.50", "$ -12.50", "-12.50 USD"] {
            assert_eq!(read(text).1, -1250, "{text}");
        }
        assert_eq!(read("+$12.5").1, 125);
        assert_eq!(read("$-0"), ("$".into(), 0, 0));
        assert!(parse_amount("$12.50-", |_| None).is_err());
        assert!(parse_amount("12.50- USD", |_| None).is_err());
    }

    #[test]
    fn marks_without_a_directive() {
        assert_eq!(read("$1,234"), ("$".into(), 1234, 3));
        assert_eq!(read("$1.234"), ("$".into(), 1234, 3));
        assert_eq!(read("$1,234.5"), ("$".into(), 12345, 1));
        assert_eq!(read("$1.234,5"), ("$".into(), 12345, 1));
        assert_eq!(read("1,234,567.89 USD").1, 123456789);
        assert_eq!(observed("1,234,567.89 USD").group_sizes, vec![3, 3]);
        assert_eq!(read("1 234.5 USD").1, 12345);
        assert_eq!(observed("1 234.5 USD").group_mark, Some(' '));
        assert_eq!(read("$.5"), ("$".into(), 5, 1));
        assert_eq!(read("$5."), ("$".into(), 5, 0));
        assert_eq!(read("$-1,2"), ("$".into(), -12, 1));
        assert_eq!(read("$1'000.00").1, 100000);
        assert_eq!(read("$0.00"), ("$".into(), 0, 2));
        assert_eq!(read("1_000 USD").1, 1000);
        assert_eq!(read("$1,00,000.00").1, 10000000);
        assert_eq!(observed("$1,00,000.00").group_sizes, vec![3, 2]);
        assert_eq!(read("$1.2.3"), ("$".into(), 123, 0));
        assert_eq!(read("$1,2,3"), ("$".into(), 123, 0));
        assert!(matches!(
            parse_amount("$12.5.", |_| None),
            Err(AmountError::BadSeparator(_))
        ));
    }

    #[test]
    fn exponents_fold_into_the_value() {
        assert_eq!(read("$1e3"), ("$".into(), 1000, 0));
        assert_eq!(read("$1E-2"), ("$".into(), 1, 2));
        assert_eq!(read("1.5e3 EUR"), ("EUR".into(), 1500, 0));
        assert_eq!(read("5 EUR").0, "EUR");
    }

    #[test]
    fn a_symbol_alone_or_foreign_digits_are_not_amounts() {
        assert!(matches!(
            parse_amount("USD", |_| None),
            Err(AmountError::NoNumber(_))
        ));
        assert!(parse_amount("$", |_| None).is_err());
        assert!(parse_amount("$٣", |_| None).is_err());
    }

    #[test]
    fn a_directive_settles_the_decimal_mark() {
        assert_eq!(read_with("$1,234", Some('.')), ("$".into(), 1234, 0));
        assert_eq!(read_with("$1.234", Some(',')), ("$".into(), 1234, 0));
        assert_eq!(read_with("$1,234", Some(',')), ("$".into(), 1234, 3));
        assert_eq!(read_with("1,234.50 USD", Some('.')).1, 123450);
    }

    #[test]
    fn numbers_of_any_size_and_up_to_255_places() {
        let p = parse_amount("$123456789012345678901234567890.12", |_| None).unwrap();
        assert_eq!(
            p.amount.money.to_string(),
            "123456789012345678901234567890.12"
        );
        let at_limit = format!("0.{}1 USD", "0".repeat(254));
        assert_eq!(
            parse_amount(&at_limit, |_| None)
                .unwrap()
                .amount
                .money
                .places(),
            255
        );
        let past = format!("0.{}1 USD", "0".repeat(255));
        assert!(matches!(
            parse_amount(&past, |_| None),
            Err(AmountError::TooManyPlaces(_))
        ));
    }

    #[test]
    fn directive_examples() {
        let (symbol, style) = style_from_example("$1,000.00").unwrap();
        assert_eq!(symbol, "$");
        assert_eq!(style.side, Side::Left);
        assert_eq!(style.group_mark, Some(','));
        assert_eq!(style.places, Some(2));
        let (_, style) = style_from_example("1.000,00 EUR").unwrap();
        assert_eq!((style.decimal_mark, style.group_mark), (',', Some('.')));
        let (_, style) = style_from_example("1 000,00 EUR").unwrap();
        assert_eq!((style.decimal_mark, style.group_mark), (',', Some(' ')));
        assert!(style.spaced);
        let (_, style) = style_from_example("$1,000").unwrap();
        assert_eq!((style.decimal_mark, style.places), (',', Some(3)));
        assert!(style_from_example("$1000").is_err());
        let (symbol, style) = style_from_example("1000.00").unwrap();
        assert_eq!((symbol.as_str(), style.places), ("", Some(2)));
    }

    #[test]
    fn rendering_pads_to_the_declared_places_and_groups() {
        let (_, dollars) = style_from_example("$1,000.00").unwrap();
        let usd = |m: i128, p: u32| Amount::new("$", Money::from_decimal(m, p));
        assert_eq!(render_amount(&usd(12345, 1), &dollars), "$1,234.50");
        assert_eq!(render_amount(&usd(-1250, 2), &dollars), "-$12.50");
        assert_eq!(render_amount(&usd(1001, 3), &dollars), "$1.001");
        let (_, euros) = style_from_example("1.000,00 EUR").unwrap();
        let eur = Amount::new("EUR", Money::from_decimal(-123456, 2));
        assert_eq!(render_amount(&eur, &euros), "-1.234,56 EUR");
        let (_, india) = style_from_example("$1,00,000.00").unwrap();
        assert_eq!(
            render_amount(&usd(1000000000, 2), &india),
            "$1,00,00,000.00"
        );
        let quoted = Amount::new("AAPL 2", Money::from_decimal(3, 0));
        assert_eq!(
            render_amount(&quoted, &Style::default_for("AAPL 2")),
            "3 \"AAPL 2\""
        );
        let bare = Amount::new("", Money::from_decimal(-5, 0));
        assert_eq!(render_amount(&bare, &Style::default_for("")), "-5");
    }

    #[test]
    fn examples_render_as_directives() {
        let (_, style) = style_from_example("$1,000.00").unwrap();
        assert_eq!(render_example("$", &style), "$1,000.00");
        let (_, style) = style_from_example("1000. USD").unwrap();
        assert_eq!(render_example("USD", &style), "1000. USD");
        let (_, style) = style_from_example("1 000,000 EUR").unwrap();
        assert_eq!(render_example("EUR", &style), "1 000,000 EUR");
    }

    #[test]
    fn usage_styles_take_side_from_the_first_and_grouping_from_any() {
        let mut usage = UsageStyle::default();
        for text in ["$1.5", "$ 1.25", "3 $", "$1,000.1"] {
            usage.add(&observed(text));
        }
        let style = usage.style("$");
        assert_eq!((style.side, style.spaced), (Side::Left, false));
        assert_eq!(style.group_mark, Some(','));
        assert_eq!(style.places, None);
    }

    proptest::proptest! {
        /// Rendering in a commodity's style and reading back under its
        /// directive gives the same value.
        #[test]
        fn values_round_trip_through_every_style(
            mantissa in proptest::num::i64::ANY,
            places in 0u32..6,
            example in proptest::sample::select(vec![
                "$1,000.00", "1.000,00 EUR", "1 000,00 CHF", "USD 1000.0", "1000. JPY", "£1'000.000",
            ]),
        ) {
            let (symbol, style) = style_from_example(example).unwrap();
            let money = Money::from_decimal(mantissa as i128, places);
            let text = render_amount(&Amount::new(symbol.clone(), money.clone()), &style);
            let back = parse_amount(&text, |_| Some(style.decimal_mark)).unwrap();
            proptest::prop_assert_eq!(back.amount.commodity, symbol);
            proptest::prop_assert_eq!(back.amount.money, money);
        }
    }
}
