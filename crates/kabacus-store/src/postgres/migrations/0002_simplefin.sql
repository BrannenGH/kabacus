-- SPDX-FileCopyrightText: 2026 Brannen Hall <brannen@hallb.me>
--
-- SPDX-License-Identifier: GPL-3.0-or-later

CREATE TABLE simplefin_access (
    ledger_id bigint NOT NULL REFERENCES ledgers (id) ON DELETE CASCADE,
    slot bigint NOT NULL,
    user_id bigint,
    access_url text NOT NULL,
    claimed_at bigint NOT NULL,
    requests text NOT NULL DEFAULT '[]',
    PRIMARY KEY (ledger_id, slot),
    FOREIGN KEY (ledger_id, user_id) REFERENCES memberships (ledger_id, user_id) ON DELETE CASCADE,
    CHECK (slot = COALESCE(user_id, 0))
);

ALTER TABLE simplefin_access ENABLE ROW LEVEL SECURITY;
ALTER TABLE simplefin_access FORCE ROW LEVEL SECURITY;
CREATE POLICY simplefin_owner ON simplefin_access
    USING (ledger_id = NULLIF(current_setting('kabacus.ledger', true), '')::bigint
           AND slot = NULLIF(current_setting('kabacus.simplefin_user', true), '')::bigint)
    WITH CHECK (ledger_id = NULLIF(current_setting('kabacus.ledger', true), '')::bigint
                AND slot = NULLIF(current_setting('kabacus.simplefin_user', true), '')::bigint);
