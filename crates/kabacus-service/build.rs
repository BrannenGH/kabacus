// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: GPL-3.0-or-later

fn main() {
    println!("cargo:rerun-if-env-changed=SOURCE_COMMIT");
    println!("cargo:rerun-if-env-changed=KABACUS_COMMIT");
    println!("cargo:rerun-if-env-changed=KABACUS_SOURCE_URL");
    println!("cargo:rerun-if-env-changed=SOURCE_URL");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs");
    let commit = std::env::var("KABACUS_COMMIT")
        .or_else(|_| std::env::var("SOURCE_COMMIT"))
        .ok()
        .filter(|c| !c.is_empty())
        .or_else(|| {
            std::process::Command::new("git")
                .args(["rev-parse", "HEAD"])
                .output()
                .ok()
                .filter(|r| r.status.success())
                .map(|r| String::from_utf8_lossy(&r.stdout).trim().to_string())
        })
        .unwrap_or_default();
    let repository = std::env::var("KABACUS_SOURCE_URL")
        .or_else(|_| std::env::var("SOURCE_URL"))
        .unwrap_or_else(|_| "https://github.com/BrannenGH/kabacus".into());
    let url = if commit.is_empty() {
        repository
    } else {
        format!("{repository}/tree/{commit}")
    };
    println!("cargo:rustc-env=KABACUS_LIBRARY_SOURCE={url}");
}
