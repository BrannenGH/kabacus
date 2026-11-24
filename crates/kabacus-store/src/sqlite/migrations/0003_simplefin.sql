-- SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
--
-- SPDX-License-Identifier: GPL-3.0-or-later

CREATE TABLE simplefin_access (
    ledger_id INTEGER NOT NULL REFERENCES ledgers (id) ON DELETE CASCADE,
    slot INTEGER NOT NULL,
    user_id INTEGER,
    access_url TEXT NOT NULL,
    claimed_at INTEGER NOT NULL,
    requests TEXT NOT NULL DEFAULT '[]',
    PRIMARY KEY (ledger_id, slot),
    FOREIGN KEY (ledger_id, user_id) REFERENCES memberships (ledger_id, user_id) ON DELETE CASCADE,
    CHECK (slot = COALESCE(user_id, 0))
);
