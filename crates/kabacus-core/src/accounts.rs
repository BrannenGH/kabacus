// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Account declarations and the account picker.
//!
//! Friendly names come from tags on `account` directives:
//! `ynab-account:` for YNAB accounts and `ynab-envelope:` for YNAB
//! categories, e.g. `account expenses:food:groceries  ; ynab-envelope:Groceries`.
//! An account's kind (fund, card reserve, ...) comes from its path, never a
//! tag.

use crate::AccountName;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountDeclaration {
    pub name: AccountName,
    pub tags: Vec<(String, String)>,
}

impl AccountDeclaration {
    /// An explicit physical/synthetic override, stored with the declaration.
    pub fn synthetic_override(&self) -> Result<Option<bool>, String> {
        let mut values = self.tags.iter().filter(|(key, _)| key == "synthetic");
        let value = values.next().map(|(_, value)| value.as_str());
        if values.next().is_some() {
            return Err("synthetic may be specified only once".to_string());
        }
        match value {
            None => Ok(None),
            Some("true") => Ok(Some(true)),
            Some("false") => Ok(Some(false)),
            Some(_) => Err("synthetic must be true or false".to_string()),
        }
    }

    /// The YNAB account or YNAB category name, if the declaration has one.
    pub fn friendly_name(&self) -> Option<&str> {
        ["ynab-account", "ynab-envelope"].iter().find_map(|wanted| {
            self.tags
                .iter()
                .find(|(name, value)| name == wanted && !value.is_empty())
                .map(|(_, value)| value.as_str())
        })
    }
}

/// Comment tags: each `name:value`, the value running to the next
/// comma or the end of the comment.
pub fn parse_comment_tags(comment: &str) -> Vec<(String, String)> {
    comment
        .split(',')
        .filter_map(|part| {
            let (before, value) = part.split_once(':')?;
            let name = before.split_whitespace().last()?;
            Some((name.to_string(), value.trim().to_string()))
        })
        .collect()
}

/// Every `account` directive in `text`, with the tags from its same-line
/// comment and any indented comment lines under it. The account name ends
/// at two spaces, a tab, or the line's end.
pub fn parse_account_declarations(text: &str) -> Vec<AccountDeclaration> {
    let mut declarations: Vec<AccountDeclaration> = Vec::new();
    let mut in_directive = false;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("account ") {
            let rest = rest.trim_start();
            let name_end = [rest.find("  "), rest.find('\t')]
                .into_iter()
                .flatten()
                .min()
                .unwrap_or(rest.len());
            let name = rest[..name_end].trim_end();
            let comment = rest[name_end..].trim_start();
            let tags = comment
                .strip_prefix(';')
                .map(parse_comment_tags)
                .unwrap_or_default();
            if !name.is_empty() {
                declarations.push(AccountDeclaration {
                    name: AccountName::new(name),
                    tags,
                });
                in_directive = true;
            }
            continue;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        match (in_directive, indented, line.trim_start().strip_prefix(';')) {
            (true, true, Some(comment)) => {
                if let Some(last) = declarations.last_mut() {
                    last.tags.extend(parse_comment_tags(comment));
                }
            }
            (true, true, None) if line.trim().is_empty() => in_directive = false,
            (_, true, None) => {}
            _ => in_directive = false,
        }
    }
    declarations
}

/// The `account` directive line declaring `name`, with `tags` in
/// its comment, or why it can't be declared. The name must be a valid
/// An account name: non-empty colon-separated parts, no double
/// spaces, tabs or `;` (which would end the name or start a comment).
pub fn render_account_declaration(name: &str, tags: &[(String, String)]) -> Result<String, String> {
    if name.trim().is_empty() {
        return Err("the account name is empty".to_string());
    }
    if name != name.trim() || name.contains("  ") || name.contains(['\t', ';', '\n', '\r']) {
        return Err(format!(
            "{name:?} isn't a valid account name: no leading or trailing spaces, double spaces, tabs, `;` or line breaks"
        ));
    }
    if name.split(':').any(|part| part.trim().is_empty()) {
        return Err(format!("{name:?} has an empty part between colons"));
    }
    let mut line = format!("account {name}");
    if !tags.is_empty() {
        let mut rendered = Vec::new();
        for (tag, value) in tags {
            let bad_name =
                tag.is_empty() || tag.contains(|c: char| c.is_whitespace() || ":,;".contains(c));
            if bad_name || value.contains([',', '\n', '\r']) {
                return Err(format!("{tag}:{value} isn't a valid tag"));
            }
            rendered.push(format!("{tag}:{value}"));
        }
        line.push_str("  ; ");
        line.push_str(&rendered.join(", "));
    }
    Ok(line)
}

