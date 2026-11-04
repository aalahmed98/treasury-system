-- Phase 2B: expected cash flows for the liquidity ladder, nostro statements and movements,
-- amendments restricted to internal bookings, and desk-scoped access to deal history.

-- ---------------------------------------------------------------------------
-- Expected cash flows from live golden deals. security_invoker: desk entitlement on deals applies.
-- FX: ccy1 moves by the notional on the value date, ccy2 by notional x rate the other way.
-- Money market: principal at the value date, principal plus interest at maturity
-- (ACT/360, ACT/365 for GBP). Amounts are signed from the bank's point of view.
-- ---------------------------------------------------------------------------
CREATE VIEW cash_flows_v WITH (security_invoker = true) AS
WITH live AS (
    SELECT d.*, k.code AS desk_code FROM deals d JOIN desks k ON k.id = d.desk_id
    WHERE d.status <> 'CANCELLED' AND d.merged_into IS NULL
)
SELECT id AS deal_id, deal_ref, desk_id, desk_code, value_date AS flow_date, ccy1::text AS ccy,
       CASE WHEN side = 'BUY' THEN notional ELSE -notional END AS amount,
       'FX_CCY1' AS leg, confirmed, status, match_status
FROM live WHERE product IN ('FX_SPOT', 'FX_FWD')
UNION ALL
SELECT id, deal_ref, desk_id, desk_code, value_date, ccy2::text,
       CASE WHEN side = 'BUY' THEN -(notional * rate) ELSE (notional * rate) END,
       'FX_CCY2', confirmed, status, match_status
FROM live WHERE product IN ('FX_SPOT', 'FX_FWD') AND ccy2 IS NOT NULL
UNION ALL
SELECT id, deal_ref, desk_id, desk_code, value_date, ccy1::text,
       CASE WHEN side = 'LEND' THEN -notional ELSE notional END,
       'MM_START', confirmed, status, match_status
FROM live WHERE product IN ('MM_DEPO', 'MM_LOAN')
UNION ALL
SELECT id, deal_ref, desk_id, desk_code, maturity_date, ccy1::text,
       CASE WHEN side = 'LEND' THEN 1 ELSE -1 END
         * (notional + notional * rate / 100 * (maturity_date - value_date)
            / CASE WHEN ccy1 = 'GBP' THEN 365 ELSE 360 END),
       'MM_END', confirmed, status, match_status
FROM live WHERE product IN ('MM_DEPO', 'MM_LOAN') AND maturity_date IS NOT NULL;

GRANT SELECT ON cash_flows_v TO treasury_app, treasury_recon;

-- ---------------------------------------------------------------------------
-- Nostro accounts, statements and movements. Bank-wide data: visible to users entitled to the
-- ALM or HEAD desk; written by the reconciler (synthetic for now, SWIFT later).
-- ---------------------------------------------------------------------------
CREATE TABLE nostro_accounts (
    id          smallserial PRIMARY KEY,
    ccy         char(3) NOT NULL UNIQUE,
    bank        text NOT NULL,
    account_no  text NOT NULL
);
INSERT INTO nostro_accounts (ccy, bank, account_no) VALUES
    ('BHD', 'Central Bank of Bahrain',  'CBB-NBB-001'),
    ('USD', 'JPMorgan Chase New York',  'JPM-NBB-USD'),
    ('EUR', 'Deutsche Bank Frankfurt',  'DB-NBB-EUR'),
    ('GBP', 'HSBC London',              'HSBC-NBB-GBP'),
    ('SAR', 'Riyad Bank',               'RB-NBB-SAR'),
    ('AED', 'Emirates NBD',             'ENBD-NBB-AED');

CREATE TABLE nostro_statements (
    id               bigserial PRIMARY KEY,
    account_id       smallint NOT NULL REFERENCES nostro_accounts(id),
    statement_date   date NOT NULL,
    kind             text NOT NULL CHECK (kind IN ('MT940', 'MT942')),
    opening_balance  numeric(20, 2) NOT NULL,
    closing_balance  numeric(20, 2) NULL,
    received_at      timestamptz NOT NULL,
    UNIQUE (account_id, statement_date, kind, received_at)
);
CREATE INDEX nostro_statements_acct_date ON nostro_statements (account_id, statement_date DESC, received_at DESC);

