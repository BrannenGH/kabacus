# Funds, card reserves and Cash on Hand

There are no monthly budgets here: no Assigned, Activity or Available, no
goals or targets. `expenses:*` and `income:*` show what came in and where
it went. On top of that, money is set aside in **funds** and **card
reserves**, and **Cash on Hand** says how much can be spent right now.

## Account kinds

Every account has a kind, worked out from its path by a rule table in the
server's config (`account_kinds` shows the table and every account's
kind). You can't set a kind on an account, and there is no `envelope`
tag any more. With the usual rules:

| Account | Kind |
|---|---|
| `assets:bank:checking`, `assets:bank:savings`, `assets:cash` | `money` |
| `assets:bank:checking:insurance` (any sub-account of a bank account) | `pre-allocation`: a fund |
| `assets:bank:checking:card-<card>` | `debt-reserve`: a card reserve |
| `assets:receivable:<who>` | `receivable` |
| `liabilities:credit-card:<card>` | `revolving-debt`: a card |
| `liabilities:loans:*`, `liabilities:mortgage:*` | `term-liability` |
| `liabilities:payable:<who>` | `payable` |

A fund or card reserve sits under the money account holding its cash, its
**home**. `list_accounts` gives each account's `kind`, `home`,
`display_name`, `balance_exclusive` (its own postings: for a money
account, its free money) and `balance_inclusive` (with sub-accounts: what
the bank shows).

`synthetic` is independent of the accounting kind. It defaults to false
(real), except funds and card reserves default to true. `physical_home`
names the real account holding the cash: a synthetic reserve inherits it
from its home. A wholly synthetic account has null `physical_home`.
`declare_account` accepts `synthetic`; `set_account_type(account, synthetic,
head)` converts an existing account without moving money or changing its
kind, postings, balances or other tags. Conversion is a rewrite and needs
the rewrite gates. Preview it and only convert on the user's request;
never reclassify accounts to bypass a real-transfer confirmation.

## Cash on Hand

`cash_on_hand(as_of?, detail?, compare_to?)`:

```
Money on Hand      = money + funds + card reserves + receivables
Future Allocations = funds + card reserves
Cash on Hand       = Money on Hand - Future Allocations - Receivables
```

That is the free money left in the money accounts. Card debt is covered by
the reserves, so it isn't subtracted again. An overdrawn fund counts as
zero in funds: its overdraft was spent from its home's free money, so it
comes off that free money (and Cash on Hand) instead. A negative card
reserve (a payment recorded without the purchases it paid for) counts as
zero in card reserves the same way. The report also lists every
account by kind, the free money in each home, every fund, every card with
its reserve and `difference` (owed minus reserve: what the reserve still
needs), and warnings. It is a summary: each card has
`unreserved_count` and `hidden_unreserved_count` but no transaction list;
`detail: "full"` lists the first 20 per card (`unreserved_limit`) with a
`next_cursor` to continue in `search_transactions`. `compare_to` adds how
each part changed since another day. Pass on the warnings; none of them
blocks a write:

| Warning | Means |
|---|---|
| `negative-fund` | A fund is overdrawn. |
| `negative-free-money` | A home's funds and reserves hold more than the account. |
| `negative-reserve` | A card's reserve is negative or the card is in credit: usually a payment whose purchases were never recorded. |
| `reserve-mismatch` | A card's reserve doesn't equal what it owes; its `unreserved` transactions say why. |
| `unreserved-unaccounted` | The flagged transactions don't add up to the card's `difference`: money moved in or out of the reserve on its own. |
| `unclassified` | No kind rule matches an account. |
| `uncategorized` | `expenses:uncategorized` isn't zero. |
| `duplicate-display-name` | Two accounts share a display name. |
| `exclusive-assertion` | A money account still has an `=` balance assertion; see below. |

## Funds

A fund is money set aside for a future expense. It carries forward for
ever; there are no months or resets. Bills and long-term savings are
funded; household and social spending comes from free money.

Move money with `allocate(from, to, amount, date, description?, head)`:

| Goal | from | to |
|---|---|---|
| Set money aside | `assets:bank:checking` | `assets:bank:checking:insurance` |
| Release it again | the fund | `assets:bank:checking` |
| Move between funds | one fund | another fund of the same account |

To set money aside into a fund that doesn't exist yet, pass
`new_fund: true` with `to` the new fund directly in the money account
`from` (`assets:bank:checking:vacation`): the same write declares it. No
other account can be made this way; it's refused if the fund exists.