/// How well `query` matches `candidate`: `None` if its characters don't
/// all appear in order. Higher is better. Rewards matches at word starts
/// (after `:`, a space, `-` or `_`) and runs of consecutive characters, so
/// `exgr` finds `expenses:groceries` and `gro` prefers `Groceries` over
/// `Gym Program`.
fn subsequence_score(query: &str, candidate: &str) -> Option<i32> {
    let candidate: Vec<char> = candidate.to_lowercase().chars().collect();
    let mut score = 0;
    let mut position = 0;
    let mut previous_match: Option<usize> = None;
    for wanted in query.to_lowercase().chars() {
        let found = (position..candidate.len()).find(|&i| candidate[i] == wanted)?;
        score += 1;
        if found == 0 || matches!(candidate[found - 1], ':' | ' ' | '-' | '_') {
            score += 5;
        }
        if previous_match.is_some_and(|p| p + 1 == found) {
            score += 3;
        } else if let Some(p) = previous_match {
            score -= ((found - p - 1) as i32).min(5);
        }
        previous_match = Some(found);
        position = found + 1;
    }
    Some(score)
}

/// Score one whitespace-free query term against an account's path and
/// friendly name, taking the better of the two.
fn term_score(term: &str, name: &str, friendly: Option<&str>) -> Option<i32> {
    let score_one = |candidate: &str| {
        subsequence_score(term, candidate).map(|s| {
            let lower = candidate.to_lowercase();
            let term = term.to_lowercase();
            s + if lower.starts_with(&term) {
                30
            } else if lower.contains(&term) {
                15
            } else {
                0
            }
        })
    };
    [Some(name), friendly]
        .into_iter()
        .flatten()
        .filter_map(score_one)
        .max()
}

/// A candidate for [`rank_accounts`].
#[derive(Debug, Clone, Copy)]
pub struct PickerEntry<'a> {
    pub name: &'a str,
    pub friendly_name: Option<&'a str>,
}

/// The accounts matching `query`, best first. Every whitespace
/// separated term must match the path or the friendly name; an empty query
/// matches everything in path order. Returns indices into `entries`.
pub fn rank_accounts(query: &str, entries: &[PickerEntry<'_>]) -> Vec<usize> {
    let terms: Vec<&str> = query.split_whitespace().collect();
    let mut scored: Vec<(i32, usize)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, entry)| {
            terms
                .iter()
                .map(|term| term_score(term, entry.name, entry.friendly_name))
                .sum::<Option<i32>>()
                .map(|score| (score, i))
        })
        .collect();
    scored.sort_by(|(score_a, a), (score_b, b)| {
        score_b
            .cmp(score_a)
            .then_with(|| entries[*a].name.len().cmp(&entries[*b].name.len()))
            .then_with(|| entries[*a].name.cmp(entries[*b].name))
    });
    scored.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_flag_is_explicit_and_validated() {
        for (tags, expected) in [
            ("", Ok(None)),
            ("synthetic:true", Ok(Some(true))),
            ("synthetic:false", Ok(Some(false))),
        ] {
            let declaration = AccountDeclaration {
                name: AccountName::new("assets:bank:checking"),
                tags: crate::declared_tags(tags),
            };
            assert_eq!(declaration.synthetic_override(), expected);
        }
        for tags in ["synthetic:yes", "synthetic:true, synthetic:false"] {
            let declaration = AccountDeclaration {
                name: AccountName::new("assets:bank:checking"),
                tags: crate::declared_tags(tags),
            };
            assert!(declaration.synthetic_override().is_err());
        }
    }

    const ACCOUNTS: &str = "\
account assets:checking            ; ynab-account:Everyday Checking, type:A
account expenses:food:groceries    ; ynab-envelope:Groceries
account expenses:fitness:gym
    ; ynab-envelope:Gym Program
account equity:opening

2026-01-01 Opening
    assets:checking  100.00 USD
    ; not-a-declaration-tag:x
    equity:opening
