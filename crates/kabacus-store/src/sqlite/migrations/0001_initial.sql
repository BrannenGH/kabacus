-- SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
--
-- SPDX-License-Identifier: GPL-3.0-or-later

-- Integrity rules only: keys, CHECKs, uniqueness and the append-only audit
-- log. Amounts are decimal text; the arithmetic is done in Rust. TEXT
-- columns use SQLite's default BINARY collation, which orders by UTF-8 bytes.

CREATE TABLE users (
    id            INTEGER PRIMARY KEY,
    display_name  TEXT    NOT NULL,
    password_hash TEXT,
    is_admin      INTEGER NOT NULL DEFAULT 0 CHECK (is_admin IN (0, 1)),
    created_at    TEXT    NOT NULL
) STRICT;

CREATE TABLE identities (
    provider TEXT    NOT NULL,
    subject  TEXT    NOT NULL,
    user_id  INTEGER NOT NULL REFERENCES users (id),
    PRIMARY KEY (provider, subject)
) STRICT;

CREATE INDEX identities_user ON identities (user_id);

CREATE TABLE ledgers (
    id         INTEGER PRIMARY KEY,
    name       TEXT    NOT NULL,
    revision   INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    created_at TEXT    NOT NULL
) STRICT;

CREATE TABLE memberships (
    ledger_id INTEGER NOT NULL REFERENCES ledgers (id) ON DELETE CASCADE,
    user_id   INTEGER NOT NULL REFERENCES users (id),
    role      TEXT    NOT NULL CHECK (role IN ('read', 'write', 'rewrite')),
    PRIMARY KEY (ledger_id, user_id)
) STRICT;

CREATE INDEX memberships_user ON memberships (user_id);

CREATE TABLE accounts (
    ledger_id     INTEGER NOT NULL REFERENCES ledgers (id) ON DELETE CASCADE,
    name          TEXT    NOT NULL CHECK (name <> ''),
    declared_tags TEXT    NOT NULL DEFAULT '',
    PRIMARY KEY (ledger_id, name)
) STRICT;

CREATE TABLE commodities (
    ledger_id   INTEGER NOT NULL REFERENCES ledgers (id) ON DELETE CASCADE,
    symbol      TEXT    NOT NULL,
    style_side  TEXT    NOT NULL DEFAULT 'left' CHECK (style_side IN ('left', 'right')),
    style_space INTEGER NOT NULL DEFAULT 0 CHECK (style_space IN (0, 1)),
    decimal_mark TEXT   NOT NULL DEFAULT '.' CHECK (decimal_mark IN ('.', ',')),
    group_mark  TEXT    NOT NULL DEFAULT '',
    group_sizes TEXT    NOT NULL DEFAULT '',
    places      INTEGER CHECK (places IS NULL OR places BETWEEN 0 AND 255),
    PRIMARY KEY (ledger_id, symbol)
) STRICT;

CREATE TABLE transactions (
    ledger_id        INTEGER NOT NULL REFERENCES ledgers (id) ON DELETE CASCADE,
    id               INTEGER NOT NULL CHECK (id > 0),
    date             TEXT    NOT NULL CHECK (date IS date(date)),
    status           TEXT    NOT NULL DEFAULT 'unmarked'
                             CHECK (status IN ('unmarked', 'pending', 'cleared')),
    code             TEXT    NOT NULL DEFAULT '',
    description      TEXT    NOT NULL DEFAULT '',
    comment          TEXT    NOT NULL DEFAULT '',
    revision_created INTEGER NOT NULL CHECK (revision_created >= 1),
    revision_changed INTEGER NOT NULL CHECK (revision_changed >= revision_created),
    PRIMARY KEY (ledger_id, id)
) STRICT;

CREATE INDEX transactions_by_date ON transactions (ledger_id, date, id);

