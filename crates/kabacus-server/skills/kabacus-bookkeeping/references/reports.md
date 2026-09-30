# Reports, reconciling, and account changes

Every report counts every transaction and gives amounts as
decimal strings in the books' commodity. Periods are `from`..`to`,
inclusive, `YYYY-MM-DD`; `interval` is `none`, `month`, `quarter` or
`year`. `exclude` takes kinds (`receivable`) or account prefixes
(`expenses:reimbursable`).

## Which report

| Question | Tool |
|---|---|
| What came in, what went out, what was left | `income_statement` |
| What we own and owe; net worth | `balance_sheet` |
| Where the money went | `category_breakdown` (`compare_previous` for the change) |
| How spendable money moved over time | `cash_on_hand_history` |
| Why Cash on Hand changed this month | `cash_flow` |
| What was fronted for others, and what came back | `receivables_activity` |
| One account or category's total | `get_balance` |

## Loan principal: equity conversion

Paying down a loan isn't an expense, but the money is no longer there to
spend. `income_statement` shows it in its own `equity_conversion` section:
the drop in what is owed on each `term-liability` account, split into
`cash` (payments) and `non_cash` (interest accruals and others' shares,
which add to the debt). `net` is income minus expenses minus that;
`net_before_conversion` leaves it out. `conversion_kinds: ["term-liability",
"non-liquid"]` also counts money moved into investments.
`equity_conversion: false` leaves the section out.

**Shared loan payments.** A co-owner's or tenant's share of a payment is a
credit to the loan and a debit to their receivable; their repayment is a
credit to the receivable and a debit to checking. Never credit an expense
with it. A 380.00 payment with a 190.00 share leaves 190.00 of conversion
and 190.00 owed until repaid.

## Cash flow

`cash_flow(from, to)` bridges `opening_cash` (Cash on Hand the day before
`from`) to `closing_cash`. Its `lines` are grouped by kind (`income`,
`expense`, `receivable`, `pre-allocation`, `equity_conversion`, ...) or by
account (`group_by: "account"`, `depth`). Each line's `cash` comes from
transactions that move free money, `non_cash` from those that don't (an
interest true-up, a share booked to a receivable). The cash column adds up
to `change`, the non-cash one to zero. Card and reserve lines that cancel
are left out. `exclude: ["receivable", "payable"]` gives
`change_excluding`: the change without money fronted for or collected from
others. `include_transactions` lists each line's transactions.

`search_transactions(cash_effect: "nonzero")` finds the transactions that
move free money; `"zero"` the rest.

## Reconciling with a statement

`reconcile(account, balance, date)` compares the account's balance at the
end of `date` with the statement's `balance` (as booked: negative for what
is owed on a card or loan; a money account's inclusive balance, funds and
reserves included). It returns `ledger_balance`, `statement_balance`,
`difference` (ledger minus statement) and `status`.

- `mode: "check"` (default) writes nothing. On a `mismatch` it suggests sets
  of up to three uncleared transactions that add up to the difference (in
  the books, not yet at the bank), and `later_transactions` that would
  explain it if they were dated by the statement. Show these to the user;
  a missing transaction is the usual cause: record it, then check again.
- `mode: "assert"` writes a balance assertion on `date`, tagged
  `reconciled`, so a later change that breaks the agreement is refused. It
  is refused itself (`reconcile_mismatch`) unless the balances match within
  `tolerance`. `mark_cleared: true` also marks the account's postings up to
  `date` cleared.
- `mode: "adjust"` posts the difference to `equity:adjustments` first. Only
  with the user's explicit yes, and `confirm_adjust: true`.

Writing modes take `head` and dry-run first, like every write.

## Renaming and removing accounts

Both are rewrites: listed only with `ledger-rewrite`, dry-run first, then
committed with `dry_run: false`.

- `rename_account(from, to)` renames the account and everything under it
  everywhere: declarations, postings, balance assertions and tag values.
  The preview gives the number of references changed, how many
  transactions, and three samples. `to` must not exist; `merge: true` merges
  into an existing account of the same kind. A new path that changes an
  account's kind (a reserve renamed out of its bank account, say) is
  refused unless `allow_kind_change: true`; tell the user what changes.
- `remove_account(account)` removes a declaration made by mistake. If
  anything still uses it, it's refused with `account_in_use` and how many
  postings, assertions, directives and sub-accounts do.
