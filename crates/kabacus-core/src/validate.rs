// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! The client-side validation rules, run live as the user types so
//! the editor can show errors before the round trip to the server. The
//! shared ledger engine always has the final say
//! — these rules exist to give fast, precise feedback, not
//! to replace it.

use crate::domain::{
    AccountName, BalanceAssertion, Posting, PostingKind, Status, Tag, Transaction,
};
use crate::money::{Amount, Money};
use chrono::NaiveDate;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq)]
pub enum ValidationError {
    /// More than one posting is missing an amount; at most one can be
    /// inferred.
    AmbiguousBalance {
        missing: usize,
    },
    /// Exactly one posting is missing an amount, but the other postings span
    /// more than one commodity, so which commodity to fill in is ambiguous.
    AmbiguousCommodity,
    /// The postings don't sum to zero in this commodity.
    Unbalanced {
        commodity: String,
        difference: Money,
    },
    UndeclaredAccount {
        account: AccountName,
    },
    BalanceAssertionFailed(Box<BalanceAssertionFailure>),
    /// An edit leaves out the posting that carried this balance assertion.
    /// Assertions are only removed explicitly ([`AssertionEdit::Clear`]).
    DroppedBalanceAssertion {
        account: AccountName,
        expected: Amount,
    },
    /// A virtual `(account)` posting isn't balanced against the others, so
    /// its amount can't be inferred.
    VirtualWithoutAmount {
        account: AccountName,
    },
    EmptyDescription,
    /// A line break or `;`, which would end the journal line it's on.
    DescriptionBreaksLine,
    InvalidDate(String),
    NoPostings,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BalanceAssertionFailure {
    pub account: AccountName,
    pub date: NaiveDate,
    pub expected: Amount,
    pub actual: Amount,
}

impl std::error::Error for ValidationError {}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidationError::AmbiguousBalance { missing } => {
                write!(f, "{missing} postings are missing an amount; at most one can be filled in automatically")
            }
            ValidationError::AmbiguousCommodity => {
                write!(f, "can't infer the missing amount's commodity: the other postings use more than one")
            }
            ValidationError::Unbalanced { commodity, difference } => {
                write!(f, "unbalanced by {difference} {commodity}")
            }
            ValidationError::UndeclaredAccount { account } => write!(f, "account {account} is not declared"),
            ValidationError::BalanceAssertionFailed(b) => write!(
                f,
                "This would change the {} balance on {}, which an assertion says must be {} {}, but it would be {} {}",
                b.account, b.date, b.expected.money, b.expected.commodity, b.actual.money, b.actual.commodity
            ),
            ValidationError::DroppedBalanceAssertion { account, expected } => write!(
                f,
                "the posting on {account} asserts its balance is {} {}; keep a posting on {account}, or clear that assertion in an edit of its own first",
                expected.money, expected.commodity
            ),
            ValidationError::VirtualWithoutAmount { account } => write!(
                f,
                "the virtual posting on {account} needs an amount: it isn't balanced against the others"
            ),
            ValidationError::EmptyDescription => write!(f, "the description can't be empty"),
            ValidationError::DescriptionBreaksLine => {
                write!(f, "the description can't hold a line break or a semicolon (;)")
            }
            ValidationError::InvalidDate(s) => write!(f, "'{s}' isn't a valid date"),
            ValidationError::NoPostings => write!(f, "a transaction needs at least one posting"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DraftPosting {
    pub account: AccountName,
    /// `None` means "leave this one for the server/editor to fill in" — at
    /// most one posting in a transaction may be `None`.
    pub amount: Option<Amount>,
    pub status: Status,
    pub balance_assertion: AssertionEdit,
}

/// What a draft posting does to its balance assertion. Editing a
/// transaction keeps each posting's assertion unless the caller says
/// otherwise; a new posting has none to keep.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum AssertionEdit {
    #[default]
    Keep,
    /// Assert this balance. An existing `==`/`=*` form is kept.
    Set(Amount),
    Clear,
}

/// What a draft posting takes over from the original posting it replaces
/// in an edit: everything the editor doesn't show.
#[derive(Debug, Clone, PartialEq)]
pub struct CarriedOver {
    pub kind: PostingKind,
    pub comment: String,
    pub tags: Vec<Tag>,
    pub balance_assertion: Option<BalanceAssertion>,
}

/// What each draft posting keeps from `original`'s postings in an edit
/// (pass `&[]` for a new transaction): its comment, tags and real/virtual
/// kind, and its balance assertion as the draft's [`AssertionEdit`] says.
/// A draft posting takes over the first not-yet-taken original posting on
/// the same account, so reordering postings or changing other accounts
/// keeps them; a posting on a new account is a plain real posting. Leaving
/// out a posting that carries an assertion is an error rather than a
/// silent removal.
pub fn carry_over(
    original: &[Posting],
    drafts: &[DraftPosting],
) -> Result<Vec<CarriedOver>, ValidationError> {
    carry_over_with(original, drafts, |_| false)
}

/// [`carry_over`], where a balance assertion a draft adds (rather than
/// changes) is inclusive (`=*`) on the accounts `inclusive` picks: money
/// accounts, whose funds and reserves must not affect reconciliation
/// against the bank.
pub fn carry_over_with(
    original: &[Posting],
    drafts: &[DraftPosting],
    inclusive: impl Fn(&AccountName) -> bool,
) -> Result<Vec<CarriedOver>, ValidationError> {
    let mut taken = vec![false; original.len()];
    let carried = drafts
        .iter()
        .map(|draft| {
            let matched = original
                .iter()
                .enumerate()
                .find(|(i, p)| !taken[*i] && p.account == draft.account)
                .map(|(i, p)| {
                    taken[i] = true;
                    p
                });
            let existing = matched.and_then(|p| p.balance_assertion.clone());
            let balance_assertion = match &draft.balance_assertion {
                AssertionEdit::Keep => existing,
                AssertionEdit::Clear => None,
                AssertionEdit::Set(amount) => Some(match existing {
                    Some(old) => BalanceAssertion {
                        amount: amount.clone(),
                        ..old
                    },
                    None => BalanceAssertion {
                        inclusive: inclusive(&draft.account),
                        ..BalanceAssertion::new(amount.clone())
                    },
                }),
            };
            match matched {
                Some(p) => CarriedOver {
                    kind: p.kind,
                    comment: p.comment.clone(),
                    tags: p.tags.clone(),
                    balance_assertion,
                },
                None => CarriedOver {
                    kind: PostingKind::Real,
                    comment: String::new(),
                    tags: vec![],
                    balance_assertion,
                },
            }
        })
        .collect();
    if let Some((dropped, _)) = original
        .iter()
        .zip(&taken)
        .find(|(p, taken)| !**taken && p.balance_assertion.is_some())
    {
        return Err(ValidationError::DroppedBalanceAssertion {
            account: dropped.account.clone(),
            expected: dropped.balance_assertion.clone().unwrap().amount,
        });
    }
    Ok(carried)
}

/// [`resolve_amounts`] for postings of the given kinds. Real postings
/// balance among themselves, balanced virtual
/// (`[account]`) postings among themselves, and virtual (`(account)`)
/// postings not at all, so each of those needs its amount.
pub fn resolve_amounts_by_kind(
    postings: &[DraftPosting],
    kinds: &[PostingKind],
) -> Result<Vec<Amount>, ValidationError> {
    if postings.is_empty() {
        return Err(ValidationError::NoPostings);
    }
    let mut resolved: Vec<Option<Amount>> = vec![None; postings.len()];
    for (i, p) in postings.iter().enumerate() {
        if kinds[i] == PostingKind::Virtual {
            let amount = p
                .amount
                .clone()
                .ok_or(ValidationError::VirtualWithoutAmount {
                    account: p.account.clone(),
                })?;
            resolved[i] = Some(amount);
        }
    }
    for kind in [PostingKind::Real, PostingKind::BalancedVirtual] {
        let group: Vec<usize> = (0..postings.len()).filter(|&i| kinds[i] == kind).collect();
        if group.is_empty() {
            continue;
        }
        let members: Vec<&DraftPosting> = group.iter().map(|&i| &postings[i]).collect();
        for (i, amount) in group.into_iter().zip(balance(&members)?) {
            resolved[i] = Some(amount);
        }
    }
    Ok(resolved.into_iter().flatten().collect())
}

/// Resolve at most one missing amount and confirm the postings balance.
/// Returns the fully-resolved amounts in the same order as `postings`.
pub fn resolve_amounts(postings: &[DraftPosting]) -> Result<Vec<Amount>, ValidationError> {
    resolve_amounts_by_kind(postings, &vec![PostingKind::Real; postings.len()])
}

/// One balancing group's amounts: at most one missing amount is filled in,
/// and the group must sum to zero in each commodity.
fn balance(postings: &[&DraftPosting]) -> Result<Vec<Amount>, ValidationError> {
    let missing: Vec<usize> = postings
        .iter()
        .enumerate()
        .filter(|(_, p)| p.amount.is_none())
        .map(|(i, _)| i)
        .collect();
    if missing.len() > 1 {
        return Err(ValidationError::AmbiguousBalance {
            missing: missing.len(),
        });
    }

    let mut sums: HashMap<String, Money> = HashMap::new();
    for p in postings {
        if let Some(a) = &p.amount {
            let entry = sums.entry(a.commodity.clone()).or_insert_with(Money::zero);
            *entry = entry.clone() + a.money.clone();
        }
    }

    let mut resolved: Vec<Amount> = Vec::with_capacity(postings.len());

    if let Some(blank_index) = missing.first().copied() {
        if sums.len() > 1 {
            return Err(ValidationError::AmbiguousCommodity);
        }
        let (commodity, sum) = sums
            .iter()
            .next()
            .map(|(c, m)| (c.clone(), m.clone()))
            .ok_or(ValidationError::AmbiguousCommodity)?;
        let fill = Amount::new(commodity, -sum);
        for (i, p) in postings.iter().enumerate() {
            resolved.push(if i == blank_index {
                fill.clone()
            } else {
                p.amount.clone().unwrap()
            });
        }
    } else {
        for (commodity, sum) in &sums {
            if !sum.is_zero() {
                return Err(ValidationError::Unbalanced {
                    commodity: commodity.clone(),
                    difference: sum.clone(),
                });
            }
        }
        for p in postings {
            resolved.push(p.amount.clone().unwrap());
        }
    }

    Ok(resolved)
}

/// Every account used must be declared.
pub fn check_accounts_declared<'a>(
    accounts: impl IntoIterator<Item = &'a AccountName>,
    declared: &HashSet<AccountName>,
) -> Result<(), Vec<ValidationError>> {
    let errors: Vec<ValidationError> = accounts
        .into_iter()
        .filter(|a| !declared.contains(*a))
        .map(|a| ValidationError::UndeclaredAccount { account: a.clone() })
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn check_description(description: &str) -> Result<(), ValidationError> {
    if description.trim().is_empty() {
        Err(ValidationError::EmptyDescription)
    } else if description.contains(['\n', '\r', ';']) {
        Err(ValidationError::DescriptionBreaksLine)
    } else {
        Ok(())
    }
}

pub fn parse_date(s: &str) -> Result<NaiveDate, ValidationError> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| ValidationError::InvalidDate(s.to_string()))
}

