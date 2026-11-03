-- Phase 2: reconciliation core. An append-only log of source observations, per-source state with links
-- to golden deals, breaks, feed freshness, and the reconciler's own least-privilege role.
--
-- Runs as treasury_owner. The reconciler connects as treasury_recon (owns nothing, NOBYPASSRLS, explicit
-- policies). The API role treasury_app can only read reconciliation data and append user actions through
-- the definer-rights functions at the bottom.

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'treasury_recon') THEN
        RAISE EXCEPTION 'role treasury_recon must exist before this migration runs (db/init/01-roles.sh creates it for Docker; operations create it elsewhere)';
    END IF;
END
$$;

-- ---------------------------------------------------------------------------
-- Reference data
-- ---------------------------------------------------------------------------
INSERT INTO desks (code, name) VALUES ('UNASSIGNED', 'Unmapped source rows');

CREATE TABLE source_systems (
    code           text PRIMARY KEY,
    name           text NOT NULL,
    may_originate  boolean NOT NULL,   -- may create a provisional golden deal
    confirms       boolean NOT NULL,   -- its CONFIRMED state sets deals.confirmed
    cancels        boolean NOT NULL,   -- its CANCEL cancels the golden deal
    precedence     smallint NOT NULL UNIQUE  -- field ownership: lowest linked source owns the economics
);
INSERT INTO source_systems (code, name, may_originate, confirms, cancels, precedence) VALUES
    ('KONDOR',    'Finastra Fusion Kondor', true,  false, true,  1),
    ('BLOOMBERG', 'Bloomberg',              true,  false, false, 2),
    ('QUANTUM',   'FIS Quantum',            false, true,  false, 3),
    ('INTERNAL',  'Internal booking',       true,  false, false, 4),
    ('USER',      'User action',            false, false, false, 9);

-- Source-side book / portfolio / account codes to desks. Unmapped codes land on UNASSIGNED.
CREATE TABLE desk_mappings (
    source         text NOT NULL REFERENCES source_systems(code),
    external_code  text NOT NULL,
    desk_id        smallint NOT NULL REFERENCES desks(id),
    PRIMARY KEY (source, external_code)
);
INSERT INTO desk_mappings (source, external_code, desk_id)
SELECT m.source, m.code, k.id
FROM (VALUES
    ('KONDOR', 'FX-BOOK', 'FX'), ('KONDOR', 'MM-BOOK', 'MM'),
    ('BLOOMBERG', 'NBB-FX', 'FX'), ('BLOOMBERG', 'NBB-MM', 'MM'),
    ('QUANTUM', 'TRS-FX', 'FX'), ('QUANTUM', 'TRS-MM', 'MM')
) AS m(source, code, desk)
JOIN desks k ON k.code = m.desk;

-- Normalised alias -> normalised canonical counterparty name.
CREATE TABLE counterparty_aliases (
    alias_norm  text PRIMARY KEY,
    canonical   text NOT NULL
);
INSERT INTO counterparty_aliases (alias_norm, canonical) VALUES
    ('GIB', 'GULF INTERNATIONAL BANK'), ('GULF INTL BANK', 'GULF INTERNATIONAL BANK'),
    ('ABC', 'ARAB BANKING CORPORATION'), ('BANK ABC', 'ARAB BANKING CORPORATION'),
    ('BANK OF BAHRAIN AND KUWAIT', 'BBK'), ('BANK OF BAHRAIN KUWAIT', 'BBK'),
    ('AUB', 'AHLI UNITED BANK'), ('AHLI UNITED', 'AHLI UNITED BANK'),
    ('AL SALAM', 'AL SALAM BANK'), ('ALSALAM BANK', 'AL SALAM BANK'),
    ('HSBC', 'HSBC BAHRAIN'), ('HSBC BANK MIDDLE EAST', 'HSBC BAHRAIN'),
    ('SCB', 'STANDARD CHARTERED'), ('STANCHART', 'STANDARD CHARTERED'), ('STANDARD CHARTERED BANK', 'STANDARD CHARTERED'),
    ('CITI', 'CITIBANK'), ('CITIBANK NA', 'CITIBANK'),
    ('JPM', 'JPMORGAN CHASE'), ('JP MORGAN', 'JPMORGAN CHASE'), ('JPMORGAN', 'JPMORGAN CHASE'),
    ('ENBD', 'EMIRATES NBD'), ('EMIRATES NBD BANK', 'EMIRATES NBD'),
    ('ALRAJHI', 'AL RAJHI BANK'), ('AL RAJHI', 'AL RAJHI BANK'),
    ('QNB', 'QATAR NATIONAL BANK');

