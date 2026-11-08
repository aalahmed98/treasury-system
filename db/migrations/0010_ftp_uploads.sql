-- Phase 2I: the FTP upload log. Every workbook ALM drops is kept, file and all, with who dropped it
-- and when, whether or not it was published. A publication points at the upload it came from, and
-- one replaced by a later workbook for the same day records when and by which publication.

CREATE TABLE ftp_uploads (
    id            serial PRIMARY KEY,
    uploaded_at   timestamptz NOT NULL DEFAULT now(),
    uploaded_by   uuid NOT NULL REFERENCES users(id),
    via           text NOT NULL DEFAULT 'browser' CHECK (via IN ('browser', 'command line')),
    file_name     text NOT NULL CHECK (length(file_name) BETWEEN 1 AND 255),
    file_sha256   text NOT NULL CHECK (file_sha256 ~ '^[0-9a-f]{64}$'),
    size_bytes    integer NOT NULL CHECK (size_bytes BETWEEN 1 AND 2097152),
    content       bytea NOT NULL,
    sheet_date    date NULL,
    currencies    text[] NOT NULL DEFAULT '{}',
    errors        integer NOT NULL DEFAULT 0 CHECK (errors >= 0),
    warnings      integer NOT NULL DEFAULT 0 CHECK (warnings >= 0)
);
CREATE INDEX ftp_uploads_at ON ftp_uploads (uploaded_at DESC);

ALTER TABLE ftp_publications
    ADD COLUMN upload_id      integer NULL REFERENCES ftp_uploads(id),
    ADD COLUMN replaced_at    timestamptz NULL,
    ADD COLUMN replaced_by_id integer NULL REFERENCES ftp_publications(id);
CREATE INDEX ftp_publications_upload ON ftp_publications (upload_id) WHERE upload_id IS NOT NULL;
CREATE INDEX ftp_publications_published_at ON ftp_publications (published_at DESC);

-- Publications already replaced: by the next one for the same day.
UPDATE ftp_publications o
   SET replaced_by_id = n.id, replaced_at = n.published_at
  FROM ftp_publications n
 WHERE NOT o.is_current
   AND n.id = (SELECT min(x.id) FROM ftp_publications x WHERE x.as_of = o.as_of AND x.id > o.id);

-- Who reads an upload: ALM (and the head) all of them, including workbooks never published; every
-- other signed-in user only those that became a publication, which every desk reads anyway.
ALTER TABLE ftp_uploads ENABLE ROW LEVEL SECURITY;
ALTER TABLE ftp_uploads FORCE ROW LEVEL SECURITY;
CREATE POLICY fu_admin ON ftp_uploads TO CURRENT_USER USING (true) WITH CHECK (true);
CREATE POLICY fu_read ON ftp_uploads FOR SELECT TO treasury_app USING (
    current_app_user() IS NOT NULL
    AND (can_see_alm() OR EXISTS (SELECT 1 FROM ftp_publications p WHERE p.upload_id = ftp_uploads.id))
);
GRANT SELECT ON ftp_uploads TO treasury_app;
GRANT SELECT ON ftp_uploads TO treasury_recon;
CREATE POLICY fu_recon ON ftp_uploads FOR SELECT TO treasury_recon USING (true);

-- Recording a drop. The fingerprint and size are computed here from the bytes, so the log cannot
-- claim a file it does not hold. Dropping the same file again unpublished reuses its entry.
CREATE FUNCTION ftp_upload(p_file_name text, p_content bytea, p_sheet_date date, p_currencies text[], p_errors integer, p_warnings integer, p_via text)
    RETURNS integer LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    u      uuid := current_app_user();
    v_sha  text;
    v_id   integer;