CREATE TABLE postings (
    ledger_id        INTEGER NOT NULL,
    transaction_id   INTEGER NOT NULL,
    position         INTEGER NOT NULL CHECK (position >= 0),
    account          TEXT    NOT NULL,
    kind             TEXT    NOT NULL DEFAULT 'real'
                             CHECK (kind IN ('real', 'virtual', 'balanced_virtual')),
    status           TEXT    NOT NULL DEFAULT 'unmarked'
                             CHECK (status IN ('unmarked', 'pending', 'cleared')),
    mantissa         TEXT    NOT NULL
                             CHECK (mantissa NOT GLOB '*[^0-9]*' AND mantissa <> ''
                                 OR substr(mantissa, 1, 1) = '-'
                                    AND length(mantissa) > 1
                                    AND substr(mantissa, 2) NOT GLOB '*[^0-9]*'),
    places           INTEGER NOT NULL CHECK (places BETWEEN 0 AND 255),
    commodity        TEXT    NOT NULL,
    cost_kind        TEXT    CHECK (cost_kind IN ('unit', 'total')),
    cost_mantissa    TEXT    CHECK (cost_mantissa NOT GLOB '*[^0-9]*' AND cost_mantissa <> ''
                                 OR substr(cost_mantissa, 1, 1) = '-'
                                    AND length(cost_mantissa) > 1
                                    AND substr(cost_mantissa, 2) NOT GLOB '*[^0-9]*'),
    cost_places      INTEGER CHECK (cost_places BETWEEN 0 AND 255),
    cost_commodity   TEXT,
    assert_op        TEXT    CHECK (assert_op IN ('=', '==', '=*', '==*')),
    assert_mantissa  TEXT    CHECK (assert_mantissa NOT GLOB '*[^0-9]*' AND assert_mantissa <> ''
                                 OR substr(assert_mantissa, 1, 1) = '-'
                                    AND length(assert_mantissa) > 1
                                    AND substr(assert_mantissa, 2) NOT GLOB '*[^0-9]*'),
    assert_places    INTEGER CHECK (assert_places BETWEEN 0 AND 255),
    assert_commodity TEXT,
    comment          TEXT    NOT NULL DEFAULT '',
    PRIMARY KEY (ledger_id, transaction_id, position),
    FOREIGN KEY (ledger_id, transaction_id)
        REFERENCES transactions (ledger_id, id) ON DELETE CASCADE,
    FOREIGN KEY (ledger_id, account) REFERENCES accounts (ledger_id, name),
    FOREIGN KEY (ledger_id, commodity) REFERENCES commodities (ledger_id, symbol),
    FOREIGN KEY (ledger_id, cost_commodity) REFERENCES commodities (ledger_id, symbol),
    CHECK ((cost_kind IS NULL) = (cost_mantissa IS NULL)
       AND (cost_kind IS NULL) = (cost_places IS NULL)
       AND (cost_kind IS NULL) = (cost_commodity IS NULL)),
    CHECK ((assert_op IS NULL) = (assert_mantissa IS NULL)
       AND (assert_op IS NULL) = (assert_places IS NULL)
       AND (assert_op IS NULL) = (assert_commodity IS NULL))
) STRICT;

CREATE INDEX postings_by_account ON postings (ledger_id, account);

CREATE TABLE audit_log (
    ledger_id INTEGER NOT NULL REFERENCES ledgers (id) ON DELETE CASCADE,
    revision  INTEGER NOT NULL CHECK (revision >= 1),
    n         INTEGER NOT NULL CHECK (n >= 0),
    user_id   INTEGER NOT NULL REFERENCES users (id),
    actor     TEXT    NOT NULL,
    kind      TEXT    NOT NULL,
    summary   TEXT    NOT NULL,
    payload   TEXT    NOT NULL CHECK (json_valid(payload)),
    PRIMARY KEY (ledger_id, revision, n)
) STRICT;

-- Single rows are never rewritten. A ledger's rows go with the ledger.
CREATE TRIGGER audit_log_no_update BEFORE UPDATE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit_log is append-only');
END;

CREATE TRIGGER audit_log_no_delete BEFORE DELETE ON audit_log
WHEN EXISTS (SELECT 1 FROM ledgers WHERE id = OLD.ledger_id)
BEGIN
    SELECT RAISE(ABORT, 'audit_log is append-only');
END;