-- Expected lag from a deal being seen in one source to being seen in another.
CREATE TABLE sla_expectations (
    from_source      text NOT NULL REFERENCES source_systems(code),
    to_source        text NOT NULL REFERENCES source_systems(code),
    product          text NOT NULL DEFAULT '*',
    expected_within  interval NOT NULL,
    calendar         text NOT NULL DEFAULT 'CLOCK' CHECK (calendar IN ('CLOCK', 'BUSINESS_DAYS')),
    grace            interval NOT NULL DEFAULT interval '60 seconds',
    PRIMARY KEY (from_source, to_source, product)
);
INSERT INTO sla_expectations (from_source, to_source, expected_within) VALUES
    ('BLOOMBERG', 'KONDOR',  interval '5 minutes'),
    ('INTERNAL',  'KONDOR',  interval '10 minutes'),
    ('KONDOR',    'QUANTUM', interval '30 minutes');

CREATE TABLE calendar_holidays (
    day   date PRIMARY KEY,
    name  text NOT NULL
);
INSERT INTO calendar_holidays (day, name) VALUES
    ('2026-12-16', 'National Day'), ('2026-12-17', 'National Day'), ('2027-01-01', 'New Year');

ALTER TABLE user_desk_access ADD COLUMN can_resolve boolean NOT NULL DEFAULT false;

-- ---------------------------------------------------------------------------
-- The event log: every observation from every source, as a full image, never updated except
-- for the processing outcome.
-- ---------------------------------------------------------------------------
CREATE TABLE source_events (
    id                  bigserial PRIMARY KEY,
    source              text NOT NULL REFERENCES source_systems(code),
    source_ref          text NOT NULL,
    event_kind          text NOT NULL CHECK (event_kind IN ('NEW', 'AMEND', 'CANCEL', 'STATUS', 'USER')),
    source_seq          bigint NULL,                 -- the source's own ordering when it has one
    source_ts           timestamptz NOT NULL,        -- the source's clock
    observed_at         timestamptz NOT NULL,        -- our clock (or the simulation clock); no default on purpose
    payload             jsonb NOT NULL,              -- full raw image
    norm                jsonb NULL,                  -- normalised deal, filled by the reconciler
    normaliser_version  smallint NOT NULL DEFAULT 0,
    desk_id             smallint NOT NULL REFERENCES desks(id),
    dedup_hash          bytea NOT NULL,              -- sha256(source|ref|kind|payload), computed by the inserter
    processed_at        timestamptz NULL,
    outcome             text NULL,                   -- LINKED CREATED AMENDED CANCELLED MERGED STALE DUP BREAK APPLIED ERROR
    outcome_detail      jsonb NULL,
    UNIQUE (source, dedup_hash)
);
CREATE INDEX source_events_pending ON source_events (id) WHERE processed_at IS NULL;
CREATE INDEX source_events_ref ON source_events (source, source_ref, id);
CREATE INDEX source_events_fresh ON source_events (source, observed_at DESC);

CREATE FUNCTION source_events_notify() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_notify('source_events_new', '');
    RETURN NULL;
END
$$;
CREATE TRIGGER source_events_notify
    AFTER INSERT ON source_events FOR EACH STATEMENT EXECUTE FUNCTION source_events_notify();

