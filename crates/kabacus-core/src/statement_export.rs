// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! A counterparty statement as CSV and PDF, plus the default
//! file name. Pure functions from a [`Statement`] to bytes, so the API and
//! MCP's `export_statement` give the app and MCP clients identical files.
//!
//! The PDF is written directly (PDF 1.4, one of the standard 14 fonts, so
//! nothing is embedded) rather than through a PDF library: a statement is
//! a single table of text, and the output stays small and deterministic.

use crate::counterparty::Statement;
use crate::format::format_money;
use crate::money::{Amount, Money};
use chrono::NaiveDate;

/// Who the statement is from and when it was issued (the statement's
/// header; the app keeps the sender's name and address in KConfig).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementHeader {
    pub sender_name: String,
    pub sender_address: Vec<String>,
    pub statement_date: NaiveDate,
}

/// `statement-<name>-<end-date>.<extension>`, with anything that isn't
/// safe in a file name replaced by `-`.
pub fn statement_filename(name: &str, to: NaiveDate, extension: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("statement-{safe}-{}.{extension}", to.format("%Y-%m-%d"))
}

/// "Balance due from <name>" when they owe you, "Balance due to
/// <name>" when you owe them.
pub fn closing_label(statement: &Statement) -> String {
    if statement.closing_balance.money.is_negative() {
        format!("Balance due to {}", statement.name)
    } else {
        format!("Balance due from {}", statement.name)
    }
}

/// Charges are positive net amounts, payments negative (shown positive).
fn charge_and_payment(net: &Amount) -> (Option<Amount>, Option<Amount>) {
    if net.money.is_negative() {
        (
            None,
            Some(Amount::new(net.commodity.clone(), -net.money.clone())),
        )
    } else {
        (Some(net.clone()), None)
    }
}

/// One RFC 4180 field. Text that a spreadsheet would run as a formula is
/// prefixed with `'` (descriptions come from bank payee text).
fn csv_text(field: &str) -> String {
    let field = if field.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{field}")
    } else {
        field.to_string()
    };
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field
    }
}

/// The statement as CSV. Amounts are exact decimals without
/// currency symbols so spreadsheets read them as numbers; the first and
/// last rows are the opening and closing net balances.
pub fn statement_csv(statement: &Statement) -> String {
    // One precision for the whole file: a zero balance has no decimal
    // places of its own, but should still read `0.00`.
    let places = std::iter::once(&statement.opening_balance)
        .chain(std::iter::once(&statement.closing_balance))
        .chain(statement.lines.iter().map(|l| &l.net_amount))
        .map(|a| a.money.places())
        .max()
        .unwrap_or(0);
    let exact = |money: Money| money.rescaled(places).to_string();
    let optional = |a: &Option<Amount>| {
        a.as_ref()
            .map(|a| exact(a.money.clone()))
            .unwrap_or_default()
    };

    let mut out = String::from("Date,Description,Charges,Payments,Balance\r\n");
    out.push_str(&format!(
        "{},Opening balance,,,{}\r\n",
        statement.from.format("%Y-%m-%d"),
        exact(statement.opening_balance.money.clone())
    ));
    for line in &statement.lines {
        let (charge, payment) = charge_and_payment(&line.net_amount);
        out.push_str(&format!(
            "{},{},{},{},{}\r\n",
            line.date.format("%Y-%m-%d"),
            csv_text(&line.description),
            optional(&charge),
            optional(&payment),
            exact(line.running_balance.money.clone())
        ));
    }
    out.push_str(&format!(
        "{},{},,,{}\r\n",
        statement.to.format("%Y-%m-%d"),
        csv_text(&closing_label(statement)),
        exact(statement.closing_balance.money.clone())
    ));
    out
}

// --- PDF ------------------------------------------------------------------

const PAGE_WIDTH: f64 = 612.0; // US Letter, in points
const PAGE_HEIGHT: f64 = 792.0;
const MARGIN: f64 = 54.0;
const BODY_SIZE: f64 = 10.0;
const LINE_HEIGHT: f64 = 14.0;

/// Helvetica advance widths (per 1000 em) for ASCII 32..=126, from the
/// standard AFM. Used to right-align amounts and to fit descriptions.
const HELVETICA_WIDTHS: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278,
    278, // ' '..'/'
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, // '0'..'9'
    278, 278, 584, 584, 584, 556, 1015, // ':'..'@'
    667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667,
    611, 722, 667, 944, 667, 667, 611, // 'A'..'Z'
    278, 278, 278, 469, 556, 333, // '['..'`'
    556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500,
    278, 556, 500, 722, 500, 500, 500, // 'a'..'z'
    334, 260, 334, 584, // '{'..'~'
];

