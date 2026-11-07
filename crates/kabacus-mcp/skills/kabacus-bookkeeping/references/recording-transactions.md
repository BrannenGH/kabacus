# Recording a transaction

Use `add_transaction`. A transaction moves money between two or more
accounts; its postings sum to zero.

Accounts default to real physical accounts (`synthetic: false`); funds and
card reserves default to synthetic. Before recording a transfer, use
`list_accounts` to compare `synthetic` and `physical_home`, including the
real home holding a reserve's cash. For different physical homes where
at least one is non-null, explain that a
real bank or cash transfer is required. Refuse to preview or record it
unless the user explicitly confirms the transfer already happened in the
real world; asking to move reserves or approving a bookkeeping preview
does not confirm this. Kabacus cannot perform the transfer. Never bypass
`allocate`'s physical-boundary rejection with this tool, edits, rewrites or
`batch` without that confirmation. Once confirmed, record its actual date
and amount. When both physical homes are null, the move is wholly synthetic
and requires no real-world transfer. Allocations within one allocation and
physical home use `allocate`. Never use `set_account_type` to bypass the
confirmation; reclassification requires the user's request.

## Fields

- `date`: `YYYY-MM-DD`, the day it happened.
- `description`: `Payee | memo`. The part before ` | ` is the payee, used for
  payee suggestions; the memo is optional (`Corner Market` alone is fine).
- `postings`: at least two. Amounts are decimal strings in the books'
  commodity: `"62.18"`, `"-0.04"`. Never a JSON number. (The exact object
  `{"commodity": "USD", "mantissa": 6218, "places": 2}` also works.) Leave
  **one** posting's `amount` out and the server fills it in so the total is
  zero.
- `tags`: optional `{key, value}` pairs. The server gives every new
  transaction its `id`, a number that never changes.

## Signs

Positive means money *into* an account's balance, negative means out:

| Event | Postings |
|---|---|
| A purchase with a debit card | `expenses:groceries +62.18`, `assets:checking` (blank) |
| A purchase on a credit card | use `record_card_purchase`, which adds the card's reserve lines |
| A paycheck | `assets:checking +1800.00`, `income:salary` (blank) |
| Paying the card | use `pay_card`, which pays it from the card's reserve |
| Moving to savings | `assets:savings +500.00`, `assets:checking` (blank) |

## Before you add

- Look the accounts up with `list_accounts` (friendly names like "Everyday
  Checking" are shown next to the account paths). Use the account paths.
- `search_transactions` with the payee finds how similar transactions were
  recorded before; follow that pattern.
- Spending something a fund was set aside for comes out of the fund, e.g.
  `assets:bank:checking:insurance` rather than `assets:bank:checking`. Every
  transaction on a credit card also moves the card's reserve. See
  [budget.md](budget.md).
- A payment involving a person goes through their receivable and payable
  accounts. See [counterparties.md](counterparties.md).

Recording many at once (a statement's worth) is one `batch`; see
[workflow.md](workflow.md).