-- ---------------------------------------------------------------------------
-- Latest state per (source, source_ref) and its link to a golden deal
-- ---------------------------------------------------------------------------
CREATE TABLE source_deals (
    source            text NOT NULL REFERENCES source_systems(code),
    source_ref        text NOT NULL,
    deal_id           uuid NULL REFERENCES deals(id),
    link_kind         text NULL CHECK (link_kind IN ('SELF', 'CROSS_REF', 'AUTO', 'MANUAL')),
    link_event_id     bigint NULL,
    linked_by         uuid NULL,
    linked_at         timestamptz NULL,
    link_score        smallint NULL,
    lifecycle         text NOT NULL CHECK (lifecycle IN ('EXECUTED', 'BOOKED', 'CONFIRMED', 'SETTLED', 'CANCELLED')),
    raw_state         text NULL,
    applied_event_id  bigint NOT NULL,
    applied_seq       bigint NULL,
    applied_ts        timestamptz NOT NULL,
    first_seen_at     timestamptz NOT NULL,
    desk_id           smallint NOT NULL REFERENCES desks(id),
    cross_refs        jsonb NOT NULL DEFAULT '{}',
    product           text NOT NULL,
    side              text NOT NULL,
    ccy1              char(3) NOT NULL,
    ccy2              char(3) NULL,
    notional          numeric(20, 4) NOT NULL,
    rate              numeric(18, 8) NOT NULL,
    trade_date        date NOT NULL,
    value_date        date NOT NULL,
    maturity_date     date NULL,
    counterparty_raw  text NOT NULL,
    counterparty      text NOT NULL,
    trader_ref        text NULL,
    updated_at        timestamptz NOT NULL,
    PRIMARY KEY (source, source_ref)
);
CREATE UNIQUE INDEX source_deals_one_per_source ON source_deals (deal_id, source) WHERE deal_id IS NOT NULL;
CREATE INDEX source_deals_unlinked ON source_deals (product, ccy1, ccy2, trade_date, notional) WHERE deal_id IS NULL;
CREATE INDEX source_deals_desk ON source_deals (desk_id);

-- ---------------------------------------------------------------------------
-- Golden deals become the consolidated record
-- ---------------------------------------------------------------------------
ALTER TABLE deals
    ALTER COLUMN trader_id DROP NOT NULL,
    ADD COLUMN origin text NOT NULL DEFAULT 'INTERNAL' REFERENCES source_systems(code),
    ADD COLUMN match_status text NOT NULL DEFAULT 'UNMATCHED'
        CHECK (match_status IN ('UNMATCHED', 'PARTIAL', 'MATCHED', 'BREAK')),
    ADD COLUMN confirmed boolean NOT NULL DEFAULT false,
    ADD COLUMN merged_into uuid NULL REFERENCES deals(id),
    ADD CONSTRAINT deals_internal_has_trader CHECK (origin <> 'INTERNAL' OR trader_id IS NOT NULL);
CREATE INDEX deals_match_bucket ON deals (product, ccy1, ccy2, trade_date, notional);
CREATE INDEX deals_match_status ON deals (match_status) WHERE match_status IN ('UNMATCHED', 'PARTIAL');

ALTER TABLE deal_history ADD COLUMN changed_by_role text NOT NULL DEFAULT '';

