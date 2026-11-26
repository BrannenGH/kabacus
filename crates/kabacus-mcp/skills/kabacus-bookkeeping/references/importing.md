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

## SimpleFIN

- Ask for a setup token if the person wants to connect. An Access URL is a private credential. Preview the connection with `dry_run: true`, confirm the host and any replacement, then connect. A local file holds that credential until the person disconnects or revokes it.
- List and link accounts explicitly. A sync fetches all of the caller's linked accounts together. Account lists and syncs share a limited daily request budget, so reuse the preview's snapshot for review and commit.
- Check `pending_count` with `get_simplefin_connection`. Use `pending: true` to review the saved queue without a bank request. Keep `pending: true` on the confirmed import. A due `automatic: true` request fetches into the queue.
- Show counts, balance checks, and warnings for every account in the preview. Send `dry_run: false` with the same snapshot and head after the person confirms. If the snapshot expired, preview again.
