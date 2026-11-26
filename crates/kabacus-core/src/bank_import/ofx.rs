// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    decode_text, parse_decimal, statement_description, ImportError, ReportedBalance, Row,
    SourceKind, Statement,
};
use crate::AccountKind;
use chrono::{NaiveDate, NaiveTime};
use quick_xml::{events::Event, Reader};

#[derive(Default)]
struct Fields {
    posted: String,
    amount: String,
    id: String,
    name: String,
    payee: String,
    memo: String,
}

struct StatementFields {
    kind: AccountKind,
    account: String,
    currency: String,
    balance: String,
    balance_date: String,
    rows: Vec<Row>,
}

impl StatementFields {
    fn new(kind: AccountKind) -> Self {
        Self {
            kind,
            account: String::new(),
            currency: String::new(),
            balance: String::new(),
            balance_date: String::new(),
            rows: Vec::new(),
        }
    }

    fn finish(self, places: u32) -> Result<Statement, ImportError> {
        let account_hint = (!self.account.is_empty()).then(|| {
            let last_four: String = self
                .account
                .chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            if self.account.chars().count() <= 4 {
                "…".into()
            } else {
                format!("…{last_four}")
            }
        });
        let reported_balance = if self.balance.is_empty() && self.balance_date.is_empty() {
            None
        } else {
            Some(ReportedBalance {
                date: ofx_date(&self.balance_date, "LEDGERBAL", "DTASOF")?,
                amount: parse_decimal(&self.balance, places)
                    .ok_or_else(|| field("LEDGERBAL", "BALAMT"))?,
            })
        };
        Ok(Statement {
            source: SourceKind::Ofx,
            kind: Some(self.kind),
            currency: Some(self.currency).filter(|currency| !currency.is_empty()),
            account_hint,
            rows: self.rows,
            reported_balance,
        })
    }
}

struct Element {
    name: String,
    text: String,
}

struct OfxParser {
    places: u32,
    row_limit: usize,
    total_rows: usize,
    stack: Vec<Element>,
    row: Option<Fields>,
    statement: Option<StatementFields>,
    statements: Vec<Statement>,
}

impl OfxParser {
    fn start(&mut self, name: &str) -> Result<(), ImportError> {
        if self.stack.len() == 64 {
            return Err(ImportError::Ofx);
        }
        let parent = self.stack.last().map(|parent| parent.name.as_str());
        match (name, parent) {
            ("STMTRS", Some("STMTTRNRS")) if self.statement.is_none() => {
                self.statement = Some(StatementFields::new(AccountKind::Money));
            }
            ("CCSTMTRS", Some("CCSTMTTRNRS")) if self.statement.is_none() => {
                self.statement = Some(StatementFields::new(AccountKind::RevolvingDebt));
            }
            ("STMTTRN", Some("BANKTRANLIST")) if self.statement.is_some() && self.row.is_none() => {
                if self.total_rows == self.row_limit {
                    return Err(ImportError::TooLarge);
                }
                self.total_rows += 1;
                self.row = Some(Fields::default());
            }
            _ => (),
        }
        self.stack.push(Element {
            name: name.into(),
            text: String::new(),
        });
        Ok(())
    }

    fn text(&mut self, value: &str) {
        if let Some(element) = self.stack.last_mut() {
            element.text.push_str(value);
        }
    }

    fn end(&mut self, name: &str) -> Result<(), ImportError> {
        let element = self.stack.pop().ok_or(ImportError::Ofx)?;
        if element.name != name {
            return Err(ImportError::Ofx);
        }
        let Some(statement) = &mut self.statement else {
            return Ok(());
        };
        let value = element.text.trim();
        let parent = self.stack.last().map(|parent| parent.name.as_str());
        if let Some(row) = &mut self.row {
            match (name, parent) {
                ("DTPOSTED", Some("STMTTRN")) => row.posted = value.into(),
                ("TRNAMT", Some("STMTTRN")) => row.amount = value.into(),
                ("FITID", Some("STMTTRN")) => row.id = value.into(),
                ("NAME", Some("STMTTRN")) => row.name = value.into(),
                ("NAME", Some("PAYEE")) => row.payee = value.into(),
                ("MEMO", Some("STMTTRN")) => row.memo = value.into(),
                ("STMTTRN", Some("BANKTRANLIST")) => {
                    let location = format!("OFX transaction {}", statement.rows.len() + 1);
                    if row.id.is_empty() {
                        return Err(field(&location, "FITID"));
                    }
                    statement.rows.push(Row {
                        import_id: SourceKind::Ofx.import_id(&row.id),
                        date: ofx_date(&row.posted, &location, "DTPOSTED")?,
                        amount: parse_decimal(&row.amount, self.places)
                            .ok_or_else(|| field(&location, "TRNAMT"))?,
                        description: statement_description(
                            if row.name.is_empty() {
                                &row.payee
                            } else {
                                &row.name
                            },
                            &row.memo,
                        ),
                    });
                    self.row = None;
                }
                _ => (),
            }
        } else {
            match (name, parent) {
                ("ACCTID", Some("BANKACCTFROM" | "CCACCTFROM")) => statement.account = value.into(),
                ("CURDEF", Some("STMTRS" | "CCSTMTRS")) => {
                    statement.currency = value.to_ascii_uppercase()
                }
                ("BALAMT", Some("LEDGERBAL")) => statement.balance = value.into(),
                ("DTASOF", Some("LEDGERBAL")) => statement.balance_date = value.into(),
                ("STMTRS", Some("STMTTRNRS")) | ("CCSTMTRS", Some("CCSTMTTRNRS")) => {
                    self.statements
                        .push(self.statement.take().unwrap().finish(self.places)?);
                }
                _ => (),
            }
        }
        Ok(())
    }