-- Identity is immutable; terminal deals keep their economics and status but may still be annotated by
-- reconciliation; the reconciler may never change the economics of an internal booking.
CREATE OR REPLACE FUNCTION deals_before_update() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    economics_changed boolean;
BEGIN
    IF NEW.id IS DISTINCT FROM OLD.id OR NEW.deal_ref IS DISTINCT FROM OLD.deal_ref
       OR NEW.desk_id IS DISTINCT FROM OLD.desk_id OR NEW.trader_id IS DISTINCT FROM OLD.trader_id
       OR NEW.created_at IS DISTINCT FROM OLD.created_at OR NEW.origin IS DISTINCT FROM OLD.origin THEN
        RAISE EXCEPTION 'deal identity (id, reference, desk, trader, origin, created_at) cannot be changed';
    END IF;
    economics_changed :=
           NEW.product IS DISTINCT FROM OLD.product OR NEW.side IS DISTINCT FROM OLD.side
        OR NEW.ccy1 IS DISTINCT FROM OLD.ccy1 OR NEW.ccy2 IS DISTINCT FROM OLD.ccy2
        OR NEW.notional IS DISTINCT FROM OLD.notional OR NEW.rate IS DISTINCT FROM OLD.rate
        OR NEW.trade_date IS DISTINCT FROM OLD.trade_date OR NEW.value_date IS DISTINCT FROM OLD.value_date
        OR NEW.maturity_date IS DISTINCT FROM OLD.maturity_date OR NEW.counterparty IS DISTINCT FROM OLD.counterparty;
    IF OLD.status IN ('SETTLED', 'CANCELLED') AND (economics_changed OR NEW.status IS DISTINCT FROM OLD.status) THEN
        RAISE EXCEPTION 'a % deal cannot be amended', lower(OLD.status);
    END IF;
    IF session_user = 'treasury_recon' AND OLD.origin = 'INTERNAL' AND economics_changed THEN
        RAISE EXCEPTION 'reconciliation may not change the economics of an internal booking';
    END IF;
    NEW.version := OLD.version + 1;
    NEW.updated_at := now();
    RETURN NEW;
END
$$;

-- History and notification as before, plus: internal bookings enter the reconciliation log as the
-- "expected deal" anchor. Definer rights, so the booking role needs no access to source_events.
CREATE OR REPLACE FUNCTION deals_after_write() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    v_kind     text;
    v_payload  jsonb;
BEGIN
    INSERT INTO deal_history (deal_id, version, op, changed_by, changed_by_role, snapshot)
    VALUES (NEW.id, NEW.version, TG_OP, current_app_user(), session_user, to_jsonb(NEW));
    PERFORM pg_notify('deal_events', json_build_object(
        'op', TG_OP, 'id', NEW.id, 'desk_id', NEW.desk_id, 'version', NEW.version
    )::text);

    IF NEW.origin = 'INTERNAL' AND session_user <> 'treasury_recon' THEN
        v_kind := CASE
            WHEN TG_OP = 'INSERT' THEN 'NEW'
            WHEN NEW.status = 'CANCELLED' THEN 'CANCEL'
            ELSE 'AMEND'
        END;
        v_payload := to_jsonb(NEW) - 'match_status' - 'confirmed' - 'merged_into' - 'version' - 'updated_at';
        INSERT INTO source_events (source, source_ref, event_kind, source_seq, source_ts, observed_at,
                                   payload, desk_id, dedup_hash)
        VALUES ('INTERNAL', NEW.id::text, v_kind, NEW.version, NEW.updated_at, NEW.updated_at, v_payload, NEW.desk_id,
                sha256(convert_to('INTERNAL|' || NEW.id::text || '|' || v_kind || '|' || v_payload::text, 'UTF8')))
        ON CONFLICT (source, dedup_hash) DO NOTHING;
    END IF;
    RETURN NEW;
END
$$;