`from` and `to` must share an allocation home and a physical home. Compare
`synthetic` and `physical_home`, not just the reserve flags: reserves in
different physical accounts are synthetic but their cash is held separately.
If physical homes differ and at least one is non-null, the move requires a real
bank or cash transfer. Explain this and refuse to preview
or record the move unless the user explicitly confirms the transfer already happened
in the real world. A request to move reserves or a yes to a bookkeeping
preview is not confirmation that cash moved. Kabacus cannot perform the
transfer. Do not bypass an `allocate` physical-boundary rejection with `add_transaction`,
edits, rewrites or `batch`. After confirmation, use `add_transaction` to
record the actual transfer with its actual date and amount.
When both physical homes are null, no real transfer is needed. `allocate`
still requires one allocation home; other synthetic moves use an ordinary
transaction.

Allocating lowers Cash on Hand: the
money is no longer free. Moving between funds doesn't change it.

`allocate` takes only what `from` holds: its balance on `date` and on
every later day must cover `amount`, or the write is refused with the
shortfall.

## Card reserves

Every card has one reserve, named after it:
`liabilities:credit-card:visa` has `assets:bank:checking:card-visa`. The
reserve always equals what the card owes, so **every** transaction on a
card also moves its reserve by the opposite amount. One that doesn't is
**unreserved**: it's still written, but `search_transactions`,
`get_register` and the write tools flag it with `unreserved: true` and
`missing_reserve` (what the reserve still needs).
`search_transactions(unreserved_only: true)` lists them all; fix one with
`edit_transaction`, adding the reserve lines, and many with one `batch` of
edits.

Every transaction counts, voided ones and their reversals too, and each is
judged on its own. The flagged amounts always add up to the card's
`difference`.

- `record_card_purchase(card, expense_account, amount, date, description,
  source, head)` writes a purchase with its reserve lines. `source` is
  `"free"` (discretionary) or the fund paying for it. A negative amount is
  a refund back to the same source.
- `pay_card(card, amount, date, head)` pays the card from its reserve.
- `card_reserves(card?)` shows each card's balance and reserve (they should
  match) and what the reserve holds by source: free money or each fund,
  counted since it last reached zero, payments taken oldest first.

## The transaction patterns

Amounts are examples.

| Event | Postings | Cash on Hand |
|---|---|---|
| Allocate to a fund | `checking:insurance +100`, `checking -100` | down |
| Release a fund | `checking +100`, `checking:insurance -100` | up |
| Move between funds | fund B `+`, fund A `-` | no change |
| Spend from a fund at the bank | `expenses:bills:insurance +100`, `checking:insurance -100` | no change |
| Discretionary purchase on a card | `expenses:household:groceries +40`, `credit-card:visa -40`, `checking -40`, `checking:card-visa +40` | down |
| Purchase from a fund on a card | `expenses:bills:insurance +100`, `credit-card:visa -100`, `checking:insurance -100`, `checking:card-visa +100` | no change |
| Interest or a fee on a card | like a discretionary purchase, to the interest or fee expense | down |
| Refund on a card | the reverse of the purchase, back to free money or the fund | up / no change |
| Pay a card | `credit-card:visa +425.00`, `checking:card-visa -425.00` | no change |
| Reimbursable expense | `receivable:hsa +100`, `checking -100` (on a card the receivable takes the expense's place, with the reserve lines) | down until repaid |
| Reimbursement arrives | `checking +100`, `receivable:hsa -100` | up |
| Income | `checking +1800`, `income:salary -1800` | up |
| Mortgage or loan from a fund | `mortgage:home +P`, `expenses:interest:home +I`, `checking:home-mortgage -(P+I)` | no change |
| A co-owner's share of a loan payment | `mortgage:home -S`, `receivable:<person> +S` (never a credit to an expense) | no change |
| They repay it | `checking +S`, `receivable:<person> -S` | up |

(`checking` is `assets:bank:checking`; `credit-card:visa` is
`liabilities:credit-card:rewards-visa`.) Keep the free money, fund
and reserve lines of one transaction in the same home.

## Balance assertions

Assertions on money accounts are inclusive (`=*`): they check the bank's
balance, funds and reserves included, so moving money between them never
breaks reconciliation. New assertions on money accounts are written that
way. If `cash_on_hand` warns `exclusive-assertion`, convert the old ones
with `convert_money_assertions(head)` (a rewrite: dry-run it, show the user,
then commit with `dry_run: false`) **before** adding reserve lines to past card
transactions, or the old `=` assertions fail.