    fn read_xml(&mut self, text: &str) -> Result<(), ImportError> {
        let mut reader = Reader::from_str(text);
        loop {
            match reader.read_event().map_err(|_| ImportError::Ofx)? {
                Event::Start(tag) => self.start(&name(tag.name().as_ref())?)?,
                Event::End(tag) => self.end(&name(tag.name().as_ref())?)?,
                Event::Empty(tag) => {
                    let name = name(tag.name().as_ref())?;
                    self.start(&name)?;
                    self.end(&name)?;
                }
                Event::Text(text) => {
                    self.text(text.decode().map_err(|_| ImportError::Ofx)?.as_ref())
                }
                Event::CData(text) => {
                    self.text(text.decode().map_err(|_| ImportError::Ofx)?.as_ref())
                }
                Event::GeneralRef(entity) => {
                    let raw = entity.decode().map_err(|_| ImportError::Ofx)?;
                    let encoded = format!("&{raw};");
                    self.text(
                        &quick_xml::escape::unescape(&encoded).map_err(|_| ImportError::Ofx)?,
                    );
                }
                Event::DocType(_) => return Err(ImportError::Ofx),
                Event::Eof if self.stack.is_empty() => return Ok(()),
                Event::Eof => return Err(ImportError::Ofx),
                _ => (),
            }
        }
    }

    fn read_sgml(&mut self, mut text: &str) -> Result<(), ImportError> {
        while let Some((before, tag)) = text.split_once('<') {
            // SGML exports sometimes include bare ampersands in payee names.
            self.text(&quick_xml::escape::unescape(before).unwrap_or_else(|_| before.into()));
            if let Some(comment) = tag.strip_prefix("!--") {
                text = comment.split_once("-->").ok_or(ImportError::Ofx)?.1;
                continue;
            }
            let (tag, after) = tag.split_once('>').ok_or(ImportError::Ofx)?;
            text = after;
            let (closing, tag) = tag
                .strip_prefix('/')
                .map_or((false, tag), |tag| (true, tag));
            let name = name(tag.trim().as_bytes())?;
            if let Some(element) = self.stack.last() {
                if (!closing || name != element.name)
                    && (!element.text.trim().is_empty() || is_field(&element.name))
                {
                    let implicit = element.name.clone();
                    self.end(&implicit)?;
                }
            }
            if closing {
                self.end(&name)?;
            } else {
                self.start(&name)?;
            }
        }
        if !text.trim().is_empty() {
            return Err(ImportError::Ofx);
        }
        Ok(())
    }
}

fn is_field(name: &str) -> bool {
    matches!(
        name,
        "DTPOSTED"
            | "DTUSER"
            | "DTAVAIL"
            | "TRNAMT"
            | "TRNTYPE"
            | "FITID"
            | "CORRECTFITID"
            | "CORRECTACTION"
            | "SRVRTID"
            | "CHECKNUM"
            | "REFNUM"
            | "SIC"
            | "PAYEEID"
            | "NAME"
            | "MEMO"
            | "ACCTID"
            | "CURDEF"
            | "BALAMT"
            | "DTASOF"
    )
}

fn name(raw: &[u8]) -> Result<String, ImportError> {
    if raw.is_empty()
        || !raw
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(b))
    {
        return Err(ImportError::Ofx);
    }
    Ok(String::from_utf8_lossy(raw).to_ascii_uppercase())
}

fn field(location: &str, name: &str) -> ImportError {
    ImportError::Field {
        location: location.into(),
        field: name.into(),
    }
}

fn ofx_date(raw: &str, location: &str, name: &str) -> Result<NaiveDate, ImportError> {
    let invalid = || field(location, name);
    let (date, suffix) = raw.split_at_checked(8).ok_or_else(invalid)?;
    let clock = match suffix.split_once('[') {
        Some((clock, zone))
            if zone
                .strip_suffix(']')
                .is_some_and(|zone| !zone.is_empty() && !zone.contains(['[', ']'])) =>
        {
            clock.trim_end()
        }
        Some(_) => return Err(invalid()),
        None => suffix,
    };
    if !clock.is_empty()
        && (clock.split('.').next().unwrap_or_default().len() != 6
            || NaiveTime::parse_from_str(clock, "%H%M%S%.f").is_err())
    {
        return Err(invalid());
    }
    NaiveDate::parse_from_str(date, "%Y%m%d").map_err(|_| invalid())
}

pub fn parse_ofx(
    bytes: &[u8],
    places: u32,
    row_limit: usize,
) -> Result<Vec<Statement>, ImportError> {
    let text = decode_text(bytes);
    let text = text.trim_start();
    let new_parser = || OfxParser {
        places,
        row_limit,
        total_rows: 0,
        stack: vec![],
        row: None,
        statement: None,
        statements: vec![],
    };
    let mut parser = new_parser();
    if text.starts_with("<?") {
        parser.read_xml(text)?;
    } else if text.starts_with('<') {
        match parser.read_xml(text) {
            Ok(()) => (),
            Err(ImportError::Ofx) => {
                parser = new_parser();
                parser.read_sgml(text)?;
            }
            Err(error) => return Err(error),
        }
    } else {
        let start = text.find('<').ok_or(ImportError::Ofx)?;
        parser.read_sgml(&text[start..])?;
    }
    if !parser.stack.is_empty() {
        return Err(ImportError::Ofx);
    }
    if parser.statements.is_empty() {
        return Err(ImportError::NoStatement);
    }
    Ok(parser.statements)
}