/// Replay every posting's balance assertion in date order and confirm the
/// running balance matches. `transactions` must already be in the
/// order they are applied (date, then source index).
pub fn check_balance_assertions(transactions: &[Transaction]) -> Vec<ValidationError> {
    let mut running: HashMap<(AccountName, String), Money> = HashMap::new();
    let mut errors = Vec::new();

    let mut ordered: Vec<&Transaction> = transactions.iter().collect();
    ordered.sort_by_key(|t| (t.date, t.id));

    for tx in ordered {
        for posting in &tx.postings {
            let key = (posting.account.clone(), posting.amount.commodity.clone());
            let entry = running.entry(key).or_insert_with(Money::zero);
            *entry = entry.clone() + posting.amount.money.clone();

            if let Some(asserted) = &posting.balance_assertion {
                let asserted = &asserted.amount;
                if *entry != asserted.money {
                    errors.push(ValidationError::BalanceAssertionFailed(Box::new(
                        BalanceAssertionFailure {
                            account: posting.account.clone(),
                            date: tx.date,
                            expected: asserted.clone(),
                            actual: Amount::new(posting.amount.commodity.clone(), entry.clone()),
                        },
                    )));
                }
            }
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usd(mantissa: i128, places: u32) -> Amount {
        Amount::new("USD", Money::from_decimal(mantissa, places))
    }

    fn acct(s: &str) -> AccountName {
        AccountName::new(s)
    }
    #[test]
    fn fills_one_missing_amount_as_negation_of_the_rest() {
        let postings = vec![
            DraftPosting {
                account: acct("expenses:groceries"),
                amount: Some(usd(4568, 2)),
                status: Status::Unmarked,
                balance_assertion: AssertionEdit::Keep,
            },
            DraftPosting {
                account: acct("assets:checking"),
                amount: None,
                status: Status::Unmarked,
                balance_assertion: AssertionEdit::Keep,
            },
        ];
        let resolved = resolve_amounts(&postings).unwrap();
        assert_eq!(resolved[1].money.to_string(), "-45.68");
    }
    #[test]
    fn more_than_one_missing_amount_is_ambiguous() {
        let postings = vec![
            DraftPosting {
                account: acct("a"),
                amount: None,
                status: Status::Unmarked,
                balance_assertion: AssertionEdit::Keep,
            },
            DraftPosting {
                account: acct("b"),
                amount: None,
                status: Status::Unmarked,
                balance_assertion: AssertionEdit::Keep,
            },
        ];
        assert_eq!(
            resolve_amounts(&postings),
            Err(ValidationError::AmbiguousBalance { missing: 2 })
        );
    }
    #[test]
    fn unbalanced_postings_with_all_amounts_given_is_rejected() {
        let postings = vec![
            DraftPosting {
                account: acct("a"),
                amount: Some(usd(100, 2)),
                status: Status::Unmarked,
                balance_assertion: AssertionEdit::Keep,
            },
            DraftPosting {
                account: acct("b"),
                amount: Some(usd(-50, 2)),
                status: Status::Unmarked,
                balance_assertion: AssertionEdit::Keep,
            },
        ];
        match resolve_amounts(&postings) {
            Err(ValidationError::Unbalanced { difference, .. }) => {
                assert_eq!(difference.to_string(), "0.50")
            }
            other => panic!("expected Unbalanced, got {other:?}"),
        }
    }
    #[test]
    fn balanced_postings_with_all_amounts_given_is_accepted() {
        let postings = vec![
            DraftPosting {
                account: acct("a"),
                amount: Some(usd(100, 2)),
                status: Status::Unmarked,
                balance_assertion: AssertionEdit::Keep,
            },
            DraftPosting {
                account: acct("b"),
                amount: Some(usd(-100, 2)),
                status: Status::Unmarked,
                balance_assertion: AssertionEdit::Keep,
            },
        ];
        assert!(resolve_amounts(&postings).is_ok());
    }
    #[test]
    fn undeclared_account_is_rejected() {
        let mut declared = HashSet::new();
        declared.insert(acct("assets:checking"));
        let used = [acct("assets:checking"), acct("expenses:groceries")];
        let result = check_accounts_declared(used.iter(), &declared);
        assert_eq!(
            result,
            Err(vec![ValidationError::UndeclaredAccount {
                account: acct("expenses:groceries")
            }])
        );
    }
    #[test]
    fn empty_description_is_rejected() {
        assert_eq!(
            check_description("   "),
            Err(ValidationError::EmptyDescription)
        );
        assert!(check_description("Grocery Store").is_ok());
    }
    /// A line break would start a new journal line and `;` a comment, so
    /// either would let a description pose as postings or tags.
    #[test]
    fn a_description_stays_on_its_journal_line() {
        for description in [
            "Cub Foods\n    assets:bank:checking  1000000.00 USD",
            "Cub Foods\r\n",
            "Cub Foods ; id:4",
        ] {
            assert_eq!(
                check_description(description),
                Err(ValidationError::DescriptionBreaksLine),
                "{description:?}"
            );
        }
        assert!(check_description("Holiday Stationstores | fuel, 12.4 gal").is_ok());
    }
    #[test]
    fn invalid_date_is_rejected() {
        assert!(parse_date("not-a-date").is_err());
        assert!(parse_date("2026-09-28").is_ok());
    }
    #[test]
    fn balance_assertion_break_is_detected() {
        use crate::domain::{Posting, PostingKind, Transaction};
        let mk = |date: &str, amount: i128, assertion: Option<i128>| Transaction {
            date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            status: Status::Unmarked,
            code: String::new(),
            description: "x".into(),
            comment: String::new(),
            tags: vec![],
            id: 1,
            postings: vec![Posting {
                account: acct("assets:checking"),
                amount: usd(amount, 2),
                status: Status::Unmarked,
                kind: PostingKind::Real,
                comment: String::new(),
                tags: vec![],
                cost: None,
                balance_assertion: assertion.map(|a| BalanceAssertion::new(usd(a, 2))),
            }],
        };
        // Balance goes to 100.00, asserted correctly; then +50 goes to
        // 150.00 but is asserted (wrongly) as 200.00.
        let txs = vec![
            mk("2026-01-01", 10000, Some(10000)),
            mk("2026-01-02", 5000, Some(20000)),
        ];
        let errors = check_balance_assertions(&txs);
        assert_eq!(errors.len(), 1);
        match &errors[0] {
            ValidationError::BalanceAssertionFailed(b) => {
                assert_eq!(b.actual.money.to_string(), "150.00");
                assert_eq!(b.expected.money.to_string(), "200.00");
            }
            other => panic!("expected BalanceAssertionFailed, got {other:?}"),
        }
    }

    fn posting(account: &str, cents: i128, assertion: Option<BalanceAssertion>) -> Posting {
        Posting {
            account: acct(account),
            amount: usd(cents, 2),
            status: Status::Unmarked,
            kind: crate::domain::PostingKind::Real,
            comment: String::new(),
            tags: vec![],
            cost: None,
            balance_assertion: assertion,
        }
    }

    fn draft(account: &str, cents: i128, edit: AssertionEdit) -> DraftPosting {
        DraftPosting {
            account: acct(account),
            amount: Some(usd(cents, 2)),
            status: Status::Unmarked,
            balance_assertion: edit,
        }
    }

    fn assertions(
        original: &[Posting],
        drafts: &[DraftPosting],
    ) -> Result<Vec<Option<BalanceAssertion>>, ValidationError> {
        Ok(carry_over(original, drafts)?
            .into_iter()
            .map(|c| c.balance_assertion)
            .collect())
    }

    /// An edit that only changes the amount on the other posting, with the
    /// postings in a new order, keeps the assertion where it was.
    #[test]
    fn edits_keep_each_postings_balance_assertion() {
        let asserted = BalanceAssertion::new(usd(-34171, 2));
        let original = vec![
            posting("expenses:groceries", 0, None),
            posting("assets:checking", 0, Some(asserted.clone())),
        ];
        let drafts = vec![
            draft("assets:checking", -500, AssertionEdit::Keep),
            draft("expenses:household", 500, AssertionEdit::Keep),
        ];
        assert_eq!(
            assertions(&original, &drafts).unwrap(),
            vec![Some(asserted), None]
        );
    }
    #[test]
    fn an_edit_can_change_or_clear_an_assertion_explicitly() {
        let strict = BalanceAssertion {
            amount: usd(-34171, 2),
            total: true,
            inclusive: true,
        };
        let original = vec![
            posting("assets:checking", 0, Some(strict)),
            posting(
                "assets:savings",
                0,
                Some(BalanceAssertion::new(usd(100, 2))),
            ),
        ];
        let drafts = vec![
            draft("assets:checking", 0, AssertionEdit::Set(usd(-35000, 2))),
            draft("assets:savings", 0, AssertionEdit::Clear),
        ];
        let resolved = assertions(&original, &drafts).unwrap();
        let changed = resolved[0].as_ref().unwrap();
        assert_eq!(changed.amount, usd(-35000, 2));
        assert_eq!(changed.operator(), "==*", "the assertion's form is kept");
        assert_eq!(resolved[1], None);
    }

    /// A new transaction has nothing to keep; `Set` asserts a plain `=`.
    #[test]
    fn new_postings_only_get_the_assertions_they_ask_for() {
        let drafts = vec![
            draft("assets:checking", -500, AssertionEdit::Set(usd(-500, 2))),
            draft("expenses:groceries", 500, AssertionEdit::Keep),
        ];
        assert_eq!(
            assertions(&[], &drafts).unwrap(),
            vec![Some(BalanceAssertion::new(usd(-500, 2))), None]
        );
    }

    /// A new assertion on a money account is inclusive (`=*`); an existing
    /// one keeps its form when its amount changes.
    #[test]
    fn new_assertions_on_money_accounts_are_inclusive() {
        let money = |a: &AccountName| a.as_str() == "assets:bank:checking";
        let drafts = vec![
            draft(
                "assets:bank:checking",
                -500,
                AssertionEdit::Set(usd(-500, 2)),
            ),
            draft("liabilities:card", 500, AssertionEdit::Set(usd(500, 2))),
        ];
        let carried = carry_over_with(&[], &drafts, money).unwrap();
        let operators: Vec<&str> = carried
            .iter()
            .map(|c| c.balance_assertion.as_ref().unwrap().operator())
            .collect();
        assert_eq!(operators, ["=*", "="]);

        let original = vec![posting(
            "assets:bank:checking",
            0,
            Some(BalanceAssertion::new(usd(100, 2))),
        )];
        let drafts = vec![draft(
            "assets:bank:checking",
            0,
            AssertionEdit::Set(usd(200, 2)),
        )];
        let carried = carry_over_with(&original, &drafts, money).unwrap();
        assert_eq!(
            carried[0].balance_assertion.as_ref().unwrap().operator(),
            "="
        );
    }
    #[test]
    fn leaving_out_an_asserted_posting_is_refused_not_silently_dropped() {
        let original = vec![
            posting("expenses:groceries", 500, None),
            posting(
                "assets:checking",
                -500,
                Some(BalanceAssertion::new(usd(-500, 2))),
            ),
        ];
        let drafts = vec![
            draft("expenses:groceries", 500, AssertionEdit::Keep),
            draft("assets:savings", -500, AssertionEdit::Keep),
        ];
        assert_eq!(
            assertions(&original, &drafts),
            Err(ValidationError::DroppedBalanceAssertion {
                account: acct("assets:checking"),
                expected: usd(-500, 2),
            })
        );
    }

    /// An edit keeps each posting's comment, its own tags and whether it is
    /// real, `(virtual)` or `[balanced virtual]`, however the postings are
    /// reordered; a posting on a new account starts plain.
    #[test]
    fn edits_carry_over_each_postings_comment_tags_and_kind() {
        let annotated = |account: &str, cents, kind, comment: &str| Posting {
            kind,
            comment: comment.to_string(),
            tags: crate::comment::declared_tags(comment),
            ..posting(account, cents, None)
        };
        let original = vec![
            annotated(
                "expenses:groceries",
                4500,
                PostingKind::Real,
                "plain words only\n",
            ),
            annotated(
                "assets:checking",
                -4500,
                PostingKind::Real,
                "card:visa\nsecond line\n",
            ),
            annotated(
                "budget:groceries",
                -4500,
                PostingKind::Virtual,
                "envelope note\n",
            ),
            annotated("equity:x", 1, PostingKind::BalancedVirtual, ""),
            annotated("equity:x", -1, PostingKind::BalancedVirtual, "second x\n"),
        ];
        let drafts = vec![
            draft("equity:x", 1, AssertionEdit::Keep),
            draft("assets:checking", -5000, AssertionEdit::Keep),
            draft("budget:groceries", -5000, AssertionEdit::Keep),
            draft("expenses:household", 5000, AssertionEdit::Keep),
            draft("equity:x", -1, AssertionEdit::Keep),
        ];
        let carried = carry_over(&original, &drafts).unwrap();
        let summary: Vec<_> = carried
            .iter()
            .map(|c| (c.kind, c.comment.as_str(), c.tags.clone()))
            .collect();
        let visa = vec![("card".to_string(), "visa".to_string())];
        assert_eq!(
            summary,
            vec![
                (PostingKind::BalancedVirtual, "", vec![]),
                (PostingKind::Real, "card:visa\nsecond line\n", visa),
                (PostingKind::Virtual, "envelope note\n", vec![]),
                (PostingKind::Real, "", vec![]),
                (PostingKind::BalancedVirtual, "second x\n", vec![]),
            ]
        );
    }

    /// Real postings balance among themselves, `[balanced
    /// virtual]` ones among themselves, and `(virtual)` ones not at all.
    #[test]
    fn virtual_postings_balance_apart_from_real_ones() {
        use PostingKind::{BalancedVirtual, Real, Virtual};
        let blank = |account: &str| DraftPosting {
            amount: None,
            ..draft(account, 0, AssertionEdit::Keep)
        };
        let postings = vec![
            draft("expenses:groceries", 4500, AssertionEdit::Keep),
            blank("assets:checking"),
            draft("budget:groceries", -4500, AssertionEdit::Keep),
            draft("budget:a", 100, AssertionEdit::Keep),
            blank("budget:b"),
        ];
        let kinds = [Real, Real, Virtual, BalancedVirtual, BalancedVirtual];
        let resolved: Vec<String> = resolve_amounts_by_kind(&postings, &kinds)
            .unwrap()
            .iter()
            .map(|a| a.money.to_string())
            .collect();
        assert_eq!(resolved, ["45.00", "-45.00", "-45.00", "1.00", "-1.00"]);

        // An unbalanced balanced-virtual group is refused like a real one:
        // here the real postings balance and `[budget:a]`/`[budget:b]`
        // don't.
        let mut all_given = postings.clone();
        all_given[1].amount = Some(usd(-4500, 2));
        all_given[4].amount = Some(usd(-4500, 2));
        match resolve_amounts_by_kind(&all_given, &kinds) {
            Err(ValidationError::Unbalanced { difference, .. }) => {
                assert_eq!(difference.to_string(), "-44.00")
            }
            other => panic!("expected Unbalanced, got {other:?}"),
        }

        // A virtual posting has nothing to be inferred from.
        assert_eq!(
            resolve_amounts_by_kind(
                &[
                    draft("a", 100, AssertionEdit::Keep),
                    blank("budget:groceries")
                ],
                &[Real, Virtual],
            ),
            Err(ValidationError::VirtualWithoutAmount {
                account: acct("budget:groceries")
            })
        );
    }

    proptest::proptest! {
        #[test]
        fn resolved_amounts_always_sum_to_zero_when_one_is_missing(
            a in -1_000_000i64..1_000_000,
            b in -1_000_000i64..1_000_000,
        ) {
            let postings = vec![
                DraftPosting { account: acct("a"), amount: Some(usd(a as i128, 2)), status: Status::Unmarked, balance_assertion: AssertionEdit::Keep },
                DraftPosting { account: acct("b"), amount: Some(usd(b as i128, 2)), status: Status::Unmarked, balance_assertion: AssertionEdit::Keep },
                DraftPosting { account: acct("c"), amount: None, status: Status::Unmarked, balance_assertion: AssertionEdit::Keep },
            ];
            let resolved = resolve_amounts(&postings).unwrap();
            let sum: Money = resolved.iter().map(|amt| amt.money.clone()).sum();
            proptest::prop_assert!(sum.is_zero());
        }
    }
}
