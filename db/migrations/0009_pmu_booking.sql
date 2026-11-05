-- Phase 2H: the PMU desk books its own trades. Purchases of bonds (adding a bond the system does
-- not know yet), sales out of the lots held (oldest first by default), repos against the bonds
-- held, reverse repos lending cash against a counterparty's bonds, closing a repo early and rolling
-- it. A booking counts at once and is provisional until it carries a Kondor deal id.
--
-- Every write goes through a definer function that checks the caller may book on the PMU desk and
-- that the face sold or pledged is free, under row locks. The server computes the prices, the
-- effective rate and the FTP rate to lock (the `pmu` crate) and passes them in; these functions
-- guard what may be stored.

-- ---------------------------------------------------------------------------
-- Where a row came from, and who booked it.
-- ---------------------------------------------------------------------------
ALTER TABLE bond_lots
    ADD COLUMN origin text NOT NULL DEFAULT 'FEED' CHECK (origin IN ('FEED', 'INTERNAL')),
    ADD COLUMN booked_by uuid NULL REFERENCES users(id),
    ADD COLUMN booked_at timestamptz NULL;
ALTER TABLE irs
    ADD COLUMN origin text NOT NULL DEFAULT 'FEED' CHECK (origin IN ('FEED', 'INTERNAL')),
    ADD COLUMN booked_by uuid NULL REFERENCES users(id),
    ADD COLUMN booked_at timestamptz NULL;
ALTER TABLE repos
    ADD COLUMN origin text NOT NULL DEFAULT 'FEED' CHECK (origin IN ('FEED', 'INTERNAL')),
    ADD COLUMN booked_by uuid NULL REFERENCES users(id),
    ADD COLUMN booked_at timestamptz NULL,
    -- REPO: the bank borrows cash against its own bonds. REVERSE: it lends cash against the
    -- counterparty's bonds, which are not in the portfolio.
    ADD COLUMN direction text NOT NULL DEFAULT 'REPO' CHECK (direction IN ('REPO', 'REVERSE')),
    ADD COLUMN closed_on date NULL,
    ADD COLUMN rolled_from uuid NULL REFERENCES repos(id),
    ADD COLUMN collateral_isin text NULL CHECK (length(collateral_isin) <= 12),
    ADD COLUMN collateral_face numeric(20,2) NULL CHECK (collateral_face > 0),
    ADD COLUMN collateral_note text NULL CHECK (length(collateral_note) <= 200),
    ADD CONSTRAINT repos_closed_in_term CHECK (closed_on IS NULL OR (closed_on > start_date AND closed_on <= end_date));

-- ---------------------------------------------------------------------------
-- Sales: one row per lot a sale draws on. The lot keeps its purchase face; what it holds on a
-- date is that face less the sales settled by then.
-- ---------------------------------------------------------------------------
CREATE TABLE lot_sales (
    id                uuid PRIMARY KEY,
    sale_ref          text NOT NULL CHECK (length(sale_ref) BETWEEN 1 AND 40),
    lot_id            uuid NOT NULL REFERENCES bond_lots(id),
    trade_date        date NOT NULL,
    settle_date       date NOT NULL CHECK (settle_date >= trade_date),
    face              numeric(20,2) NOT NULL CHECK (face > 0),
    clean_price       numeric(12,6) NOT NULL CHECK (clean_price > 0),
    accrued_received  numeric(20,2) NOT NULL DEFAULT 0,
    kondor_id         text NULL CHECK (length(kondor_id) BETWEEN 1 AND 40),
    origin            text NOT NULL DEFAULT 'INTERNAL' CHECK (origin IN ('FEED', 'INTERNAL')),
    booked_by         uuid NULL REFERENCES users(id),
    booked_at         timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX lot_sales_lot ON lot_sales (lot_id);

ALTER TABLE lot_sales ENABLE ROW LEVEL SECURITY;
ALTER TABLE lot_sales FORCE ROW LEVEL SECURITY;
CREATE POLICY lot_sales_admin ON lot_sales TO CURRENT_USER USING (true) WITH CHECK (true);
CREATE POLICY lot_sales_recon ON lot_sales TO treasury_recon USING (true) WITH CHECK (true);
CREATE POLICY lot_sales_read ON lot_sales FOR SELECT TO treasury_app USING (can_see_pmu());
GRANT SELECT ON lot_sales TO treasury_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON lot_sales TO treasury_recon;

CREATE SEQUENCE pmu_ref_seq;
GRANT USAGE ON SEQUENCE pmu_ref_seq TO treasury_app;

-- ---------------------------------------------------------------------------
-- Who may book: the PMU desk's bookers.
-- ---------------------------------------------------------------------------
CREATE FUNCTION pmu_can_book() RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM user_desk_access a JOIN desks k ON k.id = a.desk_id
        WHERE a.user_id = current_app_user() AND k.code = 'PMU' AND a.can_book
    )