-- ---------------------------------------------------------------------------
-- Breaks and the actions taken on them
-- ---------------------------------------------------------------------------
CREATE TABLE breaks (
    id               bigserial PRIMARY KEY,
    kind             text NOT NULL CHECK (kind IN ('MISSING', 'MISMATCH', 'ORPHAN', 'AMBIGUOUS', 'DUPLICATE')),
    deal_id          uuid NULL REFERENCES deals(id),
    source           text NULL REFERENCES source_systems(code),
    source_ref       text NULL,
    field            text NULL,
    desk_id          smallint NOT NULL REFERENCES desks(id),
    details          jsonb NOT NULL DEFAULT '{}',
    status           text NOT NULL DEFAULT 'OPEN' CHECK (status IN ('OPEN', 'ACKNOWLEDGED', 'RESOLVED')),
    detected_at      timestamptz NOT NULL,
    acknowledged_by  uuid NULL,
    acknowledged_at  timestamptz NULL,
    resolved_by      uuid NULL,
    resolved_at      timestamptz NULL,
    resolution       text NULL CHECK (resolution IN ('AUTO', 'LINKED', 'DISMISSED', 'SUPERSEDED')),
    note             text NULL,
    updated_at       timestamptz NOT NULL,
    break_key        text GENERATED ALWAYS AS (
                         kind || ':' || coalesce(deal_id::text, '') || ':' || coalesce(source, '')
                              || ':' || coalesce(source_ref, '') || ':' || coalesce(field, '')
                     ) STORED
);
-- Detection is idempotent: INSERT ... ON CONFLICT (break_key) WHERE status <> 'RESOLVED' DO NOTHING.
CREATE UNIQUE INDEX breaks_open_key ON breaks (break_key) WHERE status <> 'RESOLVED';
CREATE INDEX breaks_open_desk ON breaks (desk_id, detected_at DESC) WHERE status <> 'RESOLVED';
CREATE INDEX breaks_deal ON breaks (deal_id);

CREATE TABLE break_actions (
    id        bigserial PRIMARY KEY,
    break_id  bigint NOT NULL REFERENCES breaks(id),
    desk_id   smallint NOT NULL REFERENCES desks(id),
    at        timestamptz NOT NULL DEFAULT now(),
    user_id   uuid NULL,
    action    text NOT NULL,
    note      text NULL
);
CREATE INDEX break_actions_break ON break_actions (break_id, id);

CREATE FUNCTION breaks_notify() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_notify('break_events', json_build_object(
        'id', NEW.id, 'desk_id', NEW.desk_id, 'deal_id', NEW.deal_id, 'status', NEW.status
    )::text);
    RETURN NEW;
END
$$;
CREATE TRIGGER breaks_notify AFTER INSERT OR UPDATE ON breaks FOR EACH ROW EXECUTE FUNCTION breaks_notify();

-- ---------------------------------------------------------------------------
-- Freshness: adapters heartbeat even when quiet; the worker publishes the clock it runs on.
-- ---------------------------------------------------------------------------
CREATE TABLE feed_status (
    source              text PRIMARY KEY REFERENCES source_systems(code),
    last_heartbeat_at   timestamptz NULL,
    last_event_at       timestamptz NULL,
    last_source_ts      timestamptz NULL,
    events_total        bigint NOT NULL DEFAULT 0,
    duplicates_dropped  bigint NOT NULL DEFAULT 0
);
INSERT INTO feed_status (source) SELECT code FROM source_systems WHERE code <> 'USER';

CREATE TABLE recon_status (
    name          text PRIMARY KEY,
    now           timestamptz NOT NULL,     -- the reconciler's clock (simulation or wall)
    last_tick_at  timestamptz NOT NULL,
    pending       bigint NOT NULL DEFAULT 0,
    detail        jsonb NOT NULL DEFAULT '{}'
);

-- ---------------------------------------------------------------------------
-- Row-level security
-- ---------------------------------------------------------------------------
CREATE FUNCTION can_see_desk(p_desk smallint) RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (SELECT 1 FROM user_desk_access WHERE user_id = current_app_user() AND desk_id = p_desk)
$$;
CREATE FUNCTION can_resolve_desk(p_desk smallint) RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (SELECT 1 FROM user_desk_access WHERE user_id = current_app_user() AND desk_id = p_desk AND can_resolve)
$$;

ALTER TABLE source_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE source_events FORCE ROW LEVEL SECURITY;
CREATE POLICY se_admin ON source_events TO CURRENT_USER USING (true) WITH CHECK (true);
CREATE POLICY se_recon ON source_events TO treasury_recon USING (true) WITH CHECK (true);
CREATE POLICY se_read ON source_events FOR SELECT TO treasury_app USING (can_see_desk(desk_id));