";
    #[test]
    fn parses_declarations_and_their_tags() {
        let decls = parse_account_declarations(ACCOUNTS);
        let names: Vec<&str> = decls.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "assets:checking",
                "expenses:food:groceries",
                "expenses:fitness:gym",
                "equity:opening"
            ]
        );
        assert_eq!(decls[0].friendly_name(), Some("Everyday Checking"));
        assert_eq!(decls[1].friendly_name(), Some("Groceries"));
        // A tag on an indented comment line under the directive counts.
        assert_eq!(decls[2].friendly_name(), Some("Gym Program"));
        assert_eq!(decls[3].friendly_name(), None);
        assert!(decls[0].tags.contains(&("type".into(), "A".into())));
        // Comments under a transaction don't attach to the last directive.
        assert!(!decls[3]
            .tags
            .iter()
            .any(|(k, _)| k == "not-a-declaration-tag"));
    }

    /// The obsolete `envelope` tag no longer names (or makes) anything.
    #[test]
    fn an_envelope_tag_is_just_a_tag() {
        let decls =
            parse_account_declarations("account assets:checking:groceries  ; envelope:Groceries\n");
        assert_eq!(decls[0].friendly_name(), None);
    }
    #[test]
    fn account_names_may_contain_single_spaces() {
        let decls =
            parse_account_declarations("account assets:joint savings\t; ynab-account:Joint\n");
        assert_eq!(decls[0].name.as_str(), "assets:joint savings");
        assert_eq!(decls[0].friendly_name(), Some("Joint"));
    }

    fn picker() -> Vec<(String, Option<String>)> {
        parse_account_declarations(ACCOUNTS)
            .into_iter()
            .map(|d| {
                (
                    d.name.as_str().to_string(),
                    d.friendly_name().map(String::from),
                )
            })
            .collect()
    }

    fn ranked(query: &str) -> Vec<String> {
        let owned = picker();
        let entries: Vec<PickerEntry> = owned
            .iter()
            .map(|(n, f)| PickerEntry {
                name: n,
                friendly_name: f.as_deref(),
            })
            .collect();
        rank_accounts(query, &entries)
            .into_iter()
            .map(|i| owned[i].0.clone())
            .collect()
    }
    #[test]
    fn fuzzy_matches_paths_and_friendly_names() {
        // Abbreviated path.
        assert_eq!(ranked("exgro")[0], "expenses:food:groceries");
        // Friendly name only: "everyday" isn't in the path.
        assert_eq!(ranked("everyday"), ["assets:checking"]);
        // A prefix of a friendly name beats a scattered match.
        assert_eq!(ranked("gro")[0], "expenses:food:groceries");
        // Every term must match.
        assert_eq!(ranked("gym prog"), ["expenses:fitness:gym"]);
        assert!(ranked("zzz").is_empty());
        // Case doesn't matter.
        assert_eq!(ranked("CHECK")[0], "assets:checking");
    }
    #[test]
    fn renders_a_declaration_and_rejects_bad_names() {
        assert_eq!(
            render_account_declaration("expenses:pets", &[("ynab-envelope".into(), "Pets".into())]),
            Ok("account expenses:pets  ; ynab-envelope:Pets".to_string())
        );
        assert_eq!(
            render_account_declaration("assets:joint savings", &[]),
            Ok("account assets:joint savings".to_string())
        );
        for bad in [
            "",
            "expenses::x",
            ":x",
            "x:",
            "a  b",
            "a\tb",
            "a ; b",
            " a",
            "a\nb",
        ] {
            assert!(render_account_declaration(bad, &[]).is_err(), "{bad:?}");
        }
        assert!(render_account_declaration("x", &[("a b".into(), "v".into())]).is_err());
        assert!(render_account_declaration("x", &[("t".into(), "v, w".into())]).is_err());
        // What we render parses back to the same thing.
        let line =
            render_account_declaration("expenses:pets", &[("ynab-envelope".into(), "Pets".into())])
                .unwrap();
        let parsed = parse_account_declarations(&line);
        assert_eq!(parsed[0].name.as_str(), "expenses:pets");
        assert_eq!(parsed[0].friendly_name(), Some("Pets"));
    }
    #[test]
    fn empty_query_lists_everything() {
        assert_eq!(ranked("").len(), 4);
    }
}