$$;

CREATE FUNCTION pmu_require_booker() RETURNS uuid LANGUAGE plpgsql STABLE AS $$
BEGIN
    IF current_app_user() IS NULL OR NOT pmu_can_book() THEN
        RAISE EXCEPTION 'not entitled to book on the PMU desk' USING ERRCODE = '42501';
    END IF;
    RETURN current_app_user();
END
$$;

-- Face of a lot held on a date: its purchase face less what was sold and settled by then.
CREATE FUNCTION pmu_held_face(p_lot uuid, p_on date) RETURNS numeric LANGUAGE sql STABLE AS $$
    SELECT l.face - coalesce((SELECT sum(s.face) FROM lot_sales s WHERE s.lot_id = l.id AND s.settle_date <= p_on), 0)
    FROM bond_lots l WHERE l.id = p_lot
$$;

-- Face of a lot pledged to repos open at any point in [p_from, p_to), other than p_except.
CREATE FUNCTION pmu_pledged_face(p_lot uuid, p_from date, p_to date, p_except uuid) RETURNS numeric LANGUAGE sql STABLE AS $$
    SELECT coalesce(sum(c.face), 0) FROM repo_collateral c JOIN repos r ON r.id = c.repo_id
    WHERE c.lot_id = p_lot AND r.direction = 'REPO' AND r.id IS DISTINCT FROM p_except
      AND r.start_date < p_to AND coalesce(r.closed_on, r.end_date) > p_from
$$;

-- ---------------------------------------------------------------------------
-- Adding a bond the system does not know yet. The server has checked the ISIN's check digit.
-- ---------------------------------------------------------------------------
CREATE FUNCTION pmu_bond_add(p jsonb) RETURNS text
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    u uuid := pmu_require_booker();
BEGIN
    INSERT INTO bonds (isin, name, issuer, issuer_type, ccy, coupon, coupon_freq, day_count, issue_date, maturity, rate_type, float_index, float_spread, hqla_level)
    VALUES (p->>'isin', p->>'name', p->>'issuer', p->>'issuer_type', p->>'ccy', (p->>'coupon')::numeric, (p->>'coupon_freq')::smallint, p->>'day_count',
            (p->>'issue_date')::date, (p->>'maturity')::date, p->>'rate_type', nullif(p->>'float_index', ''), coalesce((p->>'float_spread')::numeric, 0),
            nullif(p->>'hqla_level', ''));
    PERFORM pg_notify('alm_events', json_build_object('pmu', true)::text);
    RETURN p->>'isin';
END
$$;

-- ---------------------------------------------------------------------------
-- A purchase: one new lot.
-- ---------------------------------------------------------------------------
CREATE FUNCTION pmu_buy(p_id uuid, p_book text, p_isin text, p_trade date, p_settle date, p_face numeric, p_clean numeric, p_accrued numeric,
                        p_kondor text, p_eir numeric, p_ftp numeric, p_ftp_pub integer) RETURNS uuid
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    u uuid := pmu_require_booker();
    m date;
BEGIN
    SELECT maturity INTO m FROM bonds WHERE isin = p_isin;
    IF m IS NULL THEN
        RAISE EXCEPTION 'no bond with ISIN %', p_isin;
    END IF;
    IF p_settle >= m THEN
        RAISE EXCEPTION 'the bond matures on % , before the purchase settles', m;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pmu_books WHERE code = p_book AND active) THEN
        RAISE EXCEPTION 'no open book %', p_book;
    END IF;
    INSERT INTO bond_lots (id, book, isin, trade_date, settle_date, face, clean_price, accrued_paid, kondor_id, eir, ftp_rate, ftp_publication_id, status, origin, booked_by, booked_at)
    VALUES (p_id, p_book, p_isin, p_trade, p_settle, p_face, p_clean, coalesce(p_accrued, 0), nullif(trim(p_kondor), ''), p_eir, p_ftp, p_ftp_pub, 'OPEN', 'INTERNAL', u, now());
    PERFORM pg_notify('alm_events', json_build_object('pmu', true)::text);
    RETURN p_id;
END
$$;

-- ---------------------------------------------------------------------------
-- A sale across one or more lots: [{lot_id, face, accrued}]. Each lot must hold that much free
-- face (held, not sold, not pledged to a repo open on the settlement date) when the sale settles.
-- ---------------------------------------------------------------------------
CREATE FUNCTION pmu_sell(p_sale_ref text, p_trade date, p_settle date, p_clean numeric, p_kondor text, p_alloc jsonb) RETURNS integer
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    u    uuid := pmu_require_booker();
    a    record;
    free numeric;
    n    integer := 0;
