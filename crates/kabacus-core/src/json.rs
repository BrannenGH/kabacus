// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Deserialization of hledger's JSON wire format (`hledger print -O json`,
//! verified against hledger 1.99.4) into the domain types in [`crate::domain`],
//! for comparing Kabacus's engine with hledger.
//!
//! Only the fields Kabacus needs are modelled; unknown fields are ignored
//! by serde's default behaviour, so this stays robust to hledger versions
//! that add fields we don't care about. Money is exact: `aquantity.floatingPoint`
//! is never read.

use crate::domain::{
    AccountName, BalanceAssertion, Cost, CostKind, Posting, PostingKind, Status, Transaction,
};
use crate::money::{Amount, Money};
use chrono::NaiveDate;
use num_bigint::BigInt;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct QuantityJson {
    #[serde(rename = "decimalMantissa")]
    #[serde(with = "crate::money::wire_mantissa")]
    pub decimal_mantissa: BigInt,
    #[serde(rename = "decimalPlaces")]
    pub decimal_places: u32,
}

#[derive(Debug, Deserialize)]
pub struct AmountJson {
    pub acommodity: String,
    pub aquantity: QuantityJson,
    #[serde(default)]
    pub acost: Option<CostJson>,
}

#[derive(Debug, Deserialize)]
pub struct CostJson {
    pub tag: String,
    pub contents: Box<AmountJson>,
}

impl CostJson {
    fn to_cost(&self) -> Cost {
        Cost {
            kind: if self.tag == "TotalCost" {
                CostKind::Total
            } else {
                CostKind::Unit
            },
            amount: Amount::from(self.contents.as_ref()),
        }
    }
}

impl From<&AmountJson> for Amount {
    fn from(a: &AmountJson) -> Self {
        Amount::new(
            a.acommodity.clone(),
            Money::from_big(
                a.aquantity.decimal_mantissa.clone(),
                a.aquantity.decimal_places,
            ),
        )
    }
}