ALTER TABLE source_deals ENABLE ROW LEVEL SECURITY;
ALTER TABLE source_deals FORCE ROW LEVEL SECURITY;
CREATE POLICY sd_admin ON source_deals TO CURRENT_USER USING (true) WITH CHECK (true);
CREATE POLICY sd_recon ON source_deals TO treasury_recon USING (true) WITH CHECK (true);
CREATE POLICY sd_read ON source_deals FOR SELECT TO treasury_app USING (can_see_desk(desk_id));

ALTER TABLE breaks ENABLE ROW LEVEL SECURITY;
ALTER TABLE breaks FORCE ROW LEVEL SECURITY;
CREATE POLICY br_admin ON breaks TO CURRENT_USER USING (true) WITH CHECK (true);
CREATE POLICY br_recon ON breaks TO treasury_recon USING (true) WITH CHECK (true);
CREATE POLICY br_read ON breaks FOR SELECT TO treasury_app USING (can_see_desk(desk_id));

ALTER TABLE break_actions ENABLE ROW LEVEL SECURITY;
ALTER TABLE break_actions FORCE ROW LEVEL SECURITY;
CREATE POLICY ba_admin ON break_actions TO CURRENT_USER USING (true) WITH CHECK (true);
CREATE POLICY ba_recon ON break_actions TO treasury_recon USING (true) WITH CHECK (true);
CREATE POLICY ba_read ON break_actions FOR SELECT TO treasury_app USING (can_see_desk(desk_id));

-- Golden deals: the reconciler sees everything, creates only external-origin deals without a trader,
-- and updates under the trigger's rules. Internal bookings stay exclusive to the booking policy.
CREATE POLICY deals_recon_sel ON deals FOR SELECT TO treasury_recon USING (true);
CREATE POLICY deals_recon_ins ON deals FOR INSERT TO treasury_recon WITH CHECK (origin <> 'INTERNAL' AND trader_id IS NULL);
CREATE POLICY deals_recon_upd ON deals FOR UPDATE TO treasury_recon USING (true) WITH CHECK (true);
CREATE POLICY deals_recon_del ON deals FOR DELETE TO treasury_recon USING (origin <> 'INTERNAL');  -- rebuild only

DROP POLICY deals_book ON deals;
CREATE POLICY deals_book ON deals FOR INSERT TO treasury_app
    WITH CHECK (
        origin = 'INTERNAL'
        AND trader_id = current_app_user()
        AND EXISTS (
            SELECT 1 FROM user_desk_access
            WHERE user_id = current_app_user() AND desk_id = deals.desk_id AND can_book
        )
    );

-- ---------------------------------------------------------------------------
-- User actions on breaks. Acknowledging is direct; resolving is appended to the event log so the
-- reconciler applies it in order and a rebuild replays it.
-- ---------------------------------------------------------------------------
CREATE FUNCTION break_ack(p_break bigint) RETURNS void
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    b  breaks%ROWTYPE;
    u  uuid := current_app_user();
BEGIN
    SELECT * INTO b FROM breaks WHERE id = p_break;
    IF NOT FOUND OR NOT can_see_desk(b.desk_id) THEN
        RAISE EXCEPTION 'break not found' USING ERRCODE = 'P0002';
    END IF;
    IF NOT can_resolve_desk(b.desk_id) THEN
        RAISE EXCEPTION 'not entitled to act on breaks for this desk' USING ERRCODE = '42501';
    END IF;
    IF b.status <> 'OPEN' THEN
        RAISE EXCEPTION 'break is already %', lower(b.status);
    END IF;
    UPDATE breaks SET status = 'ACKNOWLEDGED', acknowledged_by = u, acknowledged_at = now(), updated_at = now()
     WHERE id = p_break;
    INSERT INTO break_actions (break_id, desk_id, user_id, action) VALUES (p_break, b.desk_id, u, 'ACK');
