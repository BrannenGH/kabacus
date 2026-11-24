<!--
SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>

SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Import a statement

1. Get the current `head` with `get_version`. Use `list_accounts` to confirm the bank or credit card account with the person.
2. Call `import_statement` with the account, the file format, the head, and `content` or `content_base64`. The default is a preview. If OFX or QFX lists several statements, ask which one to use and send its `statement` index. For CSV, confirm the detected columns over the sample and send a `mapping` with zero-based column indices. Ask which date format to use if the sample could be read more than one way.
3. Show the counts, warnings and any balance difference. Review the proposed matches, using `cursor` if there are more rows. A decision needs the row's `import_id` and an `action` of `add`, `skip`, or `match`. For `match`, give the `transaction` ID from the row's candidates. Preview again with the decisions.
4. After the person approves the result, use the same head, file and decisions with `dry_run: false`. If the head changed, get a new preview and approval.
5. Point the person at transactions in `expenses:uncategorized` to categorize. A matched transaction keeps its original category.