BEGIN
    IF u IS NULL OR NOT can_see_alm() THEN
        RAISE EXCEPTION 'only ALM may publish the FTP curves' USING ERRCODE = '42501';
    END IF;
    IF p_content IS NULL OR octet_length(p_content) = 0 THEN
        RAISE EXCEPTION 'the file is empty';
    END IF;
    v_sha := encode(sha256(p_content), 'hex');
    SELECT f.id INTO v_id FROM ftp_uploads f
     WHERE f.uploaded_by = u AND f.file_sha256 = v_sha AND f.file_name = trim(p_file_name)
       AND f.uploaded_at > now() - interval '1 day'
       AND NOT EXISTS (SELECT 1 FROM ftp_publications p WHERE p.upload_id = f.id)
     ORDER BY f.id DESC LIMIT 1;
    IF v_id IS NOT NULL THEN
        UPDATE ftp_uploads SET uploaded_at = now() WHERE id = v_id;
        RETURN v_id;
    END IF;
    INSERT INTO ftp_uploads (uploaded_by, via, file_name, file_sha256, size_bytes, content, sheet_date, currencies, errors, warnings)
    VALUES (u, coalesce(p_via, 'browser'), coalesce(nullif(trim(p_file_name), ''), 'workbook.xlsx'), v_sha, octet_length(p_content), p_content,
            p_sheet_date, coalesce(p_currencies, '{}'), coalesce(p_errors, 0), coalesce(p_warnings, 0))
    RETURNING id INTO v_id;
    RETURN v_id;
END
$$;
REVOKE EXECUTE ON FUNCTION ftp_upload(text, bytea, date, text[], integer, integer, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION ftp_upload(text, bytea, date, text[], integer, integer, text) TO treasury_app;

-- The shared part of publishing: supersede the day's current publication, insert the new one with
-- its points, and record on the old one when and by which publication it was replaced.
CREATE FUNCTION ftp_publish_points(p_as_of date, p_points jsonb, p_file_name text, p_file_sha256 text, p_note text, p_upload_id integer)
    RETURNS integer LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    u      uuid := current_app_user();
    v_id   integer;
    v_old  integer;
    n      integer;
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
    SELECT id INTO v_old FROM ftp_publications WHERE as_of = p_as_of AND is_current FOR UPDATE;
    UPDATE ftp_publications SET is_current = false, replaced_at = now() WHERE id = v_old;
    INSERT INTO ftp_publications (as_of, published_by, file_name, file_sha256, note, upload_id)
    VALUES (p_as_of, u, nullif(trim(p_file_name), ''), nullif(p_file_sha256, ''), trim(p_note), p_upload_id)
    RETURNING ftp_publications.id INTO v_id;
    UPDATE ftp_publications SET replaced_by_id = v_id WHERE id = v_old;
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
REVOKE EXECUTE ON FUNCTION ftp_publish_points(date, jsonb, text, text, text, integer) FROM PUBLIC;

-- Publishing without a stored file (kept for callers that send the curves alone).
CREATE OR REPLACE FUNCTION ftp_publish(p_as_of date, p_points jsonb, p_file_name text, p_file_sha256 text, p_note text) RETURNS integer
    LANGUAGE sql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
    SELECT ftp_publish_points(p_as_of, p_points, p_file_name, p_file_sha256, p_note, NULL)
$$;

-- Publishing a dropped workbook: only the person who dropped it, and only one that read cleanly.
CREATE FUNCTION ftp_publish(p_as_of date, p_points jsonb, p_upload_id integer, p_note text) RETURNS integer
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    f ftp_uploads%ROWTYPE;
BEGIN
    SELECT * INTO f FROM ftp_uploads WHERE id = p_upload_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'that workbook is not in the upload log; drop it again';
    END IF;
    IF f.uploaded_by IS DISTINCT FROM current_app_user() THEN
        RAISE EXCEPTION 'only the person who dropped a workbook may publish it' USING ERRCODE = '42501';
    END IF;
    IF f.errors > 0 THEN
        RAISE EXCEPTION 'that workbook has % errors to fix first', f.errors;
    END IF;
    RETURN ftp_publish_points(p_as_of, p_points, f.file_name, f.file_sha256, p_note, f.id);
END
$$;
REVOKE EXECUTE ON FUNCTION ftp_publish(date, jsonb, integer, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION ftp_publish(date, jsonb, integer, text) TO treasury_app;
