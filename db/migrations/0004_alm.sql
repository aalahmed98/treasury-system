-- Phase 2C: the banking book for the ALM workbench (synthetic cohorts until the bank's own extracts
-- arrive), market curves and spots, the bank's versioned parameter set, actual and budget net
-- interest income by month, and a desk-scoped activity feed.

-- ---------------------------------------------------------------------------
-- Entitlement: bank-wide ALM data is for the ALM desk and the treasury head. Kept separate from
-- can_see_liquidity() so the two can diverge later.
-- ---------------------------------------------------------------------------
CREATE FUNCTION can_see_alm() RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM user_desk_access a JOIN desks k ON k.id = a.desk_id
        WHERE a.user_id = current_app_user() AND k.code IN ('ALM', 'HEAD')
    )
$$;

-- ---------------------------------------------------------------------------
-- Balance-sheet snapshots. A snapshot is visible to the calculators only once `complete` is set,
-- which the generator does last, so a half-written snapshot is never computed on.
-- ---------------------------------------------------------------------------
CREATE TABLE bs_snapshots (
    as_of          date PRIMARY KEY,
    business_date  date NOT NULL,
    generated_at   timestamptz NOT NULL,
    seed           bigint NOT NULL,
    complete       boolean NOT NULL DEFAULT false
);

CREATE TABLE bs_positions (
    as_of               date NOT NULL REFERENCES bs_snapshots(as_of) ON DELETE CASCADE,
    position_id         text NOT NULL,
    side                text NOT NULL CHECK (side IN ('ASSET', 'LIABILITY', 'OFF_BALANCE')),
    book                text NOT NULL,
    segment             text NOT NULL,
    product             text NOT NULL,
    ccy                 char(3) NOT NULL,
    balance             numeric(20, 2) NOT NULL,
    rate                numeric(10, 6) NOT NULL,           -- percent, like deals.rate
    rate_type           text NOT NULL CHECK (rate_type IN ('FIXED', 'FLOAT', 'ADMIN', 'NONE')),
    index_code          text NULL,
    spread              numeric(10, 6) NOT NULL DEFAULT 0,  -- percent over the index
    reset_months        smallint NULL,
    pay_months          smallint NULL,                      -- NULL: everything at maturity
    amort               text NOT NULL DEFAULT 'BULLET' CHECK (amort IN ('BULLET', 'LINEAR', 'ANNUITY')),
    next_repricing      date NULL,
    maturity            date NULL,                          -- NULL: non-maturing
    origination         date NULL,
    day_count           text NOT NULL DEFAULT 'ACT/360' CHECK (day_count IN ('ACT/360', 'ACT/365')),
    hqla_level          text NULL CHECK (hqla_level IN ('L1', 'L2A', 'L2B')),
    encumbered          boolean NOT NULL DEFAULT false,
    lcr_class           text NULL,
    nsfr_class          text NULL,                          -- type only; the maturity bucket is derived
    beta                numeric(4, 3) NOT NULL DEFAULT 0,
    behavioural_months  smallint NULL,
    undrawn             numeric(20, 2) NOT NULL DEFAULT 0,
    PRIMARY KEY (as_of, position_id)
);
CREATE INDEX bs_positions_ccy ON bs_positions (as_of, ccy);

CREATE TABLE bs_depositors (
    as_of       date NOT NULL REFERENCES bs_snapshots(as_of) ON DELETE CASCADE,
    group_name  text NOT NULL,
    segment     text NOT NULL,
    ccy         char(3) NOT NULL,
    balance     numeric(20, 2) NOT NULL,
    PRIMARY KEY (as_of, group_name, ccy)
);