CREATE TABLE nostro_movements (
    id             bigserial PRIMARY KEY,
    account_id     smallint NOT NULL REFERENCES nostro_accounts(id),
    value_date     date NOT NULL,
    booked_at      timestamptz NOT NULL,
    amount         numeric(20, 2) NOT NULL,
    reference      text NOT NULL,
    counterparty   text NULL,
    deal_ref_hint  text NULL,
    source         text NOT NULL DEFAULT 'MT942',
    UNIQUE (account_id, reference)
);
CREATE INDEX nostro_movements_acct_date ON nostro_movements (account_id, value_date);

CREATE FUNCTION can_see_liquidity() RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM user_desk_access a JOIN desks k ON k.id = a.desk_id
        WHERE a.user_id = current_app_user() AND k.code IN ('ALM', 'HEAD')
    )
$$;

ALTER TABLE nostro_statements ENABLE ROW LEVEL SECURITY;
ALTER TABLE nostro_statements FORCE ROW LEVEL SECURITY;
CREATE POLICY ns_admin ON nostro_statements TO CURRENT_USER USING (true) WITH CHECK (true);
CREATE POLICY ns_recon ON nostro_statements TO treasury_recon USING (true) WITH CHECK (true);
CREATE POLICY ns_read ON nostro_statements FOR SELECT TO treasury_app USING (can_see_liquidity());

ALTER TABLE nostro_movements ENABLE ROW LEVEL SECURITY;
ALTER TABLE nostro_movements FORCE ROW LEVEL SECURITY;
CREATE POLICY nm_admin ON nostro_movements TO CURRENT_USER USING (true) WITH CHECK (true);
CREATE POLICY nm_recon ON nostro_movements TO treasury_recon USING (true) WITH CHECK (true);
CREATE POLICY nm_read ON nostro_movements FOR SELECT TO treasury_app USING (can_see_liquidity());

GRANT SELECT ON nostro_accounts TO treasury_app, treasury_recon;
GRANT SELECT ON nostro_statements, nostro_movements TO treasury_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON nostro_statements, nostro_movements TO treasury_recon;
GRANT USAGE ON SEQUENCE nostro_statements_id_seq, nostro_movements_id_seq TO treasury_recon;

-- ---------------------------------------------------------------------------
-- Amendments: only internal bookings, only with booking rights on the desk. External-origin deals
-- belong to the reconciler.
-- ---------------------------------------------------------------------------
DROP POLICY deals_amend ON deals;
CREATE POLICY deals_amend ON deals FOR UPDATE TO treasury_app
    USING (origin = 'INTERNAL' AND EXISTS (
        SELECT 1 FROM user_desk_access
        WHERE user_id = current_app_user() AND desk_id = deals.desk_id AND can_book
    ))
    WITH CHECK (origin = 'INTERNAL' AND EXISTS (
        SELECT 1 FROM user_desk_access
        WHERE user_id = current_app_user() AND desk_id = deals.desk_id AND can_book
    ));

-- ---------------------------------------------------------------------------
-- Deal history, readable through a definer function that applies desk entitlement.
-- ---------------------------------------------------------------------------
CREATE FUNCTION deal_history_for(p_deal uuid)
    RETURNS TABLE (version integer, op text, changed_at timestamptz, changed_by uuid, changed_by_name text,
                   changed_by_role text, snapshot jsonb)
    LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT h.version, h.op, h.changed_at, h.changed_by, u.display_name, h.changed_by_role, h.snapshot
    FROM deal_history h
    JOIN deals d ON d.id = h.deal_id
    LEFT JOIN users u ON u.id = h.changed_by
    WHERE h.deal_id = p_deal AND can_see_desk(d.desk_id)
    ORDER BY h.version
$$;
REVOKE EXECUTE ON FUNCTION deal_history_for(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION deal_history_for(uuid) TO treasury_app;
