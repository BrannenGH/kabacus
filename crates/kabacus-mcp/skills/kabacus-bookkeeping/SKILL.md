---
name: kabacus-bookkeeping
description: How to read and change a household's books through Kabacus's tools without breaking them. Use before recording, editing, correcting or voiding a transaction, marking one cleared, answering who owes whom, setting money aside in funds, recording or paying credit cards, or saying how much can be spent (Cash on Hand).
---

# Kabacus bookkeeping

Kabacus keeps a household's books (double-entry)
behind one server. Every tool here goes through that server: it checks
every write strictly, makes each write one new revision of the books, and
records that the change came from you (actor `mcp`). Nothing you do is
lost: every earlier revision stays in history.

## Transfers between physical accounts

Accounts default to real physical accounts (`synthetic: false`). Funds
(called reserves in the app) and card reserves default to synthetic
(`synthetic: true`). `list_accounts` exposes this flag and `physical_home`,
the real account holding the cash, or null for a wholly synthetic account.
Before moving money, look up both accounts and compare their physical homes.
Reserves in different banks are synthetic but have different physical homes.
Within one allocation home and physical home, use `allocate`.

If physical homes differ and at least one is non-null, explain that this crosses physical
accounts and requires a real bank or cash transfer. Refuse to preview or
record that move unless the user explicitly confirms the transfer already
happened in the real world. A request to move reserves, or approval of a
bookkeeping preview, does not establish that. Kabacus records transfers;
it cannot perform them. Once confirmed, record the actual amount and date.
Never work around `allocate`'s physical-boundary rejection with
`add_transaction`, edits, rewrites or `batch` without that confirmation.
Transfers between wholly synthetic accounts (both physical homes null)
do not require a real-world transfer. `allocate` still requires one
allocation home; use an ordinary transaction for other synthetic moves.

`set_account_type(account, synthetic, head)` converts an account between
real and synthetic, preserving its accounting kind, postings and history.
It is a rewrite: preview first, and use it only when the user requests
reclassification. Never convert accounts to bypass transfer confirmation.

## Workflow

1. **Read first.** Find what you need with `list_accounts`,
   `search_transactions`, `get_transaction`, `get_register`, `get_balance`,
   `cash_on_hand`. Lists come a page at a time (`limit`, `cursor`); ask for
   only the `fields` you need, or `count_only`. Note the current `head` from
   `get_version` (or `get_history`'s first `hash`).
2. **Preview.** Call the write tool with that `head` and no `dry_run` (it
   defaults to `true`). You get the transaction's text before and after
   (`before`, `after`), its `id`, and each touched account's balance before
   and after. Nothing is written.
3. **Show the user** what will change, in their words, and get a yes when the
   change is theirs to make.
4. **Commit.** Call the same tool again with the same arguments, the same
   `head` and `dry_run: false`. The result carries the new `head`; use it for
   the next write.

If a write fails, read [references/workflow.md](references/workflow.md)
before retrying.

## Common mistakes

| Mistake | Instead |
|---|---|
| Inventing an account name | Use one from `list_accounts`; undeclared accounts are refused. The one exception is a new expense or income category: name it in `add_transaction`'s `new_accounts` and the same write declares it. |
| Amounts as JSON numbers | Decimal strings: `"45.68"`, `"-0.04"`. A number like `45.68` is refused (`amount_format`). |
| More decimal places than the commodity has (`12.345` USD) by mistake | `get_commodity` gives its `places`; an amount with more is refused. |
| Many writes one call at a time | `batch`: up to 100 operations, one check, one revision. |
| Reading a whole list to count or find a few rows | `count_only`, filters (`account`, `tag`, `payee`, ...) and `fields`. |
| Reusing an old `head` after a write | Each write changes `head`; take it from the last result. |
| Voiding to fix a typo | Use `edit_transaction`; see changing-transactions.md. |
| Setting money aside with `add_transaction` | Use `allocate`, which checks it stays inside one account. |
| A card purchase or payment without its reserve lines | Use `record_card_purchase` and `pay_card`; flagged `unreserved` otherwise. |
| Recording a payment to or from a person with `add_transaction` | Use `settle_up` (`paid` or `received`); it nets against what is already owed, then builds the other side. |
| Committing without a preview | Always dry-run first and show the user. |

## Reference files

Read only the file for the task at hand.

| Task | Read |
|---|---|
| Heads, dry runs, error codes, paging, batches | [references/workflow.md](references/workflow.md) |
| Recording a purchase, income or transfer | [references/recording-transactions.md](references/recording-transactions.md) |
| Fixing, editing, voiding, marking cleared, deleting, updating a balance assertion, or one blocking a write | [references/changing-transactions.md](references/changing-transactions.md) |
| Money lent, borrowed, split with or owed to a person | [references/counterparties.md](references/counterparties.md) |
| Funds, credit cards and their reserves, Cash on Hand, account kinds | [references/budget.md](references/budget.md) |
| Income statement, balance sheet, cash flow, receivables, loan principal, reconciling with a statement, renaming or removing accounts | [references/reports.md](references/reports.md) |