BEGIN
    IF p_alloc IS NULL OR jsonb_typeof(p_alloc) <> 'array' OR jsonb_array_length(p_alloc) = 0 THEN
        RAISE EXCEPTION 'the sale draws on no lot';
    END IF;
    FOR a IN SELECT x.lot_id, x.face, coalesce(x.accrued, 0) AS accrued FROM jsonb_to_recordset(p_alloc) AS x(lot_id uuid, face numeric, accrued numeric) LOOP
        PERFORM 1 FROM bond_lots WHERE id = a.lot_id FOR UPDATE;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'no lot %', a.lot_id;
        END IF;
        -- Free on the settlement date, and not already promised to a later sale.
        free := pmu_held_face(a.lot_id, p_settle)
              - coalesce((SELECT sum(s.face) FROM lot_sales s WHERE s.lot_id = a.lot_id AND s.settle_date > p_settle), 0)
              - pmu_pledged_face(a.lot_id, p_settle, p_settle + 1, NULL);
        IF a.face <= 0 OR a.face > free + 0.005 THEN
            RAISE EXCEPTION 'that lot has only % of face free to sell on %', round(greatest(free, 0), 2), p_settle;
        END IF;
        INSERT INTO lot_sales (id, sale_ref, lot_id, trade_date, settle_date, face, clean_price, accrued_received, kondor_id, booked_by)
        VALUES (gen_random_uuid(), p_sale_ref, a.lot_id, p_trade, p_settle, a.face, p_clean, a.accrued, nullif(trim(p_kondor), ''), u);
        UPDATE bond_lots SET updated_at = now() WHERE id = a.lot_id;
        n := n + 1;
    END LOOP;
    PERFORM pg_notify('alm_events', json_build_object('pmu', true)::text);
    RETURN n;
END
$$;

-- ---------------------------------------------------------------------------
-- A repo or reverse repo. A repo pledges face of lots held, which must be free for the whole term.
-- ---------------------------------------------------------------------------
CREATE FUNCTION pmu_repo_book(p_id uuid, p_direction text, p_cpty text, p_ccy text, p_start date, p_end date, p_cash numeric, p_rate numeric, p_haircut numeric,
                              p_kondor text, p_coll jsonb, p_cisin text, p_cface numeric, p_cnote text, p_rolled_from uuid) RETURNS uuid
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    u    uuid := pmu_require_booker();
    a    record;
    free numeric;
BEGIN
    IF p_end <= p_start THEN
        RAISE EXCEPTION 'a repo ends after it starts';
    END IF;
    INSERT INTO repos (id, counterparty, kondor_id, ccy, start_date, end_date, cash, rate, haircut_pct, direction, rolled_from, collateral_isin, collateral_face, collateral_note, origin, booked_by, booked_at)
    VALUES (p_id, trim(p_cpty), nullif(trim(p_kondor), ''), p_ccy, p_start, p_end, p_cash, p_rate, p_haircut, p_direction, p_rolled_from,
            nullif(trim(p_cisin), ''), p_cface, nullif(trim(p_cnote), ''), 'INTERNAL', u, now());
    IF p_direction = 'REPO' THEN
        IF p_coll IS NULL OR jsonb_typeof(p_coll) <> 'array' OR jsonb_array_length(p_coll) = 0 THEN
            RAISE EXCEPTION 'a repo needs bonds pledged against it';
        END IF;
        FOR a IN SELECT x.lot_id, x.face FROM jsonb_to_recordset(p_coll) AS x(lot_id uuid, face numeric) LOOP
            PERFORM 1 FROM bond_lots WHERE id = a.lot_id FOR UPDATE;
            -- The least the lot holds over the term (sales settling inside it count), less what
            -- other repos overlapping the term have pledged.
            free := least(pmu_held_face(a.lot_id, p_start), pmu_held_face(a.lot_id, p_end))
                  - pmu_pledged_face(a.lot_id, p_start, p_end, p_id);
            IF a.face <= 0 OR a.face > free + 0.005 THEN
                RAISE EXCEPTION 'that lot has only % of face free to pledge from % to %', round(greatest(free, 0), 2), p_start, p_end;
            END IF;
            INSERT INTO repo_collateral (repo_id, lot_id, face) VALUES (p_id, a.lot_id, a.face);
        END LOOP;
    ELSIF p_cisin IS NULL OR p_cface IS NULL THEN
        RAISE EXCEPTION 'a reverse repo names the bond received and its face';
    END IF;
    PERFORM pg_notify('alm_events', json_build_object('pmu', true)::text);
    RETURN p_id;
END
$$;

