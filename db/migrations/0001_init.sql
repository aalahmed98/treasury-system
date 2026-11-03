-- Phase 1 schema: desks, users, credentials, entitlements, sessions, deals, history, RLS,
-- change notification and the login-hardening functions.
--
-- Runs as treasury_owner, a non-superuser that owns every object. The server connects as
-- treasury_app, which owns nothing, cannot bypass row-level security, and gets only the grants
-- listed at the bottom. Roles and their passwords are created outside migrations
-- (db/init for Docker, operations for anything real); the guard below only makes the migration
-- self-sufficient on a cluster where the app role is missing, and it never sets a password.

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'treasury_app') THEN
        RAISE EXCEPTION 'role treasury_app must exist before this migration runs (db/init/01-roles.sh creates it for Docker; operations create it elsewhere)';
    END IF;
END
$$;

-- ---------------------------------------------------------------------------
-- Reference data
-- ---------------------------------------------------------------------------
CREATE TABLE desks (
    id    smallserial PRIMARY KEY,
    code  text NOT NULL UNIQUE,
    name  text NOT NULL
);

INSERT INTO desks (code, name) VALUES
    ('FX',   'Foreign Exchange'),
    ('MM',   'Money Market'),
    ('ALM',  'Asset & Liability Management'),
    ('HEAD', 'Treasury Head');

