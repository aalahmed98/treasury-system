-- Phase 2G, part 2: the bond portfolio of the PMU desk.
--
-- The purchase lot is the base record: one purchase of one bond into one book, with its price,
-- the effective interest rate fixed at purchase and the FTP rate locked from the curve published
-- on its trade date. A position is every open lot of a bond in a book. Swaps hedge lots; repos
-- pledge lots as collateral. Income, risk and the forecast are computed from these rows by the
-- server (the `pmu` crate), so nothing here stores an accrual.
--
-- Rows are written by the reconciler (synthetic now, the Kondor feed later); the app reads them.
-- PMU booking tickets will write through definer functions in a later phase.

CREATE TABLE pmu_books (
    code        text PRIMARY KEY CHECK (code ~ '^[A-Z0-9][A-Z0-9 _-]{0,29}$'),
    name        text NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
    accounting  text NOT NULL CHECK (accounting IN ('AFS', 'HTM', 'FVTPL')),
    ccy         char(3) NOT NULL CHECK (ccy ~ '^[A-Z]{3}$'),
    active      boolean NOT NULL DEFAULT true,
    updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE bonds (
    isin          text PRIMARY KEY CHECK (isin ~ '^[A-Z]{2}[A-Z0-9]{9}[0-9]$'),
    name          text NOT NULL CHECK (length(name) BETWEEN 1 AND 120),
    issuer        text NOT NULL CHECK (length(issuer) BETWEEN 1 AND 120),
    issuer_type   text NOT NULL CHECK (issuer_type IN ('SOVEREIGN', 'GCC_SOVEREIGN', 'SUPRANATIONAL', 'BANK', 'CORPORATE', 'SUKUK')),
    ccy           char(3) NOT NULL CHECK (ccy ~ '^[A-Z]{3}$'),
    coupon        numeric(10,6) NOT NULL CHECK (coupon BETWEEN 0 AND 30),          -- percent a year
    coupon_freq   smallint NOT NULL CHECK (coupon_freq IN (1, 2, 4, 12)),
    day_count     text NOT NULL CHECK (day_count IN ('ACT/ACT', '30/360', 'ACT/360', 'ACT/365')),
    issue_date    date NOT NULL,
    maturity      date NOT NULL CHECK (maturity > issue_date),
    rate_type     text NOT NULL CHECK (rate_type IN ('FIXED', 'FLOAT')),
    float_index   text NULL,
    float_spread  numeric(10,6) NOT NULL DEFAULT 0,                                -- percent over the index
    hqla_level    text NULL CHECK (hqla_level IN ('L1', 'L2A', 'L2B')),
    updated_at    timestamptz NOT NULL DEFAULT now(),
    CHECK ((rate_type = 'FLOAT') = (float_index IS NOT NULL))
);

CREATE TABLE bond_lots (
    id                  uuid PRIMARY KEY,
    book                text NOT NULL REFERENCES pmu_books(code),
    isin                text NOT NULL REFERENCES bonds(isin),
    trade_date          date NOT NULL,
    settle_date         date NOT NULL CHECK (settle_date >= trade_date),
    face                numeric(20,2) NOT NULL CHECK (face > 0),                   -- in the bond's currency
    clean_price         numeric(12,6) NOT NULL CHECK (clean_price > 0),           -- per 100 of face
    accrued_paid        numeric(20,2) NOT NULL DEFAULT 0,
    kondor_id           text NULL CHECK (length(kondor_id) BETWEEN 1 AND 40),
    eir                 numeric(12,8) NOT NULL,                                   -- percent, annual effective
    ftp_rate            numeric(10,6) NULL,                                       -- percent, locked at purchase
    ftp_publication_id  integer NULL REFERENCES ftp_publications(id),
    status              text NOT NULL DEFAULT 'OPEN' CHECK (status IN ('OPEN', 'MATURED', 'SOLD')),
    updated_at          timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX bond_lots_isin ON bond_lots (isin);
CREATE INDEX bond_lots_book ON bond_lots (book);

CREATE TABLE irs (
    id                  uuid PRIMARY KEY,
    book                text NOT NULL REFERENCES pmu_books(code),
    kondor_id           text NULL CHECK (length(kondor_id) BETWEEN 1 AND 40),
    ccy                 char(3) NOT NULL CHECK (ccy ~ '^[A-Z]{3}$'),
    notional            numeric(20,2) NOT NULL CHECK (notional > 0),
    start_date          date NOT NULL,
    maturity            date NOT NULL CHECK (maturity > start_date),
    pay_fixed           boolean NOT NULL,
    fixed_rate          numeric(10,6) NOT NULL,
    fixed_freq          smallint NOT NULL CHECK (fixed_freq IN (1, 2, 4, 12)),
    fixed_day_count     text NOT NULL CHECK (fixed_day_count IN ('ACT/ACT', '30/360', 'ACT/360', 'ACT/365')),
    float_index         text NOT NULL,
    float_tenor_months  smallint NOT NULL CHECK (float_tenor_months IN (1, 3, 6, 12)),
    float_spread        numeric(10,6) NOT NULL DEFAULT 0,
    status              text NOT NULL DEFAULT 'ACTIVE' CHECK (status IN ('ACTIVE', 'MATURED', 'TERMINATED')),
    updated_at          timestamptz NOT NULL DEFAULT now()
);

-- Which lots a swap hedges, and how much of its notional is assigned to each.
CREATE TABLE irs_hedges (
    irs_id    uuid NOT NULL REFERENCES irs(id) ON DELETE CASCADE,
    lot_id    uuid NOT NULL REFERENCES bond_lots(id) ON DELETE CASCADE,
    notional  numeric(20,2) NOT NULL CHECK (notional > 0),
    PRIMARY KEY (irs_id, lot_id)
);

-- Notional closed before maturity, and the cash that settled it (positive received).
CREATE TABLE irs_unwinds (
    irs_id      uuid NOT NULL REFERENCES irs(id) ON DELETE CASCADE,
    date        date NOT NULL,
    notional    numeric(20,2) NOT NULL CHECK (notional > 0),
    settlement  numeric(20,2) NOT NULL,
    PRIMARY KEY (irs_id, date)
);

CREATE TABLE repos (
    id            uuid PRIMARY KEY,
    counterparty  text NOT NULL CHECK (length(counterparty) BETWEEN 1 AND 120),
    kondor_id     text NULL CHECK (length(kondor_id) BETWEEN 1 AND 40),
    ccy           char(3) NOT NULL CHECK (ccy ~ '^[A-Z]{3}$'),
    start_date    date NOT NULL,
    end_date      date NOT NULL CHECK (end_date > start_date),
    cash          numeric(20,2) NOT NULL CHECK (cash > 0),
    rate          numeric(10,6) NOT NULL,                                         -- percent, ACT/360
    haircut_pct   numeric(6,3) NOT NULL CHECK (haircut_pct BETWEEN 0 AND 100),
    updated_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE repo_collateral (
    repo_id  uuid NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
    lot_id   uuid NOT NULL REFERENCES bond_lots(id) ON DELETE CASCADE,
    face     numeric(20,2) NOT NULL CHECK (face > 0),
    PRIMARY KEY (repo_id, lot_id)
);

CREATE TABLE repo_quotes (
    as_of         date NOT NULL,
    counterparty  text NOT NULL CHECK (length(counterparty) BETWEEN 1 AND 120),
    tenor         text NOT NULL CHECK (length(tenor) BETWEEN 1 AND 12),
    tenor_days    integer NOT NULL CHECK (tenor_days > 0),
    spread_bp     numeric(8,2) NOT NULL,                                          -- over SOFR
    quoted_at     timestamptz NOT NULL,
    PRIMARY KEY (as_of, counterparty, tenor)
);

CREATE TABLE bond_prices (
    as_of        date NOT NULL,
    isin         text NOT NULL REFERENCES bonds(isin),
    clean_price  numeric(12,6) NOT NULL CHECK (clean_price > 0),
    ytm          numeric(10,6) NOT NULL,                                          -- percent
    PRIMARY KEY (as_of, isin)
);

-- ---------------------------------------------------------------------------
-- Row-level security: the PMU desk, ALM and the head read; the reconciler writes.
-- ---------------------------------------------------------------------------
DO $$
DECLARE t text;
BEGIN
    FOREACH t IN ARRAY ARRAY['pmu_books', 'bonds', 'bond_lots', 'irs', 'irs_hedges', 'irs_unwinds', 'repos', 'repo_collateral', 'repo_quotes', 'bond_prices'] LOOP
        EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
        EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
        EXECUTE format('CREATE POLICY %I ON %I TO CURRENT_USER USING (true) WITH CHECK (true)', t || '_admin', t);
        EXECUTE format('CREATE POLICY %I ON %I TO treasury_recon USING (true) WITH CHECK (true)', t || '_recon', t);
        EXECUTE format('CREATE POLICY %I ON %I FOR SELECT TO treasury_app USING (can_see_pmu())', t || '_read', t);
        EXECUTE format('GRANT SELECT ON %I TO treasury_app', t);
        EXECUTE format('GRANT SELECT, INSERT, UPDATE, DELETE ON %I TO treasury_recon', t);
    END LOOP;
END
$$;

INSERT INTO pmu_books (code, name, accounting, ccy) VALUES
    ('AFS BHD', 'Available for sale, dinar', 'AFS', 'BHD'),
    ('AFS USD', 'Available for sale, dollar', 'AFS', 'USD'),
    ('HTM', 'Held to maturity', 'HTM', 'USD');
