-- Phase 2F: market risk of the dealing book and the ALCO pack. The FX limits are a versioned
-- document like the ALM parameters, set by the treasury head; a breach of a limit on a day can be
-- signed off with a note; the monthly ALCO pack carries commentary per section and a sign-off.
-- Risk figures themselves are computed by the server from the deals and market data already held,
-- so nothing here stores a position or a P&L.

-- ---------------------------------------------------------------------------
-- Who may set limits and sign off: the treasury head's desk.
-- ---------------------------------------------------------------------------
CREATE FUNCTION is_treasury_head() RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM user_desk_access a JOIN desks k ON k.id = a.desk_id
        WHERE a.user_id = current_app_user() AND k.code = 'HEAD'
    )
$$;

-- ---------------------------------------------------------------------------
-- The FX limits, versioned. Exactly one active version; version 1 is an empty document, which
-- reads as the application's placeholder limits until the board-approved ones are entered.
-- ---------------------------------------------------------------------------
CREATE TABLE risk_limits (
    version     serial PRIMARY KEY,
    created_at  timestamptz NOT NULL DEFAULT now(),
    created_by  uuid NULL REFERENCES users(id),
    note        text NULL,
    limits      jsonb NOT NULL,
    is_active   boolean NOT NULL DEFAULT false
);
CREATE UNIQUE INDEX risk_limits_active ON risk_limits (is_active) WHERE is_active;
INSERT INTO risk_limits (note, limits, is_active)
VALUES ('Placeholder limits until the board-approved FX limits and counterparty lines are entered', '{}'::jsonb, true);

-- A day a limit stood breached, signed off by the treasury head with a note.
CREATE TABLE limit_signoffs (
    day        date NOT NULL,
    key        text NOT NULL CHECK (length(key) BETWEEN 1 AND 200),
    note       text NOT NULL CHECK (length(note) BETWEEN 1 AND 500),
    signed_by  uuid NOT NULL REFERENCES users(id),
    signed_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (day, key)
);

-- The ALCO pack: commentary per section per month, and the month's sign-off.
CREATE TABLE alco_notes (
    period      date NOT NULL CHECK (extract(day FROM period) = 1),
    section     text NOT NULL CHECK (length(section) BETWEEN 1 AND 60),
    body        text NOT NULL CHECK (length(body) <= 4000),
    updated_by  uuid NOT NULL REFERENCES users(id),
    updated_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (period, section)
);
CREATE TABLE alco_signoffs (
    period     date PRIMARY KEY CHECK (extract(day FROM period) = 1),
    note       text NULL CHECK (length(note) <= 500),
    signed_by  uuid NOT NULL REFERENCES users(id),
    signed_at  timestamptz NOT NULL DEFAULT now()
);

-- ---------------------------------------------------------------------------
-- Row-level security. Every signed-in user reads the limits (the dealers work to them) and the
-- sign-offs; the ALCO material is for the ALM desk and the head. Writes only through the definer
-- functions below.
-- ---------------------------------------------------------------------------
ALTER TABLE risk_limits ENABLE ROW LEVEL SECURITY;
ALTER TABLE risk_limits FORCE ROW LEVEL SECURITY;
ALTER TABLE limit_signoffs ENABLE ROW LEVEL SECURITY;
ALTER TABLE limit_signoffs FORCE ROW LEVEL SECURITY;
ALTER TABLE alco_notes ENABLE ROW LEVEL SECURITY;
ALTER TABLE alco_notes FORCE ROW LEVEL SECURITY;
ALTER TABLE alco_signoffs ENABLE ROW LEVEL SECURITY;
ALTER TABLE alco_signoffs FORCE ROW LEVEL SECURITY;

CREATE POLICY rl_admin ON risk_limits TO CURRENT_USER USING (true) WITH CHECK (true);
CREATE POLICY rl_read ON risk_limits FOR SELECT TO treasury_app USING (current_app_user() IS NOT NULL);
CREATE POLICY ls_admin ON limit_signoffs TO CURRENT_USER USING (true) WITH CHECK (true);
CREATE POLICY ls_read ON limit_signoffs FOR SELECT TO treasury_app USING (current_app_user() IS NOT NULL);
CREATE POLICY an_admin ON alco_notes TO CURRENT_USER USING (true) WITH CHECK (true);
CREATE POLICY an_read ON alco_notes FOR SELECT TO treasury_app USING (can_see_alm());
CREATE POLICY as_admin ON alco_signoffs TO CURRENT_USER USING (true) WITH CHECK (true);
CREATE POLICY as_read ON alco_signoffs FOR SELECT TO treasury_app USING (can_see_alm());