fn text_width(text: &str, size: f64) -> f64 {
    let units: u32 = text
        .chars()
        .map(|c| match c as u32 {
            code @ 32..=126 => HELVETICA_WIDTHS[(code - 32) as usize] as u32,
            _ => 556,
        })
        .sum();
    units as f64 * size / 1000.0
}

/// Shorten `text` with an ellipsis until it fits in `width`.
fn fit(text: &str, width: f64, size: f64) -> String {
    if text_width(text, size) <= width {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + "...";
        if text_width(&candidate, size) <= width {
            return candidate;
        }
    }
    String::new()
}

/// A PDF string literal in WinAnsiEncoding: Latin-1 characters map to
/// their own byte, `€` to 0x80, anything else to `?`.
fn pdf_string(text: &str) -> String {
    let mut out = String::from("(");
    for c in text.chars() {
        match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            ' '..='~' => out.push(c),
            '€' => out.push_str("\\200"),
            '\u{a0}'..='\u{ff}' => out.push_str(&format!("\\{:03o}", c as u32)),
            _ => out.push('?'),
        }
    }
    out.push(')');
    out
}

/// Accumulates one page's content stream.
struct PageBuilder {
    ops: String,
}

impl PageBuilder {
    fn text(&mut self, font: &str, size: f64, x: f64, y: f64, text: &str) {
        self.ops.push_str(&format!(
            "BT /{font} {size} Tf {x:.2} {y:.2} Td {} Tj ET\n",
            pdf_string(text)
        ));
    }

    fn text_right(&mut self, right_edge: f64, y: f64, text: &str) {
        let x = right_edge - text_width(text, BODY_SIZE);
        self.text("F1", BODY_SIZE, x, y, text);
    }

    fn rule(&mut self, y: f64) {
        self.ops.push_str(&format!(
            "0.5 w {MARGIN:.2} {y:.2} m {:.2} {y:.2} l S\n",
            PAGE_WIDTH - MARGIN
        ));
    }
}

// Column positions: Date | Description | Charges | Payments | Balance.
const COL_DATE: f64 = MARGIN;
const COL_DESCRIPTION: f64 = MARGIN + 70.0;
const RIGHT_CHARGES: f64 = 390.0;
const RIGHT_PAYMENTS: f64 = 474.0;
const RIGHT_BALANCE: f64 = PAGE_WIDTH - MARGIN;

fn table_header(page: &mut PageBuilder, y: f64) {
    page.text("F2", BODY_SIZE, COL_DATE, y, "Date");
    page.text("F2", BODY_SIZE, COL_DESCRIPTION, y, "Description");
    for (right, title) in [
        (RIGHT_CHARGES, "Charges"),
        (RIGHT_PAYMENTS, "Payments"),
        (RIGHT_BALANCE, "Balance"),
    ] {
        let x = right - text_width(title, BODY_SIZE) * 1.05; // bold is a bit wider
        page.text("F2", BODY_SIZE, x, y, title);
    }
    page.rule(y - 4.0);
}

