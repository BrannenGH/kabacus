# People: receivables and payables

Every person or company the household settles up with has two accounts with
the same name:

- `assets:receivable:<name>`: they owe us.
- `liabilities:payable:<name>`: we owe them.

The receivable never goes negative and the payable never goes positive.
Money moving toward a person first pays down what we owe them, then builds
what they owe us; money coming from them does the reverse.

## Recording

| Event | Postings |
|---|---|
| Paid a shared dinner of $80.00; they owe half | `expenses:dining +40.00`, `assets:receivable:jordan +40.00`, `liabilities:visa -80.00` |
| They paid us back $40.00 | `assets:checking +40.00`, `assets:receivable:jordan -40.00` |
| They paid a $30.00 bill for us | `expenses:household +30.00`, `liabilities:payable:jordan -30.00` |

When a payment is bigger than one side's balance, split it: first clear the
payable (or receivable), then put the rest on the other account. Check both
balances with `list_accounts` or `list_counterparties` first.

## Reading

- `list_counterparties`: everyone's net position, "Owes you $X" or "You owe
  $X".
- `counterparty_statement(name, from, to)`: a netted statement with a
  running balance; `open_items: true` shows only uncleared lines.

The account name segment is lower case; the person's full name is often a
friendly name on the account declaration.