GRANT SELECT ON risk_limits, limit_signoffs, alco_notes, alco_signoffs TO treasury_app;

-- ---------------------------------------------------------------------------
-- Saving a new limits version: the treasury head only, with a note, swapping the active flag under
-- a row lock. The server validates the figures before it calls this.
-- ---------------------------------------------------------------------------
CREATE FUNCTION risk_limits_save(p_limits jsonb, p_note text) RETURNS integer
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    u  uuid := current_app_user();
    v  integer;
BEGIN
    IF u IS NULL OR NOT is_treasury_head() THEN
        RAISE EXCEPTION 'only the treasury head may change the FX limits' USING ERRCODE = '42501';
    END IF;
    IF p_limits IS NULL OR jsonb_typeof(p_limits) <> 'object' THEN
        RAISE EXCEPTION 'limits must be a JSON object';
    END IF;
    IF coalesce(length(trim(p_note)), 0) = 0 THEN
        RAISE EXCEPTION 'a note explaining the change is required';
    END IF;
    PERFORM 1 FROM risk_limits WHERE is_active FOR UPDATE;
    UPDATE risk_limits SET is_active = false WHERE is_active;
    INSERT INTO risk_limits (created_by, note, limits, is_active) VALUES (u, trim(p_note), p_limits, true)
    RETURNING version INTO v;
    PERFORM pg_notify('alm_events', json_build_object('version', NULL, 'snapshot', NULL)::text);
    RETURN v;
END
$$;
REVOKE EXECUTE ON FUNCTION risk_limits_save(jsonb, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION risk_limits_save(jsonb, text) TO treasury_app;

CREATE FUNCTION limit_signoff(p_day date, p_key text, p_note text) RETURNS void
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
    IF current_app_user() IS NULL OR NOT is_treasury_head() THEN
        RAISE EXCEPTION 'only the treasury head may sign off a limit breach' USING ERRCODE = '42501';
    END IF;
    IF coalesce(length(trim(p_note)), 0) = 0 THEN
        RAISE EXCEPTION 'a note is required';
    END IF;
    INSERT INTO limit_signoffs (day, key, note, signed_by) VALUES (p_day, p_key, trim(p_note), current_app_user())
    ON CONFLICT (day, key) DO UPDATE SET note = excluded.note, signed_by = excluded.signed_by, signed_at = now();
END
$$;
REVOKE EXECUTE ON FUNCTION limit_signoff(date, text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION limit_signoff(date, text, text) TO treasury_app;

CREATE FUNCTION alco_note_save(p_period date, p_section text, p_body text) RETURNS void
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
    IF current_app_user() IS NULL OR NOT can_see_alm() THEN
        RAISE EXCEPTION 'not entitled to the ALCO pack' USING ERRCODE = '42501';
    END IF;
    IF EXISTS (SELECT 1 FROM alco_signoffs WHERE period = p_period) THEN
        RAISE EXCEPTION 'the pack for this month is signed off; its commentary is final';
    END IF;
    INSERT INTO alco_notes (period, section, body, updated_by) VALUES (p_period, p_section, coalesce(p_body, ''), current_app_user())
    ON CONFLICT (period, section) DO UPDATE SET body = excluded.body, updated_by = excluded.updated_by, updated_at = now();
END
$$;
REVOKE EXECUTE ON FUNCTION alco_note_save(date, text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION alco_note_save(date, text, text) TO treasury_app;

CREATE FUNCTION alco_sign(p_period date, p_note text) RETURNS void
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
    IF current_app_user() IS NULL OR NOT is_treasury_head() THEN
        RAISE EXCEPTION 'only the treasury head may sign off the ALCO pack' USING ERRCODE = '42501';
    END IF;
    INSERT INTO alco_signoffs (period, note, signed_by) VALUES (p_period, nullif(trim(p_note), ''), current_app_user());
END
$$;
REVOKE EXECUTE ON FUNCTION alco_sign(date, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION alco_sign(date, text) TO treasury_app;
