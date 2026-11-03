-- Phase 1 schema: desks, users, entitlements, sessions, deals, RLS, change notification.
-- Runs as treasury_owner (table owner). The server connects as treasury_app, which owns nothing
-- and cannot bypass row-level security.

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'treasury_app') THEN
        CREATE ROLE treasury_app LOGIN PASSWORD 'app_dev' NOBYPASSRLS;
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
-- Identity
-- ---------------------------------------------------------------------------
CREATE TABLE users (
    id             uuid PRIMARY KEY,
    username       text NOT NULL UNIQUE,
    password_hash  text NOT NULL,
    display_name   text NOT NULL,
    home_desk_id   smallint NOT NULL REFERENCES desks(id),
    is_active      boolean NOT NULL DEFAULT true,
    created_at     timestamptz NOT NULL DEFAULT now()
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

CREATE TABLE audit_log (
    id        bigserial PRIMARY KEY,
    at        timestamptz NOT NULL DEFAULT now(),
    user_id   uuid NULL,
    username  text NULL,
    action    text NOT NULL,
    detail    jsonb NULL
);

-- ---------------------------------------------------------------------------
-- Deals
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
    counterparty   text NOT NULL,
    trader_id      uuid NOT NULL REFERENCES users(id),
    status         text NOT NULL CHECK (status IN ('EXECUTED', 'BOOKED', 'CONFIRMED', 'SETTLED', 'CANCELLED')),
    version        integer NOT NULL DEFAULT 1,
    created_at     timestamptz NOT NULL DEFAULT now(),
    updated_at     timestamptz NOT NULL DEFAULT now()
);
CREATE SEQUENCE deal_ref_seq;

CREATE INDEX deals_desk_trade_date_idx ON deals (desk_id, trade_date DESC);
CREATE INDEX deals_updated_at_idx ON deals (updated_at);

-- Bump version and updated_at on every update, then notify.
CREATE FUNCTION deals_before_update() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    NEW.version := OLD.version + 1;
    NEW.updated_at := now();
    RETURN NEW;
END
$$;
CREATE TRIGGER deals_before_update
    BEFORE UPDATE ON deals FOR EACH ROW EXECUTE FUNCTION deals_before_update();

CREATE FUNCTION deals_notify() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_notify('deal_events', json_build_object(
        'op', TG_OP,
        'id', NEW.id,
        'desk_id', NEW.desk_id,
        'version', NEW.version
    )::text);
    RETURN NEW;
END
$$;
CREATE TRIGGER deals_notify
    AFTER INSERT OR UPDATE ON deals FOR EACH ROW EXECUTE FUNCTION deals_notify();

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
-- Row-level security. Identity is app.user_id, set per transaction by the server.
-- NULLIF guards the empty-string placeholder Postgres leaves after a SET LOCAL ends,
-- so an unset identity sees nothing instead of raising a cast error.
-- ---------------------------------------------------------------------------
CREATE FUNCTION current_app_user() RETURNS uuid LANGUAGE sql STABLE AS $$
    SELECT NULLIF(current_setting('app.user_id', true), '')::uuid
$$;

ALTER TABLE deals ENABLE ROW LEVEL SECURITY;
ALTER TABLE deals FORCE ROW LEVEL SECURITY;

CREATE POLICY deals_read ON deals FOR SELECT TO treasury_app
    USING (desk_id IN (SELECT desk_id FROM user_desk_access WHERE user_id = current_app_user()));

CREATE POLICY deals_book ON deals FOR INSERT TO treasury_app
    WITH CHECK (EXISTS (
        SELECT 1 FROM user_desk_access
        WHERE user_id = current_app_user() AND desk_id = deals.desk_id AND can_book
    ));

CREATE POLICY deals_amend ON deals FOR UPDATE TO treasury_app
    USING (desk_id IN (SELECT desk_id FROM user_desk_access WHERE user_id = current_app_user()))
    WITH CHECK (EXISTS (
        SELECT 1 FROM user_desk_access
        WHERE user_id = current_app_user() AND desk_id = deals.desk_id AND can_book
    ));

ALTER TABLE user_desk_access ENABLE ROW LEVEL SECURITY;
ALTER TABLE user_desk_access FORCE ROW LEVEL SECURITY;

CREATE POLICY uda_self ON user_desk_access FOR SELECT TO treasury_app
    USING (user_id = current_app_user());

-- Seeding needs to create entitlements before any identity exists; allow inserts only.
CREATE POLICY uda_seed_insert ON user_desk_access FOR INSERT TO treasury_app
    WITH CHECK (true);

-- ---------------------------------------------------------------------------
-- Grants for the application role
-- ---------------------------------------------------------------------------
GRANT USAGE ON SCHEMA public TO treasury_app;
GRANT SELECT ON desks TO treasury_app;
GRANT SELECT, INSERT, UPDATE ON users TO treasury_app;
GRANT SELECT, INSERT ON user_desk_access TO treasury_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON sessions TO treasury_app;
GRANT INSERT ON audit_log TO treasury_app;
GRANT SELECT, INSERT, UPDATE ON deals TO treasury_app;
GRANT SELECT ON positions_v TO treasury_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO treasury_app;
