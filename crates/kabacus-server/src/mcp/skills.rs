// SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Bundled skill guides, in the Agent Skills layout
//! (`skills/<skill>/SKILL.md` plus `references/*.md`), compiled into the
//! server. They are served as MCP resources at `skill://<skill>/<path>`,
//! named in the server instructions, and, for clients without resources,
//! through the `get_skill_guide` tool. A file is only ever looked up by its
//! exact bundled path, so no path is built from a caller's input.

/// One bundled skill.
pub struct Skill {
    pub name: &'static str,
    /// `(path within the skill, contents)`; SKILL.md first.
    pub files: &'static [(&'static str, &'static str)],
}

macro_rules! skill_file {
    ($skill:literal, $path:literal) => {
        (
            $path,
            include_str!(concat!("../../skills/", $skill, "/", $path)),
        )
    };
}

pub static SKILLS: &[Skill] = &[Skill {
    name: "kabacus-bookkeeping",
    files: &[
        skill_file!("kabacus-bookkeeping", "SKILL.md"),
        skill_file!("kabacus-bookkeeping", "references/workflow.md"),
        skill_file!(
            "kabacus-bookkeeping",
            "references/recording-transactions.md"
        ),
        skill_file!("kabacus-bookkeeping", "references/changing-transactions.md"),
        skill_file!("kabacus-bookkeeping", "references/counterparties.md"),
        skill_file!("kabacus-bookkeeping", "references/budget.md"),
        skill_file!("kabacus-bookkeeping", "references/reports.md"),
    ],
}];

const URI_SCHEME: &str = "skill://";

impl Skill {
    /// The file at exactly `path`, if the skill bundles one.
    pub fn file(&self, path: &str) -> Option<&'static str> {
        self.files
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, body)| *body)
    }

    pub fn paths(&self) -> Vec<&'static str> {
        self.files.iter().map(|(p, _)| *p).collect()
    }

    /// The `description` from SKILL.md's frontmatter.
    pub fn description(&self) -> &'static str {
        frontmatter_field(self.files[0].1, "description").unwrap_or("")
    }

    pub fn uri(&self, path: &str) -> String {
        format!("{URI_SCHEME}{}/{path}", self.name)
    }
}

pub fn skill(name: &str) -> Option<&'static Skill> {
    SKILLS.iter().find(|s| s.name == name)
}

/// The skill, path and contents a `skill://<skill>/<path>` URI names, if
/// that file is bundled.
pub fn resolve_uri(uri: &str) -> Option<(&'static Skill, &'static str, &'static str)> {
    let (name, path) = uri.strip_prefix(URI_SCHEME)?.split_once('/')?;
    let skill = skill(name)?;
    let (path, body) = skill.files.iter().find(|(p, _)| *p == path)?;
    Some((skill, path, body))
}

/// A `key: value` line from a markdown file's leading `---` block.
fn frontmatter_field(markdown: &'static str, key: &str) -> Option<&'static str> {
    let block = markdown.strip_prefix("---\n")?;
    let block = &block[..block.find("\n---")?];
    block.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        (k.trim() == key).then(|| v.trim())
    })
}

