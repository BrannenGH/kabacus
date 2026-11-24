// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{clean_text, decode_text, ImportError, ReportedBalance, Row, SourceKind, Statement};
use crate::{parse_amount, Money};
use chrono::NaiveDate;
use csv::{Reader, ReaderBuilder, StringRecord};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

const SAMPLE_ROWS: usize = 5;
const DATE_FORMATS: &[&str] = &[
    "%Y-%m-%d",
    "%Y/%m/%d",
    "%m/%d/%Y",
    "%d/%m/%Y",
    "%m-%d-%Y",
    "%d-%m-%Y",
    "%d.%m.%Y",
    "%Y%m%d",
    "%b %d, %Y",
    "%d %b %Y",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CsvMapping {
    pub date: usize,
    pub description: Vec<usize>,
    pub date_format: Option<String>,
    pub amount: Option<usize>,
    pub debit: Option<usize>,
    pub credit: Option<usize>,
    pub id: Option<usize>,
    pub balance: Option<usize>,
    pub header: Option<bool>,
    pub delimiter: Option<char>,
    #[serde(default)]
    pub invert: bool,
    #[serde(default)]
    pub skip_rows: usize,
}

#[derive(Debug, Serialize)]
pub struct CsvInspection {
    pub delimiter: char,
    pub header: bool,
    pub columns: Vec<String>,
    pub sample: Vec<Vec<String>>,
    pub mapping: Option<CsvMapping>,
}

fn reader(text: &str, delimiter: u8) -> Reader<&[u8]> {
    ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(text.as_bytes())
}

fn read_sample(text: &str, delimiter: u8, skip_rows: usize) -> Vec<StringRecord> {
    reader(text, delimiter)
        .into_records()
        .skip(skip_rows)
        .take(SAMPLE_ROWS + 1)
        .map_while(Result::ok)
        .collect()
}

fn delimiter(text: &str, skip_rows: usize) -> u8 {
    (*b",;\t")
        .into_iter()
        .rev()
        .max_by_key(|&d| {
            let records = read_sample(text, d, skip_rows);
            let width = records.first().map_or(0, StringRecord::len);
            let same = records.iter().filter(|r| r.len() == width).count();
            (width > 1 && same > 0, same, width)
        })
        .unwrap_or(b',')
}

fn guess_mapping(headers: &StringRecord, delimiter: u8, skip_rows: usize) -> Option<CsvMapping> {
    let column = |names: &[&str]| {
        headers
            .iter()
            .position(|name| names.contains(&name.trim().to_ascii_lowercase().as_str()))
    };
    let date = column(&[
        "date",
        "posted",
        "posted date",
        "posting date",
        "transaction date",
        "booking date",
    ])?;
    let description = column(&[
        "description",
        "payee",
        "name",
        "memo",
        "details",
        "merchant",
    ])?;
    let amount = column(&["amount", "transaction amount", "net amount"]);
    let debit = column(&["debit", "withdrawal", "withdrawals", "money out"]);
    let credit = column(&["credit", "deposit", "deposits", "money in"]);
    if amount.is_none() && (debit.is_none() || credit.is_none()) {
        return None;
    }
    Some(CsvMapping {
        date,
        description: vec![description],
        date_format: None,
        amount,
        debit: debit.filter(|_| amount.is_none()),
        credit: credit.filter(|_| amount.is_none()),
        id: column(&["id", "transaction id", "reference", "reference number"]),
        balance: column(&["balance", "running balance", "ending balance"]),
        header: Some(true),
        delimiter: Some(char::from(delimiter)),
        invert: false,
        skip_rows,
    })
}

fn has_header(records: &[StringRecord]) -> bool {
    let date_cell = |cell: &str| {
        DATE_FORMATS
            .iter()
            .any(|fmt| NaiveDate::parse_from_str(cell.trim(), fmt).is_ok())
    };
    let Some(first) = records.first() else {
        return false;
    };
    !first.iter().any(date_cell)
        && (records
            .get(1)
            .is_some_and(|record| record.iter().any(date_cell))
            || first.iter().any(|cell| {
                matches!(
                    cell.trim().to_ascii_lowercase().as_str(),
                    "date" | "amount" | "description"
                )
            }))
}

pub fn inspect_csv(bytes: &[u8], skip_rows: usize) -> Result<CsvInspection, ImportError> {
    let text = decode_text(bytes);
    let delimiter = delimiter(&text, skip_rows);
    let records = read_sample(&text, delimiter, skip_rows);
    let first = records.first().ok_or(ImportError::Csv { line: 1 })?;
    let header = has_header(&records);
    Ok(CsvInspection {
        delimiter: char::from(delimiter),
        header,
        columns: if header {
            first.iter().map(clean_text).collect()
        } else {
            (1..=first.len()).map(|i| i.to_string()).collect()
        },
        sample: records
            .iter()
            .skip(usize::from(header))
            .take(SAMPLE_ROWS)
            .map(|record| record.iter().map(clean_text).collect())
            .collect(),
        mapping: header
            .then(|| guess_mapping(first, delimiter, skip_rows))
            .flatten(),
    })
}

fn field<'a>(
    record: &'a StringRecord,
    column: usize,
    name: &str,
) -> Result<(&'a str, String), ImportError> {
    let line = record.position().map_or(1, |p| p.line());
    let location = format!("line {line}, column {}", column + 1);
    match record.get(column) {
        Some(value) => Ok((value.trim(), location)),
        None => Err(ImportError::Field {
            location,
            field: name.into(),
        }),
    }
}

