// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Account kinds: every account has a kind, derived from its
//! path by an ordered rule table kept in the server's config, never stored
//! in the journals. The first matching rule wins; `*` matches exactly one
//! segment (or, inside a segment, any run of characters, as in `card-*`)
//! and `**` matches one or more segments.

use crate::AccountName;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccountKind {
    /// Free money in a bank account or cash.
    Money,
    /// A fund: money in its home set aside for a future expense.
    PreAllocation,
    /// A card reserve: money in its home that pays a card's balance.
    DebtReserve,
    /// Money someone will repay.
    Receivable,
    NonLiquid,
    /// A credit card, covered by its debt reserve.
    RevolvingDebt,
    TermLiability,
    Payable,
    Income,
    Expense,
    /// No postings of its own; only a parent of other accounts.
    Parent,
    /// No rule matches it (a warning, not an error).
    Unclassified,
}

impl AccountKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AccountKind::Money => "money",
            AccountKind::PreAllocation => "pre-allocation",
            AccountKind::DebtReserve => "debt-reserve",
            AccountKind::Receivable => "receivable",
            AccountKind::NonLiquid => "non-liquid",
            AccountKind::RevolvingDebt => "revolving-debt",
            AccountKind::TermLiability => "term-liability",
            AccountKind::Payable => "payable",
            AccountKind::Income => "income",
            AccountKind::Expense => "expense",
            AccountKind::Parent => "parent",
            AccountKind::Unclassified => "unclassified",
        }
    }

    /// The words added to a display name that would otherwise
    /// be shared with another account's.
    pub fn qualifier(self) -> &'static str {
        match self {
            AccountKind::Money => "account",
            AccountKind::PreAllocation => "fund",
            AccountKind::DebtReserve => "card reserve",
            AccountKind::Receivable => "receivable",
            AccountKind::NonLiquid => "non-liquid",
            AccountKind::RevolvingDebt => "card",
            AccountKind::TermLiability => "loan",
            AccountKind::Payable => "payable",
            AccountKind::Income => "income",
            AccountKind::Expense => "expense",
            AccountKind::Parent => "group",
            AccountKind::Unclassified => "unclassified",
        }
    }

    /// A fund or a card reserve: money set aside inside a home account.
    pub fn is_envelope(self) -> bool {
        matches!(self, AccountKind::PreAllocation | AccountKind::DebtReserve)
    }
}

impl std::fmt::Display for AccountKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One row of the rule table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KindRule {
    pub pattern: String,
    pub kind: AccountKind,
}

impl KindRule {
    pub fn new(pattern: &str, kind: AccountKind) -> Self {
        KindRule {
            pattern: pattern.to_string(),
            kind,
        }
    }

    pub fn matches(&self, account: &str) -> bool {
        let pattern: Vec<&str> = self.pattern.split(':').collect();
        let account: Vec<&str> = account.split(':').collect();
        match_segments(&pattern, &account)
    }
}

fn match_segments(pattern: &[&str], account: &[&str]) -> bool {
    match pattern.split_first() {
        None => account.is_empty(),
        Some((&"**", rest)) => (1..=account.len()).any(|n| match_segments(rest, &account[n..])),
        Some((segment, rest)) => {
            !account.is_empty()
                && glob_segment(segment, account[0])
                && match_segments(rest, &account[1..])
        }
    }
}

/// `*` inside one segment matches any run of characters.
fn glob_segment(pattern: &str, text: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == text,
        Some((prefix, rest)) => {
            let Some(text) = text.strip_prefix(prefix) else {
                return false;
            };
            (0..=text.len())
                .filter(|&i| text.is_char_boundary(i))
                .any(|i| glob_segment(rest, &text[i..]))
        }
    }
}

/// The rule table used when the config doesn't set one.
pub fn default_kind_rules() -> Vec<KindRule> {
    use AccountKind::*;
    [
        ("assets:bank:*:card-*", DebtReserve),
        ("assets:bank:*:*", PreAllocation),
        ("assets:bank:*", Money),
        ("assets:cash", Money),
        ("assets:receivable:*", Receivable),
        ("assets:investments:*", NonLiquid),
        ("liabilities:credit-card:*", RevolvingDebt),
        ("liabilities:loans:*", TermLiability),
        ("liabilities:mortgage:*", TermLiability),
        ("liabilities:payable:*", Payable),
        ("income:**", Income),
        ("expenses:**", Expense),
        ("equity:**", NonLiquid),
    ]
    .into_iter()
    .map(|(pattern, kind)| KindRule::new(pattern, kind))
    .collect()
}