-- ---------------------------------------------------------------------------
-- Identity. Password hashes live in their own table that the app role cannot read directly;
-- login goes through auth_lookup() below.
-- ---------------------------------------------------------------------------
CREATE TABLE users (
    id             uuid PRIMARY KEY,
    username       text NOT NULL UNIQUE
                   CHECK (username = lower(username) AND length(username) BETWEEN 1 AND 64),
    display_name   text NOT NULL CHECK (length(display_name) BETWEEN 1 AND 120),
    home_desk_id   smallint NOT NULL REFERENCES desks(id),
    is_active      boolean NOT NULL DEFAULT true,
    created_at     timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE user_credentials (
    user_id        uuid PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    password_hash  text NOT NULL,
    updated_at     timestamptz NOT NULL DEFAULT now()
);

-- Failed-login counters, keyed by the attempted username whether or not it exists, so the
-- lockout response cannot be used to discover which usernames are real.
CREATE TABLE login_throttle (
    username      text PRIMARY KEY,
    failures      integer NOT NULL DEFAULT 0,
    locked_until  timestamptz NULL,
    updated_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE user_desk_access (
    user_id   uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    desk_id   smallint NOT NULL REFERENCES desks(id),
    can_book  boolean NOT NULL DEFAULT false,
    PRIMARY KEY (user_id, desk_id)
);

CREATE TABLE sessions (
    token_hash    bytea PRIMARY KEY,
    user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at    timestamptz NOT NULL DEFAULT now(),
    expires_at    timestamptz NOT NULL,
    last_seen_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX sessions_user_idx ON sessions (user_id);
CREATE INDEX sessions_expiry_idx ON sessions (expires_at);

CREATE TABLE audit_log (
    id          bigserial PRIMARY KEY,
    at          timestamptz NOT NULL DEFAULT now(),
    user_id     uuid NULL,
    username    text NULL,
    action      text NOT NULL,
    client_ip   inet NULL,
    request_id  text NULL,
    detail      jsonb NULL
);
CREATE INDEX audit_log_at_idx ON audit_log (at);

-- ---------------------------------------------------------------------------
-- Deals and their append-only history
-- ---------------------------------------------------------------------------
CREATE TABLE deals (
    id             uuid PRIMARY KEY,
    deal_ref       text NOT NULL UNIQUE,
    desk_id        smallint NOT NULL REFERENCES desks(id),
    product        text NOT NULL CHECK (product IN ('FX_SPOT', 'FX_FWD', 'MM_DEPO', 'MM_LOAN')),
    side           text NOT NULL CHECK (side IN ('BUY', 'SELL', 'LEND', 'BORROW')),
    ccy1           char(3) NOT NULL,
    ccy2           char(3) NULL,
    notional       numeric(20, 4) NOT NULL CHECK (notional > 0),
    rate           numeric(18, 8) NOT NULL,
    trade_date     date NOT NULL,
    value_date     date NOT NULL,
    maturity_date  date NULL,
    counterparty   text NOT NULL CHECK (length(counterparty) BETWEEN 1 AND 200),
    trader_id      uuid NOT NULL REFERENCES users(id),
    status         text NOT NULL CHECK (status IN ('EXECUTED', 'BOOKED', 'CONFIRMED', 'SETTLED', 'CANCELLED')),
    version        integer NOT NULL DEFAULT 1,
    created_at     timestamptz NOT NULL DEFAULT now(),
    updated_at     timestamptz NOT NULL DEFAULT now()
);
CREATE SEQUENCE deal_ref_seq;
CREATE INDEX deals_desk_trade_date_idx ON deals (desk_id, trade_date DESC);
CREATE INDEX deals_updated_at_idx ON deals (updated_at);

-- Every version of every deal, written by a definer-rights trigger. The app role has no access.
CREATE TABLE deal_history (
    id          bigserial PRIMARY KEY,
    deal_id     uuid NOT NULL,
    version     integer NOT NULL,
    op          text NOT NULL,
    changed_at  timestamptz NOT NULL DEFAULT now(),
    changed_by  uuid NULL,
    snapshot    jsonb NOT NULL,
    UNIQUE (deal_id, version)
);

-- ---------------------------------------------------------------------------
-- Identity helper. set_config('app.user_id', ..., true) is transaction-local; after the
-- transaction ends the placeholder is '', which NULLIF turns into "no identity".
-- ---------------------------------------------------------------------------
CREATE FUNCTION current_app_user() RETURNS uuid LANGUAGE sql STABLE AS $$
    SELECT NULLIF(current_setting('app.user_id', true), '')::uuid
$$;

-- ---------------------------------------------------------------------------
-- Deal triggers
-- ---------------------------------------------------------------------------
CREATE FUNCTION deals_before_update() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.id <> OLD.id OR NEW.deal_ref <> OLD.deal_ref OR NEW.desk_id <> OLD.desk_id
       OR NEW.trader_id <> OLD.trader_id OR NEW.created_at <> OLD.created_at THEN
        RAISE EXCEPTION 'deal identity (id, reference, desk, trader, created_at) cannot be changed';
    END IF;
    IF OLD.status IN ('SETTLED', 'CANCELLED') THEN
        RAISE EXCEPTION 'a % deal cannot be amended', lower(OLD.status);
    END IF;
    NEW.version := OLD.version + 1;
    NEW.updated_at := now();
    RETURN NEW;
END
$$;
CREATE TRIGGER deals_before_update
    BEFORE UPDATE ON deals FOR EACH ROW EXECUTE FUNCTION deals_before_update();

-- Definer rights: the caller (treasury_app) cannot write deal_history itself.
CREATE FUNCTION deals_after_write() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
    INSERT INTO deal_history (deal_id, version, op, changed_by, snapshot)
    VALUES (NEW.id, NEW.version, TG_OP, current_app_user(), to_jsonb(NEW));
    PERFORM pg_notify('deal_events', json_build_object(
        'op', TG_OP,
        'id', NEW.id,
        'desk_id', NEW.desk_id,
        'version', NEW.version
    )::text);
    RETURN NEW;
END
$$;
CREATE TRIGGER deals_after_write
    AFTER INSERT OR UPDATE ON deals FOR EACH ROW EXECUTE FUNCTION deals_after_write();

-- Net position per desk and currency. security_invoker makes RLS on deals apply through the view.
CREATE VIEW positions_v WITH (security_invoker = true) AS
SELECT d.desk_id,
       k.code AS desk_code,
       d.ccy1::text AS ccy,
       SUM(CASE WHEN d.side IN ('BUY', 'LEND') THEN d.notional ELSE -d.notional END) AS net_notional,
       COUNT(*) AS deal_count
FROM deals d
JOIN desks k ON k.id = d.desk_id
WHERE d.status <> 'CANCELLED'
GROUP BY d.desk_id, k.code, d.ccy1;

-- ---------------------------------------------------------------------------
-- Login functions. Definer rights so the app role never touches credentials or throttle rows
-- directly; it can only ask these narrow questions.
-- ---------------------------------------------------------------------------
CREATE FUNCTION auth_lookup(p_username text)
    RETURNS TABLE (user_id uuid, password_hash text, is_active boolean, locked_until timestamptz)
    LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT u.id, c.password_hash, u.is_active, t.locked_until
    FROM (SELECT p_username AS username) q
    LEFT JOIN users u ON u.username = q.username
    LEFT JOIN user_credentials c ON c.user_id = u.id
    LEFT JOIN login_throttle t ON t.username = q.username
$$;

-- Records a failed attempt; returns the lock expiry if this attempt triggered or hit a lock.
CREATE FUNCTION auth_login_failed(p_username text, p_max_failures integer, p_lock interval)
    RETURNS timestamptz
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    v_locked timestamptz;
BEGIN
    INSERT INTO login_throttle (username, failures, updated_at) VALUES (p_username, 1, now())
    ON CONFLICT (username) DO UPDATE
        SET failures = login_throttle.failures + 1, updated_at = now();
    UPDATE login_throttle
       SET locked_until = now() + p_lock, failures = 0, updated_at = now()
     WHERE username = p_username AND failures >= p_max_failures
    RETURNING locked_until INTO v_locked;
    RETURN v_locked;
END
$$;

CREATE FUNCTION auth_login_succeeded(p_username text) RETURNS void
    LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    DELETE FROM login_throttle WHERE username = p_username
$$;

-- Housekeeping: expired and idle sessions, stale throttle rows.
CREATE FUNCTION auth_sweep(p_idle interval)
    RETURNS TABLE (sessions_removed bigint, throttle_removed bigint)
    LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    WITH s AS (
        DELETE FROM sessions WHERE expires_at < now() OR last_seen_at < now() - p_idle RETURNING 1
    ), t AS (
        DELETE FROM login_throttle
         WHERE updated_at < now() - interval '1 day' AND (locked_until IS NULL OR locked_until < now())
        RETURNING 1
    )
    SELECT (SELECT count(*) FROM s), (SELECT count(*) FROM t)
$$;

-- ---------------------------------------------------------------------------
-- Row-level security. FORCE applies it to the owner as well, so the owner's access is an
-- explicit policy rather than an implicit bypass.
-- ---------------------------------------------------------------------------
ALTER TABLE deals ENABLE ROW LEVEL SECURITY;
ALTER TABLE deals FORCE ROW LEVEL SECURITY;

CREATE POLICY deals_admin ON deals TO CURRENT_USER USING (true) WITH CHECK (true);

CREATE POLICY deals_read ON deals FOR SELECT TO treasury_app
    USING (desk_id IN (SELECT desk_id FROM user_desk_access WHERE user_id = current_app_user()));

-- Booking: must be entitled to book on that desk, and the deal must be attributed to the booker.
CREATE POLICY deals_book ON deals FOR INSERT TO treasury_app
    WITH CHECK (
        trader_id = current_app_user()
        AND EXISTS (
            SELECT 1 FROM user_desk_access
            WHERE user_id = current_app_user() AND desk_id = deals.desk_id AND can_book
        )
    );

-- Amending: booking rights are required on the deal's current desk (USING) and on the desk of
-- the resulting row (WITH CHECK). The trigger above additionally forbids changing desk_id.
CREATE POLICY deals_amend ON deals FOR UPDATE TO treasury_app
    USING (EXISTS (
        SELECT 1 FROM user_desk_access
        WHERE user_id = current_app_user() AND desk_id = deals.desk_id AND can_book
    ))
    WITH CHECK (EXISTS (
        SELECT 1 FROM user_desk_access
        WHERE user_id = current_app_user() AND desk_id = deals.desk_id AND can_book
    ));

ALTER TABLE user_desk_access ENABLE ROW LEVEL SECURITY;
ALTER TABLE user_desk_access FORCE ROW LEVEL SECURITY;

CREATE POLICY uda_admin ON user_desk_access TO CURRENT_USER USING (true) WITH CHECK (true);
CREATE POLICY uda_self ON user_desk_access FOR SELECT TO treasury_app
    USING (user_id = current_app_user());

-- ---------------------------------------------------------------------------
-- Grants for the application role: the minimum the server needs at runtime.
-- No access to user_credentials, login_throttle or deal_history; audit_log is append-only.
-- ---------------------------------------------------------------------------
GRANT USAGE ON SCHEMA public TO treasury_app;
GRANT SELECT ON desks, users, user_desk_access, positions_v TO treasury_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON sessions TO treasury_app;
GRANT INSERT ON audit_log TO treasury_app;
GRANT SELECT, INSERT, UPDATE ON deals TO treasury_app;
GRANT USAGE ON SEQUENCE deal_ref_seq, audit_log_id_seq TO treasury_app;

REVOKE EXECUTE ON FUNCTION
    auth_lookup(text), auth_login_failed(text, integer, interval),
    auth_login_succeeded(text), auth_sweep(interval)
FROM PUBLIC;
GRANT EXECUTE ON FUNCTION
    auth_lookup(text), auth_login_failed(text, integer, interval),
    auth_login_succeeded(text), auth_sweep(interval)
TO treasury_app;