fn detect_dates(
    cells: &[(&str, String)],
    format: Option<&str>,
) -> Result<Vec<NaiveDate>, ImportError> {
    if let Some(format) = format {
        return cells
            .iter()
            .map(|(cell, location)| {
                NaiveDate::parse_from_str(cell, format).map_err(|_| ImportError::Field {
                    location: location.clone(),
                    field: "date".into(),
                })
            })
            .collect();
    }
    let mut readings = DATE_FORMATS.iter().filter_map(|fmt| {
        cells
            .iter()
            .map(|(cell, _)| NaiveDate::parse_from_str(cell, fmt).ok())
            .collect::<Option<Vec<_>>>()
    });
    let dates = readings.next().ok_or(ImportError::DateFormat)?;
    if readings.any(|other| dates != other) {
        return Err(ImportError::DateFormat);
    }
    Ok(dates)
}

fn currency_symbol(c: char) -> bool {
    matches!(c, '$' | '€' | '£' | '¥' | '₹' | '₩' | '₽' | '¢')
}

fn read_amount(cell: &str, places: u32) -> Option<Money> {
    let trim = |c: char| c.is_whitespace() || currency_symbol(c);
    let mut value = cell.trim_matches(trim);
    let mut negative = false;
    if let Some(inside) = value.strip_prefix('(').and_then(|v| v.strip_suffix(')')) {
        negative = true;
        value = inside;
    } else if let Some(inside) = value.strip_suffix('-').or_else(|| value.strip_prefix('-')) {
        negative = true;
        value = inside;
    } else if let Some(inside) = value.strip_prefix('+') {
        value = inside;
    }
    let value = value.trim_matches(trim);
    if !value
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | '\'' | '’') || c.is_whitespace())
    {
        return None;
    }
    let last_dot = value.rfind('.');
    let last_comma = value.rfind(',');
    let (decimal, grouping) = match (last_dot, last_comma) {
        (Some(dot), Some(comma)) if comma > dot => (Some(comma), Some('.')),
        (Some(dot), Some(_)) => (Some(dot), Some(',')),
        (Some(dot), None) if value.matches('.').count() == 1 => (Some(dot), None),
        (Some(_), None) => (None, Some('.')),
        (None, Some(comma))
            if value.matches(',').count() == 1
                && (value.len() - comma - 1 != 3
                    || comma == 0
                    || comma > 3
                    || value.starts_with('0')) =>
        {
            (Some(comma), None)
        }
        (None, Some(_)) => (None, Some(',')),
        _ => (None, None),
    };
    let whole = decimal.map_or(value, |idx| &value[..idx]);
    let groups: Vec<&str> = whole
        .split(|c: char| Some(c) == grouping || c.is_whitespace() || matches!(c, '\'' | '’'))
        .collect();
    if groups.len() > 1
        && (!(1..=3).contains(&groups[0].len()) || groups[1..].iter().any(|g| g.len() != 3))
    {
        return None;
    }
    let mut normalized = groups.concat();
    if let Some(decimal) = decimal {
        normalized.push('.');
        normalized.push_str(&value[decimal + 1..]);
    }
    // The only permitted separators left are the decimal point and sign we add.
    if normalized.matches('.').count() > usize::from(decimal.is_some()) || normalized.contains(',')
    {
        return None;
    }
    let amount = parse_amount(&normalized, places).ok()?;
    Some(if negative { -amount } else { amount })
}