/// Why a rule table can't be used.
pub fn check_kind_rules(rules: &[KindRule]) -> Result<(), String> {
    for (i, rule) in rules.iter().enumerate() {
        let n = i + 1;
        if rule.pattern.split(':').any(|s| s.trim().is_empty()) {
            return Err(format!(
                "kind rule {n} ({:?}) has an empty segment",
                rule.pattern
            ));
        }
        if rule
            .pattern
            .split(':')
            .any(|s| s.contains("**") && s != "**")
        {
            return Err(format!(
                "kind rule {n} ({:?}): `**` must be a whole segment",
                rule.pattern
            ));
        }
        if matches!(rule.kind, AccountKind::Parent | AccountKind::Unclassified) {
            return Err(format!(
                "kind rule {n} ({:?}): {} is derived, not assigned by a rule",
                rule.pattern, rule.kind
            ));
        }
    }
    Ok(())
}

/// The first rule `account` matches, by index.
pub fn matching_rule(rules: &[KindRule], account: &str) -> Option<usize> {
    rules.iter().position(|r| r.matches(account))
}

/// `accounts` and every ancestor of each (`assets:bank:checking` brings
/// `assets:bank` and `assets`).
pub fn with_ancestors<'a>(
    accounts: impl IntoIterator<Item = &'a AccountName>,
) -> BTreeSet<AccountName> {
    let mut all = BTreeSet::new();
    for account in accounts {
        let mut path = account.as_str();
        all.insert(AccountName::new(path));
        while let Some((parent, _)) = path.rsplit_once(':') {
            all.insert(AccountName::new(parent));
            path = parent;
        }
    }
    all
}

/// An account with its kind, the rule that gave it, and, for a
/// fund or card reserve, its home: the money account holding its cash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedAccount {
    pub account: AccountName,
    pub kind: AccountKind,
    pub synthetic: bool,
    /// Index into the rule table; `None` for `parent` and `unclassified`.
    pub rule: Option<usize>,
    pub home: Option<AccountName>,
    /// The readable name, with the kind added when two accounts would
    /// otherwise read the same (`Car (fund)`, `Car (loan)`).
    pub display_name: String,
}

/// Classify every account in `accounts` (which should already
/// hold every ancestor, see [`with_ancestors`]). `posted` are the accounts
/// with postings of their own. An account with none that only parents
/// others is `parent`, unless a rule makes it a money account: a home with
/// nothing but funds in it is still where their cash is.
pub fn classify_accounts(
    rules: &[KindRule],
    accounts: &BTreeSet<AccountName>,
    posted: &HashSet<AccountName>,
    friendly_names: &HashMap<String, String>,
) -> Vec<ClassifiedAccount> {
    let mut classified: Vec<ClassifiedAccount> = accounts
        .iter()
        .map(|account| {
            let rule = matching_rule(rules, account.as_str());
            let has_children = accounts
                .range(AccountName::new(format!("{account}:"))..)
                .next()
                .is_some_and(|next| next.as_str().starts_with(&format!("{account}:")));
            let only_a_parent = has_children && !posted.contains(account);
            let (kind, rule) = match rule.map(|i| (rules[i].kind, i)) {
                Some((AccountKind::Money, i)) => (AccountKind::Money, Some(i)),
                _ if only_a_parent => (AccountKind::Parent, None),
                Some((kind, i)) => (kind, Some(i)),
                None => (AccountKind::Unclassified, None),
            };
            let home = kind
                .is_envelope()
                .then(|| {
                    account
                        .as_str()
                        .rsplit_once(':')
                        .map(|(h, _)| AccountName::new(h))
                })
                .flatten();
            ClassifiedAccount {
                account: account.clone(),
                kind,
                synthetic: kind.is_envelope(),
                rule,
                home,
                display_name: String::new(),
            }
        })
        .collect();
    let names = display_names(
        &classified
            .iter()
            .map(|c| {
                (
                    &c.account,
                    c.kind,
                    friendly_names.get(c.account.as_str()).map(String::as_str),
                )
            })
            .collect::<Vec<_>>(),
    );
    for (c, name) in classified.iter_mut().zip(names) {
        c.display_name = name;
    }
    classified
}

