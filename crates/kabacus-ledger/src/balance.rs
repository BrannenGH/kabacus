// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Filling in an omitted amount and checking that a transaction balances.
//! Implements the specification's "Ledger spec: inferring the missing
//! amount and balancing" and the engine implementation specification,
//! sections 1.1 (item 7) and 2.2 (`infer`).

use kabacus_core::{Amount, Money, Posting, PostingKind};
use num_bigint::BigInt;
use num_traits::Zero;
use std::collections::BTreeMap;

/// The postings balanced together. Unbalanced virtual postings are in
/// neither group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Real,
    BalancedVirtual,
}

impl Group {
    pub fn of(kind: PostingKind) -> Option<Group> {
        match kind {
            PostingKind::Real => Some(Group::Real),
            PostingKind::BalancedVirtual => Some(Group::BalancedVirtual),
            PostingKind::Virtual => None,
        }
    }
}

/// Exact sums per commodity, of any size and any number of places.
#[derive(Debug, Clone, Default)]
pub struct Sums(BTreeMap<String, (BigInt, u32)>);

impl Sums {
    pub fn add(&mut self, amount: &Amount) {
        let entry = self
            .0
            .entry(amount.commodity.clone())
            .or_insert((BigInt::zero(), 0));
        let places = entry.1.max(amount.money.places());
        let scale = |m: &BigInt, from: u32| m * BigInt::from(10).pow(places - from);
        entry.0 = scale(&entry.0, entry.1) + scale(amount.money.mantissa(), amount.money.places());
        entry.1 = places;
    }

    /// Every commodity summed, in symbol order (the empty one first).
    pub fn amounts(&self) -> Vec<Amount> {
        self.0
            .iter()
            .map(|(c, (m, p))| Amount::new(c.clone(), Money::from_big(m.clone(), *p)))
            .collect()
    }