/// The server instructions: read SKILL.md first, then only the
/// reference files its table points to.
pub fn instructions() -> String {
    let mut text = String::from(
        "IMPORTANT: This server provides skill guides that MUST be consulted \
         before changing the books. Read the SKILL.md of the matching skill: it \
         has the workflow, the common mistakes, and a Reference files table that \
         maps each task to one reference file. Read the file for your task \
         before proceeding; do NOT load every reference file up front.\n\n\
         How to access: read the skill as an MCP resource (resources/read with \
         the skill:// URI). If your client can't read resources, call \
         get_skill_guide() with no arguments for SKILL.md, then \
         get_skill_guide(file=\"<path>\") for a file it links. If you can read \
         resources, don't spend tokens on that tool.\n",
    );
    for skill in SKILLS {
        text.push_str(&format!(
            "\n### Skill: {} ({})\n{}\n",
            skill.name,
            skill.uri("SKILL.md"),
            skill.description()
        ));
    }
    text.push_str(
        "\nAccounts default to real physical accounts (synthetic=false); funds \
         and card reserves default to synthetic=true. Before moving money, use \
         list_accounts to compare synthetic and physical_home, the real account \
         holding a reserve's cash. If physical homes differ and at least one is non-null, \
         explain that this requires a real bank or cash transfer. Refuse to \
         preview or record it unless the user explicitly confirms the transfer \
         already happened in the real world. A request to move reserves or a yes \
         to a bookkeeping preview is not that confirmation. Kabacus only records \
         transfers; it cannot perform them. Never bypass allocate's physical-boundary \
         rejection with add_transaction, edits, rewrites or batch without that \
         confirmation. Fully synthetic accounts with no physical_home do not \
         represent a real transfer. set_account_type converts an account on the \
         user's request; never change its type to bypass transfer confirmation.\n\n\
         Every write tool, batch included, defaults to dry_run=true. Preview \
         first and show the user, then call again with dry_run=false and the \
         same `head` to commit; any call with dry_run=false and the current \
         `head` commits on its own, with nothing else to send. The rewrite \
         tools (rewrite_transaction, delete_transaction, declare_account and \
         the others) appear only when the server allows rewrites and you hold \
         ledger-rewrite, and commit the same way. Every change is one \
         revision of the books, attributed to you, so earlier versions are \
         never lost.",
    );
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_guidance_requires_a_real_world_transfer() {
        let skill = skill("kabacus-bookkeeping").unwrap();
        for (surface, text) in [
            ("instructions", instructions()),
            ("SKILL.md", skill.file("SKILL.md").unwrap().to_string()),
            (
                "budget",
                skill.file("references/budget.md").unwrap().to_string(),
            ),
            (
                "recording",
                skill
                    .file("references/recording-transactions.md")
                    .unwrap()
                    .to_string(),
            ),
        ] {
            let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(text.contains("physical accounts"), "{surface}: {text}");
            assert!(
                text.contains("explicitly confirms") && text.contains("already happened"),
                "{surface}: {text}"
            );
        }
    }

    /// Every file SKILL.md links is bundled, and every bundled file is
    /// linked, so `get_skill_guide` can serve exactly what the table names.
    #[test]
    fn skill_md_links_exactly_the_bundled_files() {
        for skill in SKILLS {
            let main = skill.file("SKILL.md").unwrap();
            let mut linked: Vec<&str> = main
                .split("](")
                .skip(1)
                .filter_map(|rest| rest.split_once(')').map(|(target, _)| target))
                .collect();
            linked.sort();
            linked.dedup();
            let mut bundled: Vec<&str> = skill.paths().into_iter().skip(1).collect();
            bundled.sort();
            assert_eq!(linked, bundled, "{}", skill.name);
        }
    }
    #[test]
    fn skills_have_a_name_and_description_in_their_frontmatter() {
        for skill in SKILLS {
            let main = skill.file("SKILL.md").unwrap();
            assert_eq!(frontmatter_field(main, "name"), Some(skill.name));
            assert!(skill.description().len() > 40, "{}", skill.name);
        }
    }
    #[test]
    fn uris_resolve_only_to_bundled_files() {
        let (skill, path, body) =
            resolve_uri("skill://kabacus-bookkeeping/references/budget.md").unwrap();
        assert_eq!(skill.name, "kabacus-bookkeeping");
        assert_eq!(path, "references/budget.md");
        assert!(body.contains("Cash on Hand"));
        for bad in [
            "skill://kabacus-bookkeeping/../Cargo.toml",
            "skill://kabacus-bookkeeping//etc/passwd",
            "skill://other/SKILL.md",
            "file:///etc/passwd",
        ] {
            assert!(resolve_uri(bad).is_none(), "{bad}");
        }
    }
}