fn amount(
    record: &StringRecord,
    column: usize,
    places: u32,
    empty_is_zero: bool,
) -> Result<Money, ImportError> {
    let (cell, location) = field(record, column, "amount")?;
    if cell.is_empty() && empty_is_zero {
        return Ok(Money::zero());
    }
    read_amount(cell, places).ok_or_else(|| ImportError::Field {
        location,
        field: "amount".into(),
    })
}

pub fn parse_csv(
    bytes: &[u8],
    mapping: &CsvMapping,
    places: u32,
    row_limit: usize,
) -> Result<Statement, ImportError> {
    let text = decode_text(bytes);
    let delimiter = match mapping.delimiter {
        Some(c @ (',' | ';' | '\t')) => c as u8,
        Some(_) => return Err(ImportError::CsvMapping),
        None => delimiter(&text, mapping.skip_rows),
    };
    if mapping.description.is_empty() {
        return Err(ImportError::CsvMapping);
    }
    let header = mapping
        .header
        .unwrap_or_else(|| has_header(&read_sample(&text, delimiter, mapping.skip_rows)));
    let mut records = Vec::new();
    for record in reader(&text, delimiter)
        .into_records()
        .skip(mapping.skip_rows)
        .skip(usize::from(header))
    {
        if records.len() == row_limit {
            return Err(ImportError::TooLarge);
        }
        records.push(record.map_err(|err| ImportError::Csv {
            line: err.position().map_or(1, |p| p.line()),
        })?);
    }
    let dates_input = records
        .iter()
        .map(|record| field(record, mapping.date, "date"))
        .collect::<Result<Vec<_>, _>>()?;
    let dates = detect_dates(&dates_input, mapping.date_format.as_deref())?;
    let mut rows = Vec::with_capacity(records.len());
    let mut seen = HashMap::new();
    let mut reported_balance: Option<ReportedBalance> = None;
    for (record, date) in records.iter().zip(dates) {
        let parts = mapping
            .description
            .iter()
            .map(|&column| field(record, column, "description").map(|(text, _)| text))
            .collect::<Result<Vec<_>, _>>()?;
        let description = clean_text(&parts.join(" "));
        let amount = match (mapping.amount, mapping.debit, mapping.credit) {
            (Some(column), None, None) => {
                let money = amount(record, column, places, false)?;
                if mapping.invert {
                    -money
                } else {
                    money
                }
            }
            (None, Some(debit), Some(credit)) if debit != credit => {
                amount(record, credit, places, true)?.abs_value()
                    - amount(record, debit, places, true)?.abs_value()
            }
            _ => return Err(ImportError::CsvMapping),
        };
        let source = SourceKind::Csv;
        let import_id = if let Some(column) = mapping.id {
            let (value, location) = field(record, column, "ID")?;
            if value.is_empty() {
                return Err(ImportError::Field {
                    location,
                    field: "ID".into(),
                });
            }
            source.import_id(value)
        } else {
            let key = format!("{date}\0{}\0{description}", amount.normalized());
            let occurrence = seen.entry(key.clone()).or_insert(0usize);
            *occurrence += 1;
            let hash = Sha256::digest(format!("{key}\0{occurrence}"));
            format!("csv:{}", hex::encode(hash))
        };
        if let Some(column) = mapping.balance {
            let (balance, location) = field(record, column, "balance")?;
            if !balance.is_empty() {
                let amount = read_amount(balance, places).ok_or_else(|| ImportError::Field {
                    location,
                    field: "balance".into(),
                })?;
                if reported_balance
                    .as_ref()
                    .is_none_or(|balance| date >= balance.date)
                {
                    reported_balance = Some(ReportedBalance { date, amount });
                }
            }
        }
        rows.push(Row {
            import_id,
            date,
            amount,
            description,
        });
    }
    Ok(Statement {
        source: SourceKind::Csv,
        kind: None,
        currency: None,
        account_hint: None,
        rows,
        reported_balance,
    })
}