-- ---------------------------------------------------------------------------
-- Market data: daily curve closes, the live curve updated in place, and spots as units of the
-- reporting currency per one unit of the quoted currency ('USD/BHD' = BHD per USD). Visible to
-- every desk; written by the reconciler's generator (Bloomberg later).
-- ---------------------------------------------------------------------------
CREATE TABLE market_rates (
    as_of       date NOT NULL,
    ccy         char(3) NOT NULL,
    index_code  text NOT NULL,
    tenor       text NOT NULL,
    tenor_days  integer NOT NULL,
    rate        numeric(10, 6) NOT NULL,
    PRIMARY KEY (as_of, ccy, index_code, tenor)
);
CREATE TABLE market_rates_live (
    ccy         char(3) NOT NULL,
    index_code  text NOT NULL,
    tenor       text NOT NULL,
    tenor_days  integer NOT NULL,
    rate        numeric(10, 6) NOT NULL,
    as_of       timestamptz NOT NULL,
    PRIMARY KEY (ccy, index_code, tenor)
);
CREATE TABLE fx_spots (
    as_of  date NOT NULL,
    pair   text NOT NULL,
    mid    numeric(16, 8) NOT NULL,
    PRIMARY KEY (as_of, pair)
);

-- ---------------------------------------------------------------------------
-- The bank's limits and assumptions, versioned. Exactly one active version. Version 1 holds the
-- Basel III defaults the application ships with (an empty document means "all defaults").
-- ---------------------------------------------------------------------------
CREATE TABLE alm_parameters (
    version     serial PRIMARY KEY,
    created_at  timestamptz NOT NULL DEFAULT now(),
    created_by  uuid NULL REFERENCES users(id),
    note        text NULL,
    params      jsonb NOT NULL,
    is_active   boolean NOT NULL DEFAULT false
);
CREATE UNIQUE INDEX alm_parameters_active ON alm_parameters (is_active) WHERE is_active;
INSERT INTO alm_parameters (note, params, is_active)
VALUES ('Basel III defaults; the CBB LM-11 and LM-12 calibration and the bank''s own limits are to be confirmed', '{}'::jsonb, true);

-- Actual and budget net interest income per month, book, side and currency, with the average
-- balance so a rate can be derived and attribution closes exactly.
CREATE TABLE alm_period (
    period       date NOT NULL,
    book         text NOT NULL,
    side         text NOT NULL CHECK (side IN ('ASSET', 'LIABILITY')),
    ccy          char(3) NOT NULL,
    kind         text NOT NULL CHECK (kind IN ('ACTUAL', 'BUDGET')),
    nii          numeric(20, 2) NOT NULL,
    avg_balance  numeric(20, 2) NOT NULL,
    PRIMARY KEY (period, book, side, ccy, kind)
);

-- ---------------------------------------------------------------------------
-- Row-level security: the API role reads ALM data only when entitled; the reconciler writes the
-- generated tables and never touches the parameter set.
-- ---------------------------------------------------------------------------
ALTER TABLE bs_snapshots ENABLE ROW LEVEL SECURITY;
ALTER TABLE bs_snapshots FORCE ROW LEVEL SECURITY;
CREATE POLICY bss_admin ON bs_snapshots TO CURRENT_USER USING (true) WITH CHECK (true);
CREATE POLICY bss_recon ON bs_snapshots TO treasury_recon USING (true) WITH CHECK (true);
CREATE POLICY bss_read ON bs_snapshots FOR SELECT TO treasury_app USING (can_see_alm());

ALTER TABLE bs_positions ENABLE ROW LEVEL SECURITY;
ALTER TABLE bs_positions FORCE ROW LEVEL SECURITY;
CREATE POLICY bsp_admin ON bs_positions TO CURRENT_USER USING (true) WITH CHECK (true);
CREATE POLICY bsp_recon ON bs_positions TO treasury_recon USING (true) WITH CHECK (true);
CREATE POLICY bsp_read ON bs_positions FOR SELECT TO treasury_app USING (can_see_alm());

