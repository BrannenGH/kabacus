# Heads, dry runs and errors

## The head

The books' version is their revision, a number that goes up by one with
every write. Every write tool takes it as `head` and the server refuses the
write if the books changed since (another app or another agent).

- Get it from `get_version` (`head`) or `get_history`: the first entry's
  `revision`.
- After each committed write, the result's `head` is the new version.
- A dry run returns the same `head` you sent, unchanged. A stale head is
  refused in a dry run too (`head_conflict`), so a preview never shows a
  change to books that have moved on.

## Dry runs

Every write tool defaults to `dry_run: true`. The preview is exactly what the
server would do:

- Every ordinary write returns `{id, before, after, balance_changes}`: the
  transaction's text before (none for a new one) and after, and each
  touched account's balance before and after. A write that changes several
  transactions also lists the first few under `transactions`, and `changed`
  says how many there are in all. A card transaction missing its reserve
  line is flagged `unreserved: true` (it is still allowed).
- `include_diff: true` adds a unified diff of the changed transactions.
  Only the transactions you touch ever change.
- `warnings` says what you should know but wasn't refused: `unchanged`
  (nothing to write).

Every dry run is checked exactly like the real write (the strict check,
balance assertions included), so a clean preview means the commit will
pass too, unless the books change meanwhile.

## Errors

Every error's data is `{code, message, path?, details?}`. `code` is one of
`head_conflict`, `validation_failed`, `role_required`, `amount_format`,
`not_found`, `account_in_use`, `batch_too_large`, `reconcile_mismatch`,
`rewrite_disabled`. `path` names the argument at
fault (`postings[1].amount`, `ops[3]`).

| Error | Meaning | What to do |
|---|---|---|
| `head_conflict`: "the books changed since head …" | Someone wrote in between | Re-read what you were changing, take `details.current_head`, preview again. Never resend blindly. |
| `amount_format` | An amount was a JSON number or not a decimal | Send it as a string: `"45.68"`. |
| "… (rule: declared-account)", or "not declared" | An account isn't declared | Pick one from `list_accounts`; don't invent accounts. |
| "… (rule: balance)", or "doesn't balance" | Postings don't sum to zero | Fix the amounts, or leave exactly one amount unset. |
| "At the end of …, … holds …, not the … you asserted. (rule: assertion)" | A balance assertion would break | The message names the date, the account, both balances and the asserting transaction and posting; the error's `details` has `transaction` and `posting` as fields, every failure under `failures`, and `fix_tool: "set_balance_assertion"`. Tell the user. If the assertion is out of date, see "A balance assertion blocks a write" in changing-transactions.md; never change the transaction's date to dodge it. |
| "… (rule: validation)" or another rule | Caught before the books were checked: a bad date, an empty description, no postings | Read the message; it names the field. |
| "… was not found" | Wrong or stale id | Search again; an id is the transaction's number, like `42`. |
| missing role | The server's config doesn't allow this | Tell the user; there's no other route. |

`details.failures` lists every failure of the check, each with its
sentence; quote them to the user when it helps.

## Pages

`search_transactions` and `get_register` return a page:
`{transactions | lines, next_cursor, total_count, truncated}`, 200 rows by
default (`limit` up to 1,000). Pass `next_cursor` back as `cursor`, with the
same filters, for the next page. Ask only for what you need:
`count_only: true` for a count, `fields: ["id", "date", "payee"]` for some
columns. `cash_on_hand` is a summary unless you ask for `detail: "full"`.

## Batches

`batch` runs up to 100 write operations as one: each is
`{tool, args, client_ref?}`, the tool's own arguments without `head` and
`dry_run`. They apply in order, each seeing the ones before it, so one
batch can declare a fund and then allocate into it; `{"$ref": "<client_ref>.id"}`
in an argument is an earlier operation's id. The result is checked once
and becomes one revision (`commit: "per_op"` for one each).

- Dry-run it first (the default). In the default `atomic` mode any failure
  writes nothing and names the operation (`details.index`, `client_ref`).
  `mode: "continue_on_error"` (dry runs only) lists every failure at once.
- A batch with a rewrite tool in it needs the rewrite gates; dry-run it and
  show the user before committing. It commits like any batch: one call
  with `dry_run: false` and the current `head`.
- Give it a `batch_id` when you commit: if you're unsure whether it went
  through, committing the same id again returns the first result and
  writes nothing.