/// `home-mortgage` -> `Home Mortgage`. The app names people the
/// same way, so People and Accounts agree.
pub fn humanize(leaf: &str) -> String {
    leaf.split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}

/// Each account's display name: its friendly name, or its leaf
/// made readable, with its kind's qualifier added (`Car (fund)`, `Car
/// (loan)`) wherever two accounts would otherwise share a name. Names that
/// still clash after that are left to [`duplicate_display_names`].
pub fn display_names(accounts: &[(&AccountName, AccountKind, Option<&str>)]) -> Vec<String> {
    let base: Vec<String> = accounts
        .iter()
        .map(|(account, _, friendly)| match friendly.map(str::trim) {
            Some(name) if !name.is_empty() => name.to_string(),
            _ => humanize(account.leaf()),
        })
        .collect();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for name in &base {
        *counts.entry(name.to_lowercase()).or_default() += 1;
    }
    base.into_iter()
        .zip(accounts)
        .map(|(name, (_, kind, _))| {
            if counts[&name.to_lowercase()] > 1 {
                format!("{name} ({})", kind.qualifier())
            } else {
                name
            }
        })
        .collect()
}

/// Groups of accounts that share a display name, each sorted.
pub fn duplicate_display_names(accounts: &[ClassifiedAccount]) -> Vec<(String, Vec<AccountName>)> {
    let mut by_name: std::collections::BTreeMap<String, (String, Vec<AccountName>)> =
        std::collections::BTreeMap::new();
    for c in accounts {
        by_name
            .entry(c.display_name.to_lowercase())
            .or_insert_with(|| (c.display_name.clone(), Vec::new()))
            .1
            .push(c.account.clone());
    }
    by_name
        .into_values()
        .filter(|(_, accounts)| accounts.len() > 1)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(account: &str) -> Option<AccountKind> {
        let rules = default_kind_rules();
        matching_rule(&rules, account).map(|i| rules[i].kind)
    }
    #[test]
    fn the_default_rules_classify_the_example_accounts() {
        use AccountKind::*;
        for (account, expected) in [
            ("assets:bank:checking:card-rewards-visa", Some(DebtReserve)),
            ("assets:bank:checking:insurance", Some(PreAllocation)),
            ("assets:bank:savings:emergency-fund", Some(PreAllocation)),
            ("assets:bank:checking", Some(Money)),
            ("assets:bank:union-savings", Some(Money)),
            ("assets:cash", Some(Money)),
            ("assets:receivable:hsa", Some(Receivable)),
            ("assets:investments:brokerage", Some(NonLiquid)),
            ("liabilities:credit-card:spare-card", Some(RevolvingDebt)),
            ("liabilities:loans:car", Some(TermLiability)),
            ("liabilities:mortgage:home", Some(TermLiability)),
            ("liabilities:payable:jordan", Some(Payable)),
            ("income:salary", Some(Income)),
            ("expenses:bills:insurance", Some(Expense)),
            ("equity:opening", Some(NonLiquid)),
            // `**` needs at least one segment; `*` exactly one.
            ("income", None),
            ("assets:receivable", None),
            ("assets:bank:checking:insurance:extra", None),
            ("assets:other", None),
        ] {
            assert_eq!(kind(account), expected, "{account}");
        }
    }
    #[test]
    fn patterns_glob_within_a_segment_and_across_segments() {
        let rule = KindRule::new("a:**:z", AccountKind::Expense);
        assert!(rule.matches("a:b:z"));
        assert!(rule.matches("a:b:c:z"));
        assert!(!rule.matches("a:z"));
        let rule = KindRule::new("a:card-*", AccountKind::DebtReserve);
        assert!(rule.matches("a:card-x"));
        assert!(!rule.matches("a:cardx"));
        assert!(!rule.matches("a:card-x:y"));
        let rule = KindRule::new("a:*-fund", AccountKind::PreAllocation);
        assert!(rule.matches("a:emergency-fund"));
        assert!(!rule.matches("a:emergency"));
    }
    #[test]
    fn bad_rule_tables_are_refused() {
        assert!(check_kind_rules(&default_kind_rules()).is_ok());
        for bad in [
            KindRule::new("assets::x", AccountKind::Money),
            KindRule::new("assets:x**", AccountKind::Money),
            KindRule::new("assets:*", AccountKind::Parent),
            KindRule::new("assets:*", AccountKind::Unclassified),
        ] {
            assert!(
                check_kind_rules(std::slice::from_ref(&bad)).is_err(),
                "{bad:?}"
            );
        }
    }

    fn classify(accounts: &[&str], posted: &[&str]) -> Vec<ClassifiedAccount> {
        let names: Vec<AccountName> = accounts.iter().map(|a| AccountName::new(*a)).collect();
        let posted: HashSet<AccountName> = posted.iter().map(|a| AccountName::new(*a)).collect();
        classify_accounts(
            &default_kind_rules(),
            &with_ancestors(&names),
            &posted,
            &HashMap::new(),
        )
    }
    #[test]
    fn parents_homes_and_unclassified_accounts() {
        let all = classify(
            &[
                "assets:bank:checking",
                "assets:bank:checking:insurance",
                "assets:bank:checking:card-spare-card",
                "assets:bank:savings:emergency-fund",
                "assets:other",
                "expenses:bills:insurance",
            ],
            &[
                "assets:bank:checking",
                "assets:bank:checking:insurance",
                "assets:bank:savings:emergency-fund",
                "expenses:bills:insurance",
            ],
        );
        let get = |a: &str| all.iter().find(|c| c.account.as_str() == a).unwrap();
        assert_eq!(get("assets").kind, AccountKind::Parent);
        assert_eq!(get("assets:bank").kind, AccountKind::Parent);
        assert_eq!(get("expenses:bills").kind, AccountKind::Parent);
        assert_eq!(get("expenses").kind, AccountKind::Parent);
        assert_eq!(get("assets:bank").rule, None);
        // A home with no postings of its own is still a money account.
        assert_eq!(get("assets:bank:savings").kind, AccountKind::Money);
        assert_eq!(get("assets:bank:savings").rule, Some(2));
        assert_eq!(get("assets:other").kind, AccountKind::Unclassified);
        let fund = get("assets:bank:checking:insurance");
        assert_eq!(fund.kind, AccountKind::PreAllocation);
        assert_eq!(fund.home, Some(AccountName::new("assets:bank:checking")));
        let reserve = get("assets:bank:checking:card-spare-card");
        assert_eq!(reserve.kind, AccountKind::DebtReserve);
        assert_eq!(reserve.rule, Some(0));
        assert_eq!(reserve.home, Some(AccountName::new("assets:bank:checking")));
        assert_eq!(get("assets:bank:checking").home, None);
    }
    #[test]
    fn display_names_are_qualified_by_kind_only_where_they_clash() {
        let car_fund = AccountName::new("assets:bank:checking:car");
        let car_loan = AccountName::new("liabilities:loans:car");
        let mortgage_fund = AccountName::new("assets:bank:checking:home-mortgage");
        let mortgage = AccountName::new("liabilities:mortgage:home-mortgage");
        let ar = AccountName::new("assets:receivable:jordan");
        let ap = AccountName::new("liabilities:payable:jordan");
        let checking = AccountName::new("assets:bank:checking");
        let names = display_names(&[
            (&car_fund, AccountKind::PreAllocation, None),
            (&car_loan, AccountKind::TermLiability, None),
            (&mortgage_fund, AccountKind::PreAllocation, None),
            (&mortgage, AccountKind::TermLiability, Some("Home Mortgage")),
            (&ar, AccountKind::Receivable, Some("AR: Jordan")),
            (&ap, AccountKind::Payable, Some("AR: Jordan")),
            (&checking, AccountKind::Money, Some("  Everyday Checking ")),
        ]);
        assert_eq!(
            names,
            [
                "Car (fund)",
                "Car (loan)",
                "Home Mortgage (fund)",
                "Home Mortgage (loan)",
                "AR: Jordan (receivable)",
                "AR: Jordan (payable)",
                "Everyday Checking",
            ]
        );
    }
    #[test]
    fn names_that_still_clash_are_reported() {
        let all = classify(
            &[
                "expenses:household:misc",
                "expenses:social:misc",
                "expenses:pets",
            ],
            &[
                "expenses:household:misc",
                "expenses:social:misc",
                "expenses:pets",
            ],
        );
        let dupes = duplicate_display_names(&all);
        assert_eq!(dupes.len(), 1);
        assert_eq!(dupes[0].0, "Misc (expense)");
        assert_eq!(dupes[0].1.len(), 2);
    }
}
