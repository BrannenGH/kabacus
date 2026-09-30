// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Bakes the exact commit and the source URL into the binary for
//! `get_version`. `KABACUS_COMMIT` (the variable, or a file of that name at
//! the repository root) and `KABACUS_SOURCE_URL` override what the checkout
//! says, for builds without git history (containers, packages).

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rerun-if-env-changed=KABACUS_COMMIT");
    println!("cargo::rerun-if-env-changed=KABACUS_SOURCE_URL");

    let commit = std::env::var("KABACUS_COMMIT")
        .ok()
        .filter(|c| !c.is_empty())
        .or_else(commit_file)
        .or_else(git_commit)
        .unwrap_or_default();
    println!("cargo::rustc-env=KABACUS_COMMIT={}", commit.trim());

    let url = std::env::var("KABACUS_SOURCE_URL")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| std::env::var("CARGO_PKG_REPOSITORY").unwrap_or_default());
    let url = url.trim().trim_end_matches('/').trim_end_matches(".git");
    println!("cargo::rustc-env=KABACUS_SOURCE_URL={url}");

    watch_head();
}

fn commit_file() -> Option<String> {
    println!("cargo::rerun-if-changed=../../KABACUS_COMMIT");
    let commit = std::fs::read_to_string("../../KABACUS_COMMIT").ok()?;
    Some(commit.trim().to_string()).filter(|c| !c.is_empty())
}

fn git_commit() -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    String::from_utf8(out.stdout).ok()
}

/// Rebuild when the checked-out commit moves: `.git/HEAD` for a switch, the
/// ref it names for a new commit on the same branch.
fn watch_head() {
    let git_dir = Path::new("../../.git");
    let head = git_dir.join("HEAD");
    println!("cargo::rerun-if-changed={}", head.display());
    if let Some(reference) = std::fs::read_to_string(&head)
        .ok()
        .and_then(|h| h.strip_prefix("ref: ").map(|r| r.trim().to_string()))
    {
        println!(
            "cargo::rerun-if-changed={}",
            git_dir.join(reference).display()
        );
    }
}