    /// The commodities that don't sum to zero.
    pub fn nonzero(&self) -> Vec<Amount> {
        self.amounts()
            .into_iter()
            .filter(|a| !a.money.is_zero())
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// What a posting counts as when balancing: its cost if it has one,
/// otherwise its amount.
pub fn balancing_amount(posting: &Posting) -> Amount {
    match &posting.cost {
        Some(cost) => cost.total_for(&posting.amount),
        None => posting.amount.clone(),
    }
}

/// A transaction that can't be balanced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unbalanced {
    /// More than one posting in `group` leaves out its amount.
    TwoOmitted { group: Group },
    /// `group` sums to `left_over` instead of zero.
    LeftOver {
        group: Group,
        left_over: Vec<Amount>,
    },
}

/// A posting as written: its amount may be left out.
#[derive(Debug, Clone)]
pub struct Written {
    pub posting: Posting,
    pub has_amount: bool,
}

/// Fill in each group's omitted amount: the negated sum of the others,
/// per commodity, in symbol order. One posting is returned per commodity it
/// takes; a posting with nothing to take an amount from is returned as
/// none at all. The result balances, or the reason it can't is returned.
pub fn infer(written: &[Written]) -> Result<Vec<Vec<Posting>>, Unbalanced> {
    for group in [Group::Real, Group::BalancedVirtual] {
        let omitted = written
            .iter()
            .filter(|w| !w.has_amount && Group::of(w.posting.kind) == Some(group))
            .count();
        if omitted > 1 {
            return Err(Unbalanced::TwoOmitted { group });
        }
    }
    let sums = |group: Group| {
        let mut sums = Sums::default();
        for w in written
            .iter()
            .filter(|w| w.has_amount && Group::of(w.posting.kind) == Some(group))
        {
            sums.add(&balancing_amount(&w.posting));
        }
        sums
    };
    let mut out = Vec::with_capacity(written.len());
    for w in written {
        if w.has_amount {
            out.push(vec![w.posting.clone()]);
            continue;
        }
        let Some(group) = Group::of(w.posting.kind) else {
            out.push(Vec::new());
            continue;
        };
        let sums = sums(group);
        let nonzero = sums.nonzero();
        let amounts = if nonzero.is_empty() {
            sums.amounts()
                .into_iter()
                .map(|a| Amount::new(a.commodity, Money::from_decimal(0, a.money.places())))
                .collect()
        } else {
            nonzero
        };
        out.push(
            amounts
                .into_iter()
                .enumerate()
                .map(|(i, amount)| Posting {
                    amount: Amount::new(amount.commodity, -amount.money),
                    comment: if i == 0 {
                        w.posting.comment.clone()
                    } else {
                        String::new()
                    },
                    tags: if i == 0 {
                        w.posting.tags.clone()
                    } else {
                        Vec::new()
                    },
                    ..w.posting.clone()
                })
                .collect(),
        );
    }
    let postings: Vec<Posting> = out.iter().flatten().cloned().collect();
    check_balanced(&postings)?;
    Ok(out)
}

/// Each group sums to exactly zero in every commodity.
pub fn check_balanced(postings: &[Posting]) -> Result<(), Unbalanced> {
    for group in [Group::Real, Group::BalancedVirtual] {
        let mut sums = Sums::default();
        for p in postings.iter().filter(|p| Group::of(p.kind) == Some(group)) {
            sums.add(&balancing_amount(p));
        }
        let left_over = sums.nonzero();
        if !left_over.is_empty() {
            return Err(Unbalanced::LeftOver { group, left_over });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kabacus_core::{AccountName, Cost, CostKind, Status};

    fn amount(text: &str) -> Amount {
        crate::amount::parse_amount(text, |_| None).unwrap().amount
    }

    fn written(account: &str, text: Option<&str>) -> Written {
        let (kind, name) = if let Some(inner) = account.strip_prefix('[') {
            (PostingKind::BalancedVirtual, inner.trim_end_matches(']'))
        } else if let Some(inner) = account.strip_prefix('(') {
            (PostingKind::Virtual, inner.trim_end_matches(')'))
        } else {
            (PostingKind::Real, account)
        };
        Written {
            posting: Posting {
                account: AccountName::new(name),
                amount: text
                    .map(amount)
                    .unwrap_or_else(|| Amount::new("", Money::zero())),
                status: Status::Unmarked,
                kind,
                comment: String::new(),
                tags: vec![],
                cost: None,
                balance_assertion: None,
            },
            has_amount: text.is_some(),
        }
    }

    fn inferred(postings: &[Written]) -> Vec<Vec<String>> {
        infer(postings)
            .unwrap()
            .iter()
            .map(|ps| {
                ps.iter()
                    .map(|p| format!("{} {}", p.amount.money, p.amount.commodity))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn the_omitted_amount_is_the_negated_sum_per_commodity_in_symbol_order() {
        // J2, L10, L11
        let got = inferred(&[
            written("expenses:e", Some("10 USD")),
            written("expenses:e", Some("5 EUR")),
            written("assets:a", None),
        ]);
        assert_eq!(got[2], vec!["-5 EUR", "-10 USD"]);
        let got = inferred(&[
            written("expenses:e", Some("10 ZAR")),
            written("expenses:e", Some("4 AUD")),
            written("expenses:e", Some("3 USD")),
            written("expenses:e", Some("2 EUR")),
            written("assets:a", None),
        ]);
        assert_eq!(got[4], vec!["-4 AUD", "-2 EUR", "-3 USD", "-10 ZAR"]);
        let got = inferred(&[
            written("expenses:e", Some("10 USD")),
            written("expenses:e", Some("4")),
            written("assets:a", None),
        ]);
        assert_eq!(got[2], vec!["-4 ", "-10 USD"]);
    }

    #[test]
    fn netted_commodities_get_no_amount_and_all_zero_gives_a_zero() {
        // J4, J6, M11, J12
        let got = inferred(&[
            written("expenses:e", Some("10 USD")),
            written("expenses:e", Some("5 EUR")),
            written("assets:a", Some("-10 USD")),
            written("assets:b", None),
        ]);
        assert_eq!(got[3], vec!["-5 EUR"]);
        let got = inferred(&[
            written("expenses:e", Some("10 USD")),
            written("assets:a", Some("-10 USD")),
            written("assets:b", None),
        ]);
        assert_eq!(got[2], vec!["0 USD"]);
        let got = inferred(&[
            written("expenses:e", Some("1.50 USD")),
            written("expenses:e", Some("-1.5 USD")),
            written("assets:b", None),
        ]);
        assert_eq!(got[2], vec!["0.00 USD"]);
        let got = inferred(&[
            written("expenses:e", Some("0 USD")),
            written("expenses:e", Some("0 EUR")),
            written("assets:a", None),
        ]);
        assert_eq!(got[2], vec!["0 EUR", "0 USD"]);
    }

    #[test]
    fn places_come_from_the_most_precise_amount_and_position_doesnt_matter() {
        // J10, J11, M4
        assert_eq!(
            inferred(&[written("e", Some("10.005 USD")), written("a", None)])[1],
            vec!["-10.005 USD"]
        );
        assert_eq!(
            inferred(&[
                written("e", Some("1.5 USD")),
                written("e", Some("1.25 USD")),
                written("a", None)
            ])[2],
            vec!["-2.75 USD"]
        );
        assert_eq!(
            inferred(&[
                written("a", None),
                written("e", Some("4 USD")),
                written("i", Some("-1.25 USD"))
            ])[0],
            vec!["-2.75 USD"]
        );
    }

    #[test]
    fn groups_are_inferred_and_balanced_separately() {
        // L4, L7, L8
        let got = inferred(&[
            written("expenses:e", Some("10 USD")),
            written("assets:a", None),
            written("[assets:b]", Some("5 USD")),
            written("[income:i]", None),
        ]);
        assert_eq!(got[1], vec!["-10 USD"]);
        assert_eq!(got[3], vec!["-5 USD"]);
        let got = inferred(&[
            written("expenses:e", Some("10 USD")),
            written("assets:a", Some("-10 USD")),
            written("(v)", None),
            written("(assets:b)", Some("3 EUR")),
        ]);
        assert!(
            got[2].is_empty(),
            "an unbalanced virtual posting takes nothing"
        );
    }

    #[test]
    fn two_omitted_in_a_group_is_refused_but_one_per_group_is_fine() {
        // I3, N2, N1
        assert_eq!(
            infer(&[
                written("e", Some("10 USD")),
                written("a", None),
                written("b", None)
            ])
            .unwrap_err(),
            Unbalanced::TwoOmitted { group: Group::Real }
        );
        assert_eq!(
            infer(&[
                written("e", Some("1 USD")),
                written("a", Some("-1 USD")),
                written("[v]", None),
                written("[b]", None)
            ])
            .unwrap_err(),
            Unbalanced::TwoOmitted {
                group: Group::BalancedVirtual
            }
        );
        assert!(infer(&[
            written("e", Some("1 USD")),
            written("a", Some("-1 USD")),
            written("(v)", None),
            written("(b)", None)
        ])
        .is_ok());
    }

    fn left_over(postings: &[Written]) -> Unbalanced {
        infer(postings).unwrap_err()
    }

    #[test]
    fn balancing_is_exact_per_commodity() {
        // I5, I7, I10, K1, K9, M2, M10, L3, L5, L9
        let usd = |m: i128, p: u32| Amount::new("USD", Money::from_decimal(m, p));
        assert_eq!(
            left_over(&[
                written("e", Some("10 USD")),
                written("e", Some("5 EUR")),
                written("a", Some("-5 USD")),
                written("b", Some("-5 EUR"))
            ]),
            Unbalanced::LeftOver {
                group: Group::Real,
                left_over: vec![usd(5, 0)]
            }
        );
        assert!(matches!(
            left_over(&[written("e", Some("5 USD"))]),
            Unbalanced::LeftOver { .. }
        ));
        assert_eq!(
            left_over(&[written("e", Some("10 USD")), written("i", Some("-10"))]),
            Unbalanced::LeftOver {
                group: Group::Real,
                left_over: vec![Amount::new("", Money::from_decimal(-10, 0)), usd(10, 0)]
            }
        );
        assert_eq!(
            left_over(&[
                written("e", Some("1.001 USD")),
                written("a", Some("-1.00 USD"))
            ]),
            Unbalanced::LeftOver {
                group: Group::Real,
                left_over: vec![usd(1, 3)]
            }
        );
        assert!(infer(&[
            written("e", Some("0.333 USD")),
            written("e", Some("0.333 USD")),
            written("e", Some("0.333 USD")),
            written("a", Some("-1 USD"))
        ])
        .is_err());
        assert!(infer(&[written("e", Some("5 USD")), written("a", Some("-5 usd"))]).is_err());
        assert!(infer(&[
            written("e", Some("0.10 USD")),
            written("e", Some("0.20 USD")),
            written("a", Some("-0.3 USD"))
        ])
        .is_ok());
        assert_eq!(
            left_over(&[
                written("e", Some("10 USD")),
                written("a", Some("-10 USD")),
                written("[b]", Some("5 USD")),
                written("[i]", Some("-4 USD"))
            ]),
            Unbalanced::LeftOver {
                group: Group::BalancedVirtual,
                left_over: vec![usd(1, 0)]
            }
        );
        assert!(infer(&[
            written("e", Some("1000000000000000000 USD")),
            written("a", Some("-1000000000000000000 USD")),
            written("e", Some("0.000000000000000001 USD")),
            written("a", Some("-0.000000000000000001 USD"))
        ])
        .is_ok());
    }

    #[test]
    fn a_cost_counts_in_its_own_commodity() {
        // E18 of the scope page: 10 EUR @ 1.1 USD infers -11 USD.
        let mut fx = written("a", Some("10 EUR"));
        fx.posting.cost = Some(Cost {
            kind: CostKind::Unit,
            amount: amount("1.1 USD"),
        });
        let got = infer(&[fx.clone(), written("b", None)]).unwrap();
        assert_eq!(
            got[1][0].amount,
            Amount::new("USD", Money::from_decimal(-11, 0))
        );
        fx.posting.cost = Some(Cost {
            kind: CostKind::Total,
            amount: amount("11 USD"),
        });
        let got = infer(&[fx, written("b", None)]).unwrap();
        assert_eq!(
            got[1][0].amount,
            Amount::new("USD", Money::from_decimal(-11, 0))
        );
    }
}
