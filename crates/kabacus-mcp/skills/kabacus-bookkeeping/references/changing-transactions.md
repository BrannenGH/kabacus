# Changing an existing transaction

Find it first with `search_transactions` or `get_register`; you need its
`id`, its number in the books. `get_transaction(id, include_history: true)`
shows it with the writes that changed it. Every change is its own revision,
so the earlier version can always be recovered.

## Which tool

| Situation | Tool | What happens |
|---|---|---|
| A typo or a wrong amount, account, date or payee | `edit_transaction` | The transaction is changed in place and keeps its id. |
| The description or postings were wrong but the date was right | `correct_transaction` | Changed in place like an edit, keeping its id and date. |
| It didn't happen (duplicate, cancelled) | `void_transaction` | A reversal is added, dated today, whose comment says which transaction it reverses (`Reverses #42 of …`). The original stays, and the two cancel. Confirm with the user. |
| It has cleared the bank, or is pending | `set_cleared` | Marks one account's posting `pending` or `cleared`; nothing else changes. |

When unsure between editing and voiding, ask the user. Editing keeps the
register tidy; voiding keeps an explicit trail in the books themselves.
Either way the history keeps every earlier version.

## edit_transaction

Pass the full new version: `date`, `description`, `postings` (same rules as
adding: decimal-string amounts, at most one blank). `tags` are kept as they
were unless you pass them. Posting statuses are
whatever you pass, so copy `status` from the current postings when the edit
shouldn't change them.

Everything else is kept as it is, and `rewrite_transaction` does the
same: the transaction's cleared mark, code and comment, and on every posting whose account you still pass, its comment and
whether it's `(virtual)` or `[balanced virtual]`. When you pass `tags`, only
the declarations of tags you added or dropped change in the comment.
Balanced virtual postings balance among themselves, so give each
`(virtual)` posting its amount.

Balance assertions (`assets:checking  -20.00 USD = 980.00 USD`) are kept
on every posting whose account you still pass, so leave them out unless you
mean to change them:

- To change an assertion's amount, pass `balance_assertion` (an exact
  amount, like `amount`) on that posting. Its `==` or `=*` form is kept.
  Do this when the user tells you the asserted balance itself was wrong, or
  when they confirm the new balance against a statement.
- To remove one, pass `clear_balance_assertion: true` on that posting.
- An edit that leaves out an asserted posting's account is refused. Clear
  the assertion in an edit of its own first, if the user agrees.

Changing an amount can break an assertion on the same posting or on a
later one ("rule: assertion"). Don't change the assertion just to make the
error go away: tell the user which balance no longer matches and ask.
An assertion on another transaction is changed with
`set_balance_assertion` (see "A balance assertion blocks a write" below).

The dry run returns the transaction's text `before` and `after`; check that
only what you meant to change differs before committing. Amounts are shown
in the commodity's style (`-$9.93`, `$1,234.56`).

## Rewrites: delete, declare an account, change an assertion

`delete_transaction`, `rewrite_transaction`, `declare_account`,
`rename_account`, `remove_account`, `set_balance_assertion` and
`convert_money_assertions` rewrite the books rather than adding to them. They're listed
only when your sign-in holds `ledger-rewrite` and the server wasn't started
with `--no-rewrite`; otherwise you don't have them, so void instead of
deleting and ask the user to declare a missing account.

They default to `dry_run: true`, like every write tool:

1. Call the tool with the current `head`. It returns a `diff` (for an
   account rename or removal, `details`: how many references change and
   three samples). Nothing has changed.
2. Show the user the diff and get an explicit yes. Then call the tool again
   with `dry_run: false` and the same `head`.

Prefer `edit_transaction` to `rewrite_transaction` and `void_transaction`
to `delete_transaction`: they're validated as ordinary writes and need no
extra confirmation.

## A balance assertion blocks a write

A balance assertion says what an account's balance must be on a date. A
real transaction that reached the user late (a bank sync, a migration
check) can contradict one. The write then fails with "At the end of …, …
not the … you asserted.", and the error's `details` has the asserting
`transaction` and `posting`, with both balances in `failures`.

Don't change the transaction's date to get past it. Tell the user which
assertion blocks it, and with their yes:

1. Remove the assertion: `set_balance_assertion` with the error's
   `transaction`, the posting's `account` and no `balance`, and
   `dry_run: false`.
2. Retry the write with the new `head`.
3. If the assertion should stay, set it to its new balance:
   `set_balance_assertion` again with the same `transaction` and
   `account` and the new `balance`, and `dry_run: false`. A balance the
   books don't reach is refused, so check it against the user's statement
   or bank first.

Setting the new balance before the retry fails: the books don't reach it
until the blocked transaction is in. All three steps can be one `batch`
(it's a rewrite, so dry-run it first and show the user).