CREATE FUNCTION pmu_repo_close(p_id uuid, p_on date) RETURNS void
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    u uuid := pmu_require_booker();
    r repos%ROWTYPE;
BEGIN
    SELECT * INTO r FROM repos WHERE id = p_id FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'no repo %', p_id USING ERRCODE = 'P0002';
    END IF;
    IF r.closed_on IS NOT NULL THEN
        RAISE EXCEPTION 'that repo was closed on %', r.closed_on;
    END IF;
    IF p_on <= r.start_date OR p_on >= r.end_date THEN
        RAISE EXCEPTION 'a repo can be closed between % and %', r.start_date + 1, r.end_date - 1;
    END IF;
    UPDATE repos SET closed_on = p_on, updated_at = now() WHERE id = p_id;
    PERFORM pg_notify('alm_events', json_build_object('pmu', true)::text);
END
$$;

-- Rolling: the old repo closes on p_on and a new one starts that day with the same collateral.
CREATE FUNCTION pmu_repo_roll(p_id uuid, p_new uuid, p_on date, p_end date, p_rate numeric, p_cash numeric, p_kondor text) RETURNS uuid
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    u    uuid := pmu_require_booker();
    r    repos%ROWTYPE;
    coll jsonb;
BEGIN
    SELECT * INTO r FROM repos WHERE id = p_id FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'no repo %', p_id USING ERRCODE = 'P0002';
    END IF;
    IF r.closed_on IS NOT NULL THEN
        RAISE EXCEPTION 'that repo was closed on %', r.closed_on;
    END IF;
    IF p_on <= r.start_date OR p_on > r.end_date THEN
        RAISE EXCEPTION 'a repo can be rolled between % and %', r.start_date + 1, r.end_date;
    END IF;
    IF p_on < r.end_date THEN
        UPDATE repos SET closed_on = p_on, updated_at = now() WHERE id = p_id;
    END IF;
    SELECT coalesce(jsonb_agg(jsonb_build_object('lot_id', lot_id, 'face', face)), '[]'::jsonb) INTO coll FROM repo_collateral WHERE repo_id = p_id;
    RETURN pmu_repo_book(p_new, r.direction, r.counterparty, r.ccy, p_on, p_end, p_cash, p_rate, r.haircut_pct, p_kondor,
                         CASE WHEN r.direction = 'REPO' THEN coll ELSE NULL END, r.collateral_isin, r.collateral_face, r.collateral_note, p_id);
END
$$;

-- Attaching the Kondor deal id to an internal booking: lot, repo, or every row of a sale.
CREATE FUNCTION pmu_set_kondor(p_kind text, p_id text, p_kondor text) RETURNS void
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    u uuid := pmu_require_booker();
    n integer;
BEGIN
    IF coalesce(length(trim(p_kondor)), 0) = 0 THEN
        RAISE EXCEPTION 'a Kondor deal id is required';
    END IF;
    IF p_kind = 'lot' THEN
        UPDATE bond_lots SET kondor_id = trim(p_kondor), updated_at = now() WHERE id = p_id::uuid AND origin = 'INTERNAL';
    ELSIF p_kind = 'repo' THEN
        UPDATE repos SET kondor_id = trim(p_kondor), updated_at = now() WHERE id = p_id::uuid AND origin = 'INTERNAL';
    ELSIF p_kind = 'sale' THEN
        UPDATE lot_sales SET kondor_id = trim(p_kondor), updated_at = now() WHERE sale_ref = p_id;
    ELSE
        RAISE EXCEPTION 'unknown booking kind %', p_kind;
    END IF;
    GET DIAGNOSTICS n = ROW_COUNT;
    IF n = 0 THEN
        RAISE EXCEPTION 'no booking of ours with that id' USING ERRCODE = 'P0002';
    END IF;
    PERFORM pg_notify('alm_events', json_build_object('pmu', true)::text);
END
$$;

DO $$
DECLARE f text;
BEGIN
    FOREACH f IN ARRAY ARRAY[
        'pmu_bond_add(jsonb)',
        'pmu_buy(uuid, text, text, date, date, numeric, numeric, numeric, text, numeric, numeric, integer)',
        'pmu_sell(text, date, date, numeric, text, jsonb)',
        'pmu_repo_book(uuid, text, text, text, date, date, numeric, numeric, numeric, text, jsonb, text, numeric, text, uuid)',
        'pmu_repo_close(uuid, date)',
        'pmu_repo_roll(uuid, uuid, date, date, numeric, numeric, text)',
        'pmu_set_kondor(text, text, text)'
    ] LOOP
        EXECUTE format('REVOKE EXECUTE ON FUNCTION %s FROM PUBLIC', f);
        EXECUTE format('GRANT EXECUTE ON FUNCTION %s TO treasury_app', f);
    END LOOP;
END
$$;
