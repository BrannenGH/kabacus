// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unified diffs of journal text, for previews and history.

/// Lines of context around each change.
pub const CONTEXT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Same,
    Del,
    Add,
}

/// The longest common subsequence's edit script, or a plain replacement
/// when the inputs are too big to compare line by line.
fn edit_script<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<(Op, &'a str)> {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let (a, b) = (
        &old[prefix..old.len() - suffix],
        &new[prefix..new.len() - suffix],
    );
    let mut script: Vec<(Op, &str)> = old[..prefix].iter().map(|l| (Op::Same, *l)).collect();
    if a.len() == b.len() && a.len() * b.len() > 4_000_000 {
        // Same length and too big for LCS: compare line by line.
        for (x, y) in a.iter().zip(b) {
            if x == y {
                script.push((Op::Same, x));
            } else {
                script.push((Op::Del, x));
                script.push((Op::Add, y));
            }
        }
    } else if a.len() * b.len() > 4_000_000 {
        script.extend(a.iter().map(|l| (Op::Del, *l)));
        script.extend(b.iter().map(|l| (Op::Add, *l)));
    } else {
        let (n, m) = (a.len(), b.len());
        let mut lcs = vec![vec![0u32; m + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lcs[i][j] = if a[i] == b[j] {
                    lcs[i + 1][j + 1] + 1
                } else {
                    lcs[i + 1][j].max(lcs[i][j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n || j < m {
            if i < n && j < m && a[i] == b[j] {
                script.push((Op::Same, a[i]));
                i += 1;
                j += 1;
            } else if i < n && (j == m || lcs[i + 1][j] >= lcs[i][j + 1]) {
                script.push((Op::Del, a[i]));
                i += 1;
            } else {
                script.push((Op::Add, b[j]));
                j += 1;
            }
        }
    }
    script.extend(old[old.len() - suffix..].iter().map(|l| (Op::Same, *l)));
    script
}

/// A unified diff of `old` and `new`: only the changed lines, with
/// [`CONTEXT`] lines around each change. Never the whole file.
pub fn unified_diff(path: &str, old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let script = edit_script(&old_lines, &new_lines);
    let changed: Vec<usize> = script
        .iter()
        .enumerate()
        .filter(|(_, (op, _))| *op != Op::Same)
        .map(|(i, _)| i)
        .collect();
    let mut out = String::new();
    if changed.is_empty() {
        return out;
    }
    out.push_str(&format!("--- a/{path}\n+++ b/{path}\n"));
    // Group changes whose context overlaps into hunks.
    let mut hunks: Vec<(usize, usize)> = Vec::new();
    for &i in &changed {
        let start = i.saturating_sub(CONTEXT);
        let end = (i + CONTEXT + 1).min(script.len());
        match hunks.last_mut() {
            Some(last) if start <= last.1 => last.1 = end,
            _ => hunks.push((start, end)),
        }
    }
    // Line numbers at each script position.
    let mut old_no = vec![0usize; script.len() + 1];
    let mut new_no = vec![0usize; script.len() + 1];
    for (i, (op, _)) in script.iter().enumerate() {
        old_no[i + 1] = old_no[i] + usize::from(*op != Op::Add);
        new_no[i + 1] = new_no[i] + usize::from(*op != Op::Del);
    }
    for (start, end) in hunks {
        let old_len = old_no[end] - old_no[start];
        let new_len = new_no[end] - new_no[start];
        out.push_str(&format!(
            "@@ -{},{old_len} +{},{new_len} @@\n",
            old_no[start] + usize::from(old_len > 0),
            new_no[start] + usize::from(new_len > 0),
        ));
        for (op, line) in &script[start..end] {
            let mark = match op {
                Op::Same => ' ',
                Op::Del => '-',
                Op::Add => '+',
            };
            out.push(mark);
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diffs_show_only_changed_lines_with_context() {
        let old: String = (1..=100).map(|i| format!("line {i}\n")).collect();
        let new = old.replace("line 50\n", "line fifty\nline 50b\n");
        let diff = unified_diff("2022.journal", &old, &new);
        assert!(diff.contains("@@ -47,7 +47,8 @@"), "{diff}");
        assert!(
            diff.contains("-line 50\n+line fifty\n+line 50b\n"),
            "{diff}"
        );
        assert!(
            diff.contains(" line 47\n") && diff.contains(" line 53\n"),
            "{diff}"
        );
        assert!(
            !diff.contains("line 46\n") && !diff.contains("line 54\n"),
            "{diff}"
        );
        assert_eq!(unified_diff("x", &old, &old), "");
    }
}