/// The statement as a PDF: the sender and counterparty
/// header, the opening balance, one row per line with a running net
/// balance, and the closing balance with its "due from/to" label. Long
/// statements continue on further pages, each with the table header and a
/// page number.
pub fn statement_pdf(statement: &Statement, header: &StatementHeader) -> Vec<u8> {
    let bottom = MARGIN + LINE_HEIGHT * 2.0; // leave room for the page number
    let description_width = RIGHT_CHARGES - 70.0 - COL_DESCRIPTION;

    let mut pages: Vec<PageBuilder> = Vec::new();
    let mut page = PageBuilder { ops: String::new() };
    let mut y = PAGE_HEIGHT - MARGIN;

    // Letterhead.
    page.text("F2", 14.0, MARGIN, y, &header.sender_name);
    y -= LINE_HEIGHT * 1.3;
    for line in &header.sender_address {
        page.text("F1", BODY_SIZE, MARGIN, y, line);
        y -= LINE_HEIGHT;
    }
    y -= LINE_HEIGHT;
    page.text(
        "F2",
        12.0,
        MARGIN,
        y,
        &format!("Statement for {}", statement.name),
    );
    y -= LINE_HEIGHT * 1.3;
    page.text(
        "F1",
        BODY_SIZE,
        MARGIN,
        y,
        &format!(
            "Period: {} to {}",
            statement.from.format("%Y-%m-%d"),
            statement.to.format("%Y-%m-%d")
        ),
    );
    y -= LINE_HEIGHT;
    page.text(
        "F1",
        BODY_SIZE,
        MARGIN,
        y,
        &format!(
            "Statement date: {}",
            header.statement_date.format("%Y-%m-%d")
        ),
    );
    y -= LINE_HEIGHT * 2.0;

    table_header(&mut page, y);
    y -= LINE_HEIGHT * 1.3;

    let mut rows: Vec<[String; 5]> = vec![[
        statement.from.format("%Y-%m-%d").to_string(),
        "Opening balance".to_string(),
        String::new(),
        String::new(),
        format_money(&statement.opening_balance),
    ]];
    for line in &statement.lines {
        let (charge, payment) = charge_and_payment(&line.net_amount);
        rows.push([
            line.date.format("%Y-%m-%d").to_string(),
            line.description.clone(),
            charge.as_ref().map(format_money).unwrap_or_default(),
            payment.as_ref().map(format_money).unwrap_or_default(),
            format_money(&line.running_balance),
        ]);
    }

    for [date, description, charge, payment, balance] in rows {
        if y < bottom {
            pages.push(page);
            page = PageBuilder { ops: String::new() };
            y = PAGE_HEIGHT - MARGIN;
            table_header(&mut page, y);
            y -= LINE_HEIGHT * 1.3;
        }
        page.text("F1", BODY_SIZE, COL_DATE, y, &date);
        let description = fit(&description, description_width, BODY_SIZE);
        page.text("F1", BODY_SIZE, COL_DESCRIPTION, y, &description);
        page.text_right(RIGHT_CHARGES, y, &charge);
        page.text_right(RIGHT_PAYMENTS, y, &payment);
        page.text_right(RIGHT_BALANCE, y, &balance);
        y -= LINE_HEIGHT;
    }

    if y < bottom + LINE_HEIGHT {
        pages.push(page);
        page = PageBuilder { ops: String::new() };
        y = PAGE_HEIGHT - MARGIN;
    }
    page.rule(y + LINE_HEIGHT - 4.0);
    y -= 4.0;
    page.text(
        "F2",
        BODY_SIZE,
        COL_DESCRIPTION,
        y,
        &closing_label(statement),
    );
    let closing = format_money(&statement.closing_balance);
    let x = RIGHT_BALANCE - text_width(&closing, BODY_SIZE) * 1.05;
    page.text("F2", BODY_SIZE, x, y, &closing);
    pages.push(page);

    let count = pages.len();
    for (i, page) in pages.iter_mut().enumerate() {
        let label = format!("Page {} of {count}", i + 1);
        let x = PAGE_WIDTH - MARGIN - text_width(&label, 8.0);
        page.text("F1", 8.0, x, MARGIN, &label);
    }
    write_pdf(&pages)
}