#[derive(Debug, Deserialize)]
pub struct BalanceAssertionJson {
    pub baamount: AmountJson,
    pub batotal: bool,
    pub bainclusive: bool,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum StatusJson {
    Unmarked,
    Pending,
    Cleared,
}

impl From<StatusJson> for Status {
    fn from(s: StatusJson) -> Self {
        match s {
            StatusJson::Unmarked => Status::Unmarked,
            StatusJson::Pending => Status::Pending,
            StatusJson::Cleared => Status::Cleared,
        }
    }
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum RealJson {
    RealPosting,
    VirtualPosting,
    BalancedVirtualPosting,
}

impl From<RealJson> for PostingKind {
    fn from(r: RealJson) -> Self {
        match r {
            RealJson::RealPosting => PostingKind::Real,
            RealJson::VirtualPosting => PostingKind::Virtual,
            RealJson::BalancedVirtualPosting => PostingKind::BalancedVirtual,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PostingJson {
    pub paccount: String,
    pub pamount: Vec<AmountJson>,
    pub pbalanceassertion: Option<BalanceAssertionJson>,
    #[serde(default)]
    pub pcomment: String,
    pub preal: RealJson,
    pub pstatus: StatusJson,
}

#[derive(Debug, thiserror::Error)]
pub enum FromJsonError {
    #[error("invalid date: {0}")]
    InvalidDate(String),
}

impl PostingJson {
    /// One posting per commodity of the amount, as Kabacus stores them.
    /// A posting hledger shows with no amount reads as one zero.
    fn postings(&self) -> Vec<Posting> {
        let posting = |amount: Amount, cost: Option<Cost>| Posting {
            account: AccountName::new(self.paccount.clone()),
            amount,
            status: Status::from(self.pstatus),
            kind: PostingKind::from(self.preal),
            comment: self.pcomment.clone(),
            // Not `ptags`: those include tags inherited from the account's
            // `account` directive, which aren't the posting's to write.
            tags: crate::comment::declared_tags(&self.pcomment),
            cost,
            balance_assertion: self.pbalanceassertion.as_ref().map(|ba| BalanceAssertion {
                amount: Amount::from(&ba.baamount),
                total: ba.batotal,
                inclusive: ba.bainclusive,
            }),
        };
        if self.pamount.is_empty() {
            return vec![posting(Amount::new("", Money::zero()), None)];
        }
        self.pamount
            .iter()
            .map(|a| posting(Amount::from(a), a.acost.as_ref().map(CostJson::to_cost)))
            .collect()
    }
}

#[derive(Debug, Deserialize)]
pub struct TransactionJson {
    pub tdate: String,
    pub tstatus: StatusJson,
    #[serde(default)]
    pub tcode: String,
    pub tdescription: String,
    #[serde(default)]
    pub tcomment: String,
    #[serde(default)]
    pub ttags: Vec<(String, String)>,
    pub tpostings: Vec<PostingJson>,
    pub tindex: u64,
}

fn parse_date(s: &str) -> Result<NaiveDate, FromJsonError> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| FromJsonError::InvalidDate(s.to_string()))
}

impl TryFrom<&TransactionJson> for Transaction {
    type Error = FromJsonError;
    fn try_from(t: &TransactionJson) -> Result<Self, Self::Error> {
        let postings = t.tpostings.iter().flat_map(PostingJson::postings).collect();
        Ok(Transaction {
            date: parse_date(&t.tdate)?,
            status: Status::from(t.tstatus),
            code: t.tcode.clone(),
            description: t.tdescription.clone(),
            comment: t.tcomment.clone(),
            tags: t.ttags.clone(),
            postings,
            id: t.tindex,
        })
    }
}

pub fn parse_transactions(json: &str) -> Result<Vec<Transaction>, ParseError> {
    let parsed: Vec<TransactionJson> = serde_json::from_str(json)?;
    let transactions = parsed
        .iter()
        .map(Transaction::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(transactions)
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Convert(#[from] FromJsonError),
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real output captured from `hledger -f <fixture> print -O json`
    // (hledger 1.99.4), trimmed to the fields we model.
    const SAMPLE: &str = r#"[
      {
        "tcode": "", "tcomment": "a memo here\ntag1:value1\n",
        "tdate": "2026-01-05", "tdate2": null,
        "tdescription": "Grocery Store", "tindex": 2,
        "tpostings": [
          {
            "paccount": "expenses:groceries",
            "pamount": [{"acommodity": "USD", "aquantity": {"decimalMantissa": 45678, "decimalPlaces": 3, "floatingPoint": 45.678}}],
            "pbalanceassertion": null, "pcomment": "", "preal": "RealPosting",
            "pstatus": "Unmarked", "ptags": []
          },
          {
            "paccount": "assets:checking",
            "pamount": [{"acommodity": "USD", "aquantity": {"decimalMantissa": -45678, "decimalPlaces": 3, "floatingPoint": -45.678}}],
            "pbalanceassertion": {
              "baamount": {"acommodity": "USD", "aquantity": {"decimalMantissa": 954322, "decimalPlaces": 3, "floatingPoint": 954.322}},
              "batotal": false, "bainclusive": false
            },
            "pcomment": "", "preal": "RealPosting", "pstatus": "Cleared", "ptags": []
          }
        ],
        "tprecedingcomment": "", "tstatus": "Pending",
        "ttags": [["tag1", "value1"]]
      }
    ]"#;
    #[test]
    fn parses_real_hledger_json_using_mantissa_not_float() {
        let txs = parse_transactions(SAMPLE).unwrap();
        assert_eq!(txs.len(), 1);
        let tx = &txs[0];
        assert_eq!(tx.postings[0].amount.money.to_string(), "45.678");
        assert_eq!(tx.postings[1].amount.money.to_string(), "-45.678");
        assert_eq!(tx.tags, vec![("tag1".to_string(), "value1".to_string())]);
        assert_eq!(tx.status, Status::Pending);
        assert_eq!(tx.postings[1].status, Status::Cleared);
    }

    #[test]
    fn parses_balance_assertion_amount_exactly() {
        let txs = parse_transactions(SAMPLE).unwrap();
        let ba = txs[0].postings[1].balance_assertion.as_ref().unwrap();
        assert_eq!(ba.amount.money.to_string(), "954.322");
        assert_eq!(ba.operator(), "=");
    }

    /// hledger's `ptags` include tags a posting inherits from its
    /// account's `account` directive; a posting's own tags are only those
    /// its comment declares, so an edit never writes inherited ones into it.
    /// (Captured from hledger 1.99.4 for `account assets:checking  ;
    /// type:A, note:bank`.)
    #[test]
    fn a_postings_tags_are_its_own_not_its_accounts() {
        let json = r##"[{
          "tcode": "#42", "tcomment": "trip comment, id:abc-123\n",
          "tdate": "2026-01-05", "tdate2": "2026-01-07",
          "tdescription": "Grocery", "tindex": 1,
          "tpostings": [{
            "paccount": "assets:checking",
            "pamount": [{"acommodity": "$", "aquantity": {"decimalMantissa": -4500, "decimalPlaces": 2, "floatingPoint": -45}}],
            "pbalanceassertion": null,
            "pcomment": "card:visa\nposting line two\n", "preal": "VirtualPosting",
            "pstatus": "Pending",
            "ptags": [["card", "visa"], ["type", "A"], ["note", "bank"]]
          }],
          "tstatus": "Cleared", "ttags": [["id", "abc-123"]]
        }]"##;
        let tx = &parse_transactions(json).unwrap()[0];
        let posting = &tx.postings[0];
        assert_eq!(posting.tags, vec![("card".to_string(), "visa".to_string())]);
        assert_eq!(posting.comment, "card:visa\nposting line two\n");
        assert_eq!(posting.kind, PostingKind::Virtual);
        assert_eq!(tx.code, "#42");
        assert_eq!(tx.comment, "trip comment, id:abc-123\n");
    }

