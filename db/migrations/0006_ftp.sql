-- Phase 2G, part 1: the PMU desk and the day's funds transfer pricing (FTP) curves.
--
-- ALM publishes the FTP curves each day from its Excel workbook. Every desk reads them. The set of
-- curves is whatever was published: a currency is added by publishing a curve for it and retired
-- by no longer publishing one, so nothing here lists the currencies. Each publication is kept;
-- publishing again for the same day supersedes the earlier one without deleting it.

-- ---------------------------------------------------------------------------
-- The Portfolio Management Unit: the bank's bond portfolio, its hedges and its repo funding.
-- ---------------------------------------------------------------------------
INSERT INTO desks (code, name) VALUES ('PMU', 'Portfolio Management');

-- Who may read the bond portfolio: the PMU desk, ALM (it reports the portfolio to ALCO) and the head.
CREATE FUNCTION can_see_pmu() RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM user_desk_access a JOIN desks k ON k.id = a.desk_id
        WHERE a.user_id = current_app_user() AND k.code IN ('PMU', 'ALM', 'HEAD')
    )
$$;

-- ---------------------------------------------------------------------------
-- FTP publications and their points.
-- ---------------------------------------------------------------------------
CREATE TABLE ftp_publications (
    id            serial PRIMARY KEY,
    as_of         date NOT NULL,
    published_at  timestamptz NOT NULL DEFAULT now(),
    published_by  uuid NULL REFERENCES users(id),          -- NULL: written by the reconciler (synthetic)
    file_name     text NULL CHECK (length(file_name) <= 255),
    file_sha256   text NULL CHECK (file_sha256 ~ '^[0-9a-f]{64}$'),
    note          text NULL CHECK (length(note) <= 500),
    is_current    boolean NOT NULL DEFAULT true
);
CREATE UNIQUE INDEX ftp_publications_current ON ftp_publications (as_of) WHERE is_current;
CREATE INDEX ftp_publications_as_of ON ftp_publications (as_of DESC);

CREATE TABLE ftp_points (
    publication_id  integer NOT NULL REFERENCES ftp_publications(id) ON DELETE CASCADE,
    ccy             char(3) NOT NULL CHECK (ccy ~ '^[A-Z]{3}$'),
    tenor           text NOT NULL CHECK (length(tenor) BETWEEN 1 AND 12),
    tenor_days      integer NOT NULL CHECK (tenor_days > 0),
    rate            numeric(10,6) NOT NULL CHECK (rate BETWEEN -5 AND 50),   -- percent
    PRIMARY KEY (publication_id, ccy, tenor_days)
);

ALTER TABLE ftp_publications ENABLE ROW LEVEL SECURITY;
ALTER TABLE ftp_publications FORCE ROW LEVEL SECURITY;
ALTER TABLE ftp_points ENABLE ROW LEVEL SECURITY;
ALTER TABLE ftp_points FORCE ROW LEVEL SECURITY;

CREATE POLICY fp_admin ON ftp_publications TO CURRENT_USER USING (true) WITH CHECK (true);
CREATE POLICY fp_recon ON ftp_publications TO treasury_recon USING (true) WITH CHECK (true);
CREATE POLICY fp_read ON ftp_publications FOR SELECT TO treasury_app USING (current_app_user() IS NOT NULL);
CREATE POLICY fpt_admin ON ftp_points TO CURRENT_USER USING (true) WITH CHECK (true);
CREATE POLICY fpt_recon ON ftp_points TO treasury_recon USING (true) WITH CHECK (true);
CREATE POLICY fpt_read ON ftp_points FOR SELECT TO treasury_app USING (current_app_user() IS NOT NULL);

GRANT SELECT ON ftp_publications, ftp_points TO treasury_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON ftp_publications, ftp_points TO treasury_recon;
GRANT USAGE ON SEQUENCE ftp_publications_id_seq TO treasury_recon;

-- ---------------------------------------------------------------------------
-- Publishing: ALM or the head, with a note. The curves arrive as a JSON array of
-- {ccy, tenor, tenor_days, rate}; the server has already read and checked the workbook, and this
-- checks again so nothing malformed can be stored whoever calls it.
-- ---------------------------------------------------------------------------
CREATE FUNCTION ftp_publish(p_as_of date, p_points jsonb, p_file_name text, p_file_sha256 text, p_note text) RETURNS integer
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    u   uuid := current_app_user();
    v_id  integer;
    n   integer;
BEGIN
    IF u IS NULL OR NOT can_see_alm() THEN
        RAISE EXCEPTION 'only ALM may publish the FTP curves' USING ERRCODE = '42501';
    END IF;
    IF p_as_of IS NULL THEN
        RAISE EXCEPTION 'the publication needs a date';
    END IF;
    IF p_points IS NULL OR jsonb_typeof(p_points) <> 'array' OR jsonb_array_length(p_points) = 0 THEN
        RAISE EXCEPTION 'the publication holds no curve points';
    END IF;
    IF coalesce(length(trim(p_note)), 0) = 0 THEN
        RAISE EXCEPTION 'a note is required';
    END IF;
    PERFORM 1 FROM ftp_publications WHERE as_of = p_as_of AND is_current FOR UPDATE;
    UPDATE ftp_publications SET is_current = false WHERE as_of = p_as_of AND is_current;
    INSERT INTO ftp_publications (as_of, published_by, file_name, file_sha256, note)
    VALUES (p_as_of, u, nullif(trim(p_file_name), ''), nullif(p_file_sha256, ''), trim(p_note))
    RETURNING ftp_publications.id INTO v_id;
    INSERT INTO ftp_points (publication_id, ccy, tenor, tenor_days, rate)
    SELECT v_id, x.ccy, x.tenor, x.tenor_days, x.rate
    FROM jsonb_to_recordset(p_points) AS x(ccy text, tenor text, tenor_days integer, rate numeric);
    GET DIAGNOSTICS n = ROW_COUNT;
    IF n = 0 THEN
        RAISE EXCEPTION 'the publication holds no curve points';
    END IF;
    PERFORM pg_notify('alm_events', json_build_object('ftp', p_as_of)::text);
    RETURN v_id;
END
$$;
REVOKE EXECUTE ON FUNCTION ftp_publish(date, jsonb, text, text, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION ftp_publish(date, jsonb, text, text, text) TO treasury_app;