END
$$;

CREATE FUNCTION break_resolve(p_break bigint, p_action text, p_deal uuid, p_note text) RETURNS bigint
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    b         breaks%ROWTYPE;
    u         uuid := current_app_user();
    v_payload jsonb;
    v_id      bigint;
BEGIN
    SELECT * INTO b FROM breaks WHERE id = p_break;
    IF NOT FOUND OR NOT can_see_desk(b.desk_id) THEN
        RAISE EXCEPTION 'break not found' USING ERRCODE = 'P0002';
    END IF;
    IF NOT can_resolve_desk(b.desk_id) THEN
        RAISE EXCEPTION 'not entitled to act on breaks for this desk' USING ERRCODE = '42501';
    END IF;
    IF b.status = 'RESOLVED' THEN
        RAISE EXCEPTION 'break is already resolved';
    END IF;
    IF p_action NOT IN ('LINK', 'DISMISS') THEN
        RAISE EXCEPTION 'action must be LINK or DISMISS';
    END IF;
    IF p_action = 'LINK' THEN
        IF p_deal IS NULL THEN
            RAISE EXCEPTION 'LINK needs a target deal';
        END IF;
        IF NOT EXISTS (SELECT 1 FROM deals d WHERE d.id = p_deal AND can_see_desk(d.desk_id)) THEN
            RAISE EXCEPTION 'target deal not found' USING ERRCODE = 'P0002';
        END IF;
    ELSIF coalesce(length(trim(p_note)), 0) = 0 THEN
        RAISE EXCEPTION 'DISMISS needs a note';
    END IF;

    INSERT INTO break_actions (break_id, desk_id, user_id, action, note) VALUES (p_break, b.desk_id, u, p_action, p_note);
    v_payload := jsonb_build_object('break_id', p_break, 'action', p_action, 'deal_id', p_deal,
                                    'note', p_note, 'user_id', u, 'at', now());
    INSERT INTO source_events (source, source_ref, event_kind, source_ts, observed_at, payload, desk_id, dedup_hash)
    VALUES ('USER', 'break:' || p_break::text, 'USER', now(), now(), v_payload, b.desk_id,
            sha256(convert_to('USER|break:' || p_break::text || '|USER|' || v_payload::text, 'UTF8')))
    RETURNING id INTO v_id;
    RETURN v_id;
END
$$;

-- ---------------------------------------------------------------------------
-- Grants
-- ---------------------------------------------------------------------------
GRANT USAGE ON SCHEMA public TO treasury_recon;
GRANT SELECT ON desks, source_systems, desk_mappings, counterparty_aliases, sla_expectations, calendar_holidays TO treasury_recon;
GRANT SELECT, INSERT ON source_events TO treasury_recon;
GRANT UPDATE (norm, normaliser_version, desk_id, processed_at, outcome, outcome_detail) ON source_events TO treasury_recon;
GRANT SELECT, INSERT, UPDATE, DELETE ON source_deals, breaks, break_actions, feed_status, recon_status TO treasury_recon;
GRANT SELECT, INSERT, UPDATE, DELETE ON deals TO treasury_recon;
GRANT USAGE ON SEQUENCE deal_ref_seq, source_events_id_seq, breaks_id_seq, break_actions_id_seq TO treasury_recon;

GRANT SELECT ON source_systems, desk_mappings, counterparty_aliases, sla_expectations, calendar_holidays,
                source_events, source_deals, breaks, break_actions, feed_status, recon_status TO treasury_app;
REVOKE UPDATE ON deals FROM treasury_app;
GRANT UPDATE (product, side, ccy1, ccy2, notional, rate, trade_date, value_date, maturity_date, counterparty, status)
    ON deals TO treasury_app;

REVOKE EXECUTE ON FUNCTION break_ack(bigint), break_resolve(bigint, text, uuid, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION break_ack(bigint), break_resolve(bigint, text, uuid, text) TO treasury_app;