    #[test]
    fn a_cost_and_an_inferred_amount_in_several_commodities() {
        let json = r#"[{
          "tcode": "", "tcomment": "", "tdate": "2026-01-02", "tdescription": "fx", "tindex": 1,
          "tpostings": [
            {"paccount": "a", "pamount": [{"acommodity": "EUR",
              "acost": {"tag": "TotalCost", "contents": {"acommodity": "USD", "aquantity": {"decimalMantissa": 11, "decimalPlaces": 0}}},
              "aquantity": {"decimalMantissa": 10, "decimalPlaces": 0}}],
             "pbalanceassertion": null, "preal": "RealPosting", "pstatus": "Unmarked"},
            {"paccount": "b", "pamount": [
              {"acommodity": "EUR", "aquantity": {"decimalMantissa": -5, "decimalPlaces": 0}},
              {"acommodity": "USD", "aquantity": {"decimalMantissa": -11, "decimalPlaces": 0}}],
             "pbalanceassertion": null, "preal": "RealPosting", "pstatus": "Unmarked"}
          ],
          "tstatus": "Unmarked", "ttags": []
        }]"#;
        let tx = &parse_transactions(json).unwrap()[0];
        assert_eq!(tx.postings.len(), 3);
        let cost = tx.postings[0].cost.as_ref().unwrap();
        assert_eq!(cost.kind, CostKind::Total);
        assert_eq!(cost.amount.commodity, "USD");
        assert_eq!(tx.postings[1].amount.commodity, "EUR");
        assert_eq!(tx.postings[2].amount.commodity, "USD");
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let with_extra = SAMPLE.replace("\"tindex\": 2,", "\"tindex\": 2, \"tsourcepos\": [1,2],");
        assert!(parse_transactions(&with_extra).is_ok());
    }
}