ALTER TABLE bs_depositors ENABLE ROW LEVEL SECURITY;
ALTER TABLE bs_depositors FORCE ROW LEVEL SECURITY;
CREATE POLICY bsd_admin ON bs_depositors TO CURRENT_USER USING (true) WITH CHECK (true);
CREATE POLICY bsd_recon ON bs_depositors TO treasury_recon USING (true) WITH CHECK (true);
CREATE POLICY bsd_read ON bs_depositors FOR SELECT TO treasury_app USING (can_see_alm());

ALTER TABLE alm_period ENABLE ROW LEVEL SECURITY;
ALTER TABLE alm_period FORCE ROW LEVEL SECURITY;
CREATE POLICY ap_admin ON alm_period TO CURRENT_USER USING (true) WITH CHECK (true);
CREATE POLICY ap_recon ON alm_period TO treasury_recon USING (true) WITH CHECK (true);
CREATE POLICY ap_read ON alm_period FOR SELECT TO treasury_app USING (can_see_alm());

ALTER TABLE alm_parameters ENABLE ROW LEVEL SECURITY;
ALTER TABLE alm_parameters FORCE ROW LEVEL SECURITY;
CREATE POLICY alp_admin ON alm_parameters TO CURRENT_USER USING (true) WITH CHECK (true);
CREATE POLICY alp_read ON alm_parameters FOR SELECT TO treasury_app USING (can_see_alm());

GRANT SELECT ON bs_snapshots, bs_positions, bs_depositors, alm_parameters, alm_period, market_rates, market_rates_live, fx_spots TO treasury_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON bs_snapshots, bs_positions, bs_depositors, alm_period, market_rates, market_rates_live, fx_spots TO treasury_recon;

-- ---------------------------------------------------------------------------
-- Saving a new parameter version: any user entitled to ALM, through a definer function that
-- validates the document, swaps the active flag under a row lock and tells open clients.
-- ---------------------------------------------------------------------------
CREATE FUNCTION alm_parameters_save(p_params jsonb, p_note text) RETURNS integer
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    u    uuid := current_app_user();
    v    integer;
    k    text;
    val  numeric;
BEGIN
    IF u IS NULL OR NOT can_see_alm() THEN
        RAISE EXCEPTION 'not entitled to change ALM parameters' USING ERRCODE = '42501';
    END IF;
    IF p_params IS NULL OR jsonb_typeof(p_params) <> 'object' THEN
        RAISE EXCEPTION 'parameters must be a JSON object';
    END IF;
    FOREACH k IN ARRAY ARRAY['lcr', 'nsfr', 'gap', 'stress', 'nii', 'funding'] LOOP
        IF NOT (p_params ? k) OR jsonb_typeof(p_params -> k) <> 'object' THEN
            RAISE EXCEPTION 'parameters are missing the % section', k;
        END IF;
    END LOOP;
    FOR k, val IN SELECT e.key, e.value::numeric FROM jsonb_each_text(coalesce(p_params -> 'lcr' -> 'outflow_rates', '{}'::jsonb)) e LOOP
        IF val < 0 OR val > 100 THEN
            RAISE EXCEPTION 'outflow rate % must be between 0 and 100', k;
        END IF;
    END LOOP;
    FOR k, val IN SELECT e.key, e.value::numeric FROM jsonb_each_text(coalesce(p_params -> 'lcr' -> 'inflow_rates', '{}'::jsonb)) e LOOP
        IF val < 0 OR val > 100 THEN
            RAISE EXCEPTION 'inflow rate % must be between 0 and 100', k;
        END IF;
    END LOOP;
    IF coalesce(length(trim(p_note)), 0) = 0 THEN
        RAISE EXCEPTION 'a note explaining the change is required';
    END IF;
    PERFORM 1 FROM alm_parameters WHERE is_active FOR UPDATE;
    UPDATE alm_parameters SET is_active = false WHERE is_active;
    INSERT INTO alm_parameters (created_by, note, params, is_active) VALUES (u, trim(p_note), p_params, true)
    RETURNING version INTO v;
    PERFORM pg_notify('alm_events', json_build_object('version', v)::text);
    RETURN v;
