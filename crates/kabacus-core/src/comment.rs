// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Journal comments and the tags they declare. A transaction's or posting's
//! comment is the text after
//! each `;`, one line per comment line, the first being the one on the
//! transaction's or posting's own line (empty when there is none). Tags are
//! declared inside that text, so writing an edited transaction back keeps
//! the comment and only adds or removes the declarations whose tags changed.

use crate::domain::Tag;
use std::ops::Range;

/// One `name:value` declaration in a comment, with the byte range to cut
/// to remove it (its trailing comma included).
struct Declaration {
    tag: Tag,
    span: Range<usize>,
}

/// Tag syntax: a tag name is a
/// run of anything but whitespace and `:`, ended by `:`; its value runs to
/// the next `,` or the end of the line and is trimmed. Other words are
/// skipped, and an empty name before a `:` is no tag.
fn declarations(comment: &str) -> Vec<Declaration> {
    let mut found = Vec::new();
    let mut i = 0;
    while let Some(c) = comment[i..].chars().next() {
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        let start = i;
        i = comment[i..]
            .find(|c: char| c == ':' || c.is_whitespace())
            .map_or(comment.len(), |end| i + end);
        let name = &comment[start..i];
        if !comment[i..].starts_with(':') {
            continue;
        }
        i += 1;
        if name.is_empty() {
            continue;
        }
        let value_end = comment[i..]
            .find([',', '\n'])
            .map_or(comment.len(), |end| i + end);
        let value = comment[i..value_end].trim();
        i = value_end;
        if comment[i..].starts_with(',') {
            i += 1;
        }
        found.push(Declaration {
            tag: (name.to_string(), value.to_string()),
            span: start..i,
        });
    }
    found
}

/// The tags `comment` declares, in order.
pub fn declared_tags(comment: &str) -> Vec<Tag> {
    declarations(comment).into_iter().map(|d| d.tag).collect()
}

/// `comment` with its tag declarations matched to `tags`: a declaration
/// whose tag isn't in `tags` is cut out, the rest of the comment is left as
/// it was, and the tags it doesn't declare yet are returned, in `tags`'
/// order, for the caller to add.
pub fn reconcile_tags(comment: &str, tags: &[Tag]) -> (String, Vec<Tag>) {
    let mut missing: Vec<Option<&Tag>> = tags.iter().map(Some).collect();
    let mut cut = Vec::new();
    for declaration in declarations(comment) {
        match missing
            .iter_mut()
            .find(|slot| slot.is_some_and(|tag| *tag == declaration.tag))
        {
            Some(slot) => *slot = None,
            None => cut.push(declaration.span),
        }
    }
    let missing = missing.into_iter().flatten().cloned().collect();
    if cut.is_empty() {
        return (comment.to_string(), missing);
    }

    // Spans never cross a line, so line numbers survive the cuts.
    let touched: Vec<usize> = cut
        .iter()
        .map(|span| comment[..span.start].matches('\n').count())
        .collect();
    let mut kept = String::new();
    let mut from = 0;
    for span in cut {
        kept.push_str(&comment[from..span.start]);
        from = span.end
            + comment[span.end..]
                .find(|c: char| c != ' ' && c != '\t')
                .unwrap_or(comment.len() - span.end);
    }
    kept.push_str(&comment[from..]);

    // A line that lost a tag loses the separator before it, and goes
    // altogether if nothing is left (except the first, which is the
    // transaction's or posting's own line).
    let lines: Vec<String> = kept
        .split('\n')
        .enumerate()
        .filter_map(|(n, line)| {
            if !touched.contains(&n) {
                return Some(line.to_string());
            }
            let line = line.trim_end_matches([' ', '\t', ',']);
            (n == 0 || !line.is_empty()).then(|| line.to_string())
        })
        .collect();
    (lines.join("\n"), missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(pairs: &[(&str, &str)]) -> Vec<Tag> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn reads_tags_from_comments() {
        let comment = "see http://x.com/a, foo:bar\n\
                       a:1, b:2, c:3\n\
                       tag-novalue:, other: spaced value , x\n\
                       word: then text, more words y:z\n\
                       :lead x:1 [br:k] (pa:v)\n\
                       dup:1, dup:2\n\
                       date:2026-01-02 trailing\n\
                       [br:k]\n\
                       plain words only\n";
        assert_eq!(
            declared_tags(comment),
            tags(&[
                ("http", "//x.com/a"),
                ("foo", "bar"),
                ("a", "1"),
                ("b", "2"),
                ("c", "3"),
                ("tag-novalue", ""),
                ("other", "spaced value"),
                ("word", "then text"),
                ("y", "z"),
                ("x", "1 [br:k] (pa:v)"),
                ("dup", "1"),
                ("dup", "2"),
                ("date", "2026-01-02 trailing"),
                ("[br", "k]"),
            ])
        );
        assert_eq!(
            declared_tags("id:x, reverses:y, void"),
            tags(&[("id", "x"), ("reverses", "y")])
        );
        assert_eq!(declared_tags(""), vec![]);
    }
    #[test]
    fn unchanged_tags_leave_the_comment_exactly_as_it_was() {
        let comment = "trip comment, id:abc-123\nsecond line, receipt:yes\n";
        let same = tags(&[("receipt", "yes"), ("id", "abc-123")]);
        assert_eq!(
            reconcile_tags(comment, &same),
            (comment.to_string(), vec![])
        );
    }
    #[test]
    fn changed_tags_cut_or_add_only_their_declarations() {
        let comment = "trip comment, id:abc-123\na:1, b:2, c:3\nreceipt:yes\n";
        let (kept, missing) = reconcile_tags(
            comment,
            &tags(&[("id", "abc-123"), ("a", "1"), ("c", "3"), ("new", "")]),
        );
        assert_eq!(kept, "trip comment, id:abc-123\na:1, c:3\n");
        assert_eq!(missing, tags(&[("new", "")]));

        // Cutting the last tag on the comment's first line leaves the
        // words before it.
        let (kept, _) = reconcile_tags("trip comment, id:abc-123\n", &[]);
        assert_eq!(kept, "trip comment\n");

        // A duplicate is kept once per occurrence in `tags`.
        let (kept, missing) = reconcile_tags("dup:1, dup:1\n", &tags(&[("dup", "1")]));
        assert_eq!(kept, "dup:1\n");
        assert_eq!(missing, vec![]);

        // An emptied first line stays (it is the transaction's own line).
        let (kept, _) = reconcile_tags("id:x\nnote\n", &[]);
        assert_eq!(kept, "\nnote\n");
    }
}