/// Serialize pages into a PDF file with a correct cross-reference table.
fn write_pdf(pages: &[PageBuilder]) -> Vec<u8> {
    // Objects: 1 catalog, 2 page tree, 3 Helvetica, 4 Helvetica-Bold, then
    // a (page, contents) pair per page.
    let page_id = |i: usize| 5 + 2 * i;
    let kids: Vec<String> = (0..pages.len())
        .map(|i| format!("{} 0 R", page_id(i)))
        .collect();
    let mut objects: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            pages.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_string(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>"
            .to_string(),
    ];
    for (i, page) in pages.iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_WIDTH} {PAGE_HEIGHT}] \
             /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {} 0 R >>",
            page_id(i) + 1
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{}endstream",
            page.ops.len(),
            page.ops
        ));
    }

    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.push_str(&format!("{} 0 obj\n{object}\nendobj\n", i + 1));
    }
    let xref_offset = out.len();
    out.push_str(&format!(
        "xref\n0 {}\n0000000000 65535 f \n",
        objects.len() + 1
    ));
    for offset in offsets {
        out.push_str(&format!("{offset:010} 00000 n \n"));
    }
    out.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
        objects.len() + 1
    ));
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::counterparty::StatementLine;

    fn usd(cents: i128) -> Amount {
        Amount::new("USD", Money::from_decimal(cents, 2))
    }

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn statement(lines: usize) -> Statement {
        let mut running = 1000;
        let lines = (0..lines)
            .map(|i| {
                let net = if i % 3 == 2 { -2500 } else { 4200 };
                running += net;
                StatementLine {
                    date: date("2026-02-01") + chrono::Days::new(i as u64 % 27),
                    description: if i == 0 {
                        "Dinner, \"split\" (half)".to_string()
                    } else {
                        format!("Line {i}")
                    },
                    net_amount: usd(net),
                    running_balance: usd(running),
                    open: false,
                }
            })
            .collect();
        Statement {
            name: "jordan".to_string(),
            from: date("2026-02-01"),
            to: date("2026-02-28"),
            opening_balance: usd(1000),
            lines,
            closing_balance: usd(running),
        }
    }

    fn header() -> StatementHeader {
        StatementHeader {
            sender_name: "Sam Sender".to_string(),
            sender_address: vec!["1 Example Way".to_string(), "Springfield".to_string()],
            statement_date: date("2026-03-01"),
        }
    }
    #[test]
    fn filename_is_statement_name_end_date() {
        assert_eq!(
            statement_filename("jordan", date("2026-02-28"), "pdf"),
            "statement-jordan-2026-02-28.pdf"
        );
        assert_eq!(
            statement_filename("mary jo/../x", date("2026-02-28"), "csv"),
            "statement-mary-jo-..-x-2026-02-28.csv"
        );
    }
    #[test]
    fn csv_has_opening_lines_and_labelled_closing() {
        let csv = statement_csv(&statement(3));
        let rows: Vec<&str> = csv.split("\r\n").collect();
        assert_eq!(rows[0], "Date,Description,Charges,Payments,Balance");
        assert_eq!(rows[1], "2026-02-01,Opening balance,,,10.00");
        assert_eq!(
            rows[2],
            "2026-02-01,\"Dinner, \"\"split\"\" (half)\",42.00,,52.00"
        );
        assert_eq!(rows[4], "2026-02-03,Line 2,,25.00,69.00");
        assert_eq!(rows[5], "2026-02-28,Balance due from jordan,,,69.00");
    }
    #[test]
    fn csv_amounts_share_one_precision() {
        // A zero opening balance has no decimal places of its own.
        let mut s = statement(1);
        s.opening_balance = Amount::new("USD", Money::zero());
        let csv = statement_csv(&s);
        assert!(
            csv.contains("\r\n2026-02-01,Opening balance,,,0.00\r\n"),
            "{csv}"
        );
    }
    #[test]
    fn csv_defuses_spreadsheet_formulas() {
        assert_eq!(csv_text("=HYPERLINK(\"x\")"), "\"'=HYPERLINK(\"\"x\"\")\"");
        assert_eq!(csv_text("-refund"), "'-refund");
        assert_eq!(csv_text("Plain"), "Plain");
    }
    #[test]
    fn closing_label_says_who_owes_whom() {
        let mut s = statement(0);
        assert_eq!(closing_label(&s), "Balance due from jordan");
        s.closing_balance = usd(-500);
        assert_eq!(closing_label(&s), "Balance due to jordan");
    }

    /// The xref table must point at each object exactly, or readers
    /// complain or silently repair the file.
    fn assert_well_formed(pdf: &[u8]) {
        let text = String::from_utf8(pdf.to_vec()).unwrap();
        assert!(text.starts_with("%PDF-1.4\n") && text.ends_with("%%EOF\n"));
        let startxref: usize = text
            .rsplit("startxref\n")
            .next()
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(text[startxref..].starts_with("xref\n"));
        let entries: Vec<&str> = text[startxref..]
            .lines()
            .skip(3)
            .take_while(|l| !l.starts_with("trailer"))
            .collect();
        for (i, entry) in entries.iter().enumerate() {
            let offset: usize = entry[..10].parse().unwrap();
            assert!(
                text[offset..].starts_with(&format!("{} 0 obj\n", i + 1)),
                "xref entry {} is off",
                i + 1
            );
        }
        // Stream lengths match their contents.
        for (at, _) in text.match_indices("/Length ") {
            let length: usize = text[at + 8..].split(' ').next().unwrap().parse().unwrap();
            let start = text[at..].find("stream\n").unwrap() + at + 7;
            assert!(text[start + length..].starts_with("endstream"));
        }
    }
    #[test]
    fn pdf_is_well_formed_and_paginates() {
        let one_page = statement_pdf(&statement(5), &header());
        assert_well_formed(&one_page);
        assert!(String::from_utf8_lossy(&one_page).contains("/Count 1 "));

        let long = statement_pdf(&statement(120), &header());
        assert_well_formed(&long);
        let text = String::from_utf8_lossy(&long);
        assert!(text.contains("/Count 3 "), "120 lines should need 3 pages");
        assert!(text.contains("(Page 3 of 3)"));
        // Deterministic: the same statement gives the same bytes.
        assert_eq!(long, statement_pdf(&statement(120), &header()));
    }
    #[test]
    fn pdf_strings_are_escaped_and_encoded() {
        assert_eq!(pdf_string("a (b) \\ c"), "(a \\(b\\) \\\\ c)");
        assert_eq!(pdf_string("Café €5"), "(Caf\\351 \\2005)");
        assert_eq!(pdf_string("日本"), "(??)");
    }
    #[test]
    fn long_descriptions_are_shortened_to_fit() {
        let fitted = fit(&"W".repeat(200), 100.0, BODY_SIZE);
        assert!(fitted.ends_with("..."));
        assert!(text_width(&fitted, BODY_SIZE) <= 100.0);
        assert_eq!(fit("Short", 100.0, BODY_SIZE), "Short");
    }
}