END
$$;
REVOKE EXECUTE ON FUNCTION alm_parameters_save(jsonb, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION alm_parameters_save(jsonb, text) TO treasury_app;

-- ---------------------------------------------------------------------------
-- Recent activity for the caller's desks: bookings, amendments and cancellations by people,
-- breaks detected, break actions, and parameter versions. The audit and history tables stay closed
-- to the API role; this definer function is the only way through.
-- ---------------------------------------------------------------------------
CREATE INDEX deal_history_changed_at ON deal_history (changed_at DESC);

CREATE FUNCTION recent_activity(p_limit integer)
    RETURNS TABLE (at timestamptz, kind text, actor text, summary text, deal_id uuid, break_id bigint, desk_code text)
    LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT x.at, x.kind, x.actor, x.summary, x.deal_id, x.break_id, x.desk_code FROM (
        (SELECT h.changed_at AS at,
                CASE WHEN h.op = 'INSERT' THEN 'booked'
                     WHEN h.snapshot ->> 'status' = 'CANCELLED' THEN 'cancelled'
                     ELSE 'amended' END AS kind,
                coalesce(u.display_name, h.changed_by_role) AS actor,
                d.deal_ref || ' ' || d.product || ' ' || d.ccy1::text || coalesce('/' || d.ccy2::text, '') || ' ' || d.notional::text AS summary,
                d.id AS deal_id, NULL::bigint AS break_id, k.code AS desk_code
         FROM deal_history h
         JOIN deals d ON d.id = h.deal_id
         JOIN desks k ON k.id = d.desk_id
         LEFT JOIN users u ON u.id = h.changed_by
         WHERE h.changed_by IS NOT NULL AND can_see_desk(d.desk_id)
         ORDER BY h.changed_at DESC LIMIT p_limit)
        UNION ALL
        (SELECT b.detected_at, 'break ' || lower(b.kind), 'reconciler',
                '#' || b.id::text || ' ' || coalesce(d.deal_ref, coalesce(b.source, '') || ' ' || coalesce(b.source_ref, '')) || coalesce(' on ' || b.field, ''),
                b.deal_id, b.id, k.code
         FROM breaks b
         JOIN desks k ON k.id = b.desk_id
         LEFT JOIN deals d ON d.id = b.deal_id
         WHERE can_see_desk(b.desk_id)
         ORDER BY b.detected_at DESC LIMIT p_limit)
        UNION ALL
        (SELECT a.at, 'break ' || lower(a.action), coalesce(u.display_name, 'user'),
                '#' || a.break_id::text || coalesce(' ' || d.deal_ref, '') || coalesce(': ' || a.note, ''),
                b.deal_id, a.break_id, k.code
         FROM break_actions a
         JOIN breaks b ON b.id = a.break_id
         JOIN desks k ON k.id = a.desk_id
         LEFT JOIN deals d ON d.id = b.deal_id
         LEFT JOIN users u ON u.id = a.user_id
         WHERE can_see_desk(a.desk_id)
         ORDER BY a.at DESC LIMIT p_limit)
        UNION ALL
        (SELECT p.created_at, 'parameters', coalesce(u.display_name, 'system'),
                'ALM parameters v' || p.version::text || coalesce(': ' || p.note, ''),
                NULL::uuid, NULL::bigint, 'ALM'
         FROM alm_parameters p
         LEFT JOIN users u ON u.id = p.created_by
         WHERE can_see_alm()
         ORDER BY p.created_at DESC LIMIT p_limit)
    ) x
    ORDER BY x.at DESC
    LIMIT p_limit
$$;
REVOKE EXECUTE ON FUNCTION recent_activity(integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION recent_activity(integer) TO treasury_app;
