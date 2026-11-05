-- SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
--
-- SPDX-License-Identifier: GPL-3.0-or-later

-- The database is a .kabacus file: application id 'KBCS'.
PRAGMA application_id = 1262633811;

-- Transaction ids count up per ledger and are never reused, not even
-- after the highest is deleted.
ALTER TABLE ledgers ADD COLUMN next_transaction_id INTEGER NOT NULL DEFAULT 1
    CHECK (next_transaction_id >= 1);
