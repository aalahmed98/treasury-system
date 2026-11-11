-- Saved ALM simulations: a rate shock, the what-if assumptions as changes against the bank's
-- parameters, and trial hedges, kept under a name. A scenario is its owner's alone until they
-- share it with the ALM desk; a shared one may be put in the ALCO pack. Nothing here is an
-- official figure: a scenario is a recipe that is recomputed on the snapshot it is opened on.

CREATE TABLE alm_scenarios (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name        text NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
    owner       uuid NOT NULL REFERENCES users(id),
    shared      boolean NOT NULL DEFAULT false,
    in_pack     boolean NOT NULL DEFAULT false,
    body        jsonb NOT NULL CHECK (jsonb_typeof(body) = 'object'),
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now(),
    CHECK (shared OR NOT in_pack)
);
CREATE UNIQUE INDEX alm_scenarios_owner_name ON alm_scenarios (owner, lower(name));

-- ---------------------------------------------------------------------------
-- Row-level security: an ALM user reads their own scenarios and the shared ones. Writes only
-- through the definer functions below.
-- ---------------------------------------------------------------------------
ALTER TABLE alm_scenarios ENABLE ROW LEVEL SECURITY;
ALTER TABLE alm_scenarios FORCE ROW LEVEL SECURITY;
CREATE POLICY asc_admin ON alm_scenarios TO CURRENT_USER USING (true) WITH CHECK (true);
CREATE POLICY asc_read ON alm_scenarios FOR SELECT TO treasury_app USING (can_see_alm() AND (shared OR owner = current_app_user()));
GRANT SELECT ON alm_scenarios TO treasury_app;

-- Save a new scenario (no id) or replace one of the caller's own.
CREATE FUNCTION alm_scenario_save(p_id uuid, p_name text, p_body jsonb) RETURNS uuid
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    u   uuid := current_app_user();
    n   text := trim(coalesce(p_name, ''));
    v   uuid;
BEGIN
    IF u IS NULL OR NOT can_see_alm() THEN
        RAISE EXCEPTION 'not entitled to save ALM scenarios' USING ERRCODE = '42501';
    END IF;
    IF length(n) = 0 OR length(n) > 80 THEN
        RAISE EXCEPTION 'a scenario needs a name of up to 80 characters';
    END IF;
    IF p_body IS NULL OR jsonb_typeof(p_body) <> 'object' THEN
        RAISE EXCEPTION 'a scenario must be a JSON object';
    END IF;
    IF EXISTS (SELECT 1 FROM alm_scenarios WHERE owner = u AND lower(name) = lower(n) AND id IS DISTINCT FROM p_id) THEN
        RAISE EXCEPTION 'you already have a scenario called %', n;
    END IF;
    IF p_id IS NULL THEN
        IF (SELECT count(*) FROM alm_scenarios WHERE owner = u) >= 50 THEN
            RAISE EXCEPTION 'fifty saved scenarios at most; delete one first';
        END IF;
        INSERT INTO alm_scenarios (name, owner, body) VALUES (n, u, p_body) RETURNING id INTO v;
    ELSE
        UPDATE alm_scenarios SET name = n, body = p_body, updated_at = now() WHERE id = p_id AND owner = u RETURNING id INTO v;
        IF v IS NULL THEN
            RAISE EXCEPTION 'no scenario of yours with that id' USING ERRCODE = 'P0002';
        END IF;
    END IF;
    RETURN v;
END
$$;
REVOKE EXECUTE ON FUNCTION alm_scenario_save(uuid, text, jsonb) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION alm_scenario_save(uuid, text, jsonb) TO treasury_app;

-- Share one of the caller's own with the desk, or take it back (which also takes it out of the pack).
CREATE FUNCTION alm_scenario_share(p_id uuid, p_shared boolean) RETURNS void
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
    IF current_app_user() IS NULL OR NOT can_see_alm() THEN
        RAISE EXCEPTION 'not entitled to share ALM scenarios' USING ERRCODE = '42501';
    END IF;
    UPDATE alm_scenarios SET shared = p_shared, in_pack = in_pack AND p_shared, updated_at = now() WHERE id = p_id AND owner = current_app_user();
    IF NOT FOUND THEN
        RAISE EXCEPTION 'no scenario of yours with that id' USING ERRCODE = 'P0002';
    END IF;
END
$$;
REVOKE EXECUTE ON FUNCTION alm_scenario_share(uuid, boolean) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION alm_scenario_share(uuid, boolean) TO treasury_app;

-- Put a shared scenario in the ALCO pack, or take it out. Anyone on the ALM desk may.
CREATE FUNCTION alm_scenario_pack(p_id uuid, p_in_pack boolean) RETURNS void
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
    IF current_app_user() IS NULL OR NOT can_see_alm() THEN
        RAISE EXCEPTION 'not entitled to the ALCO pack' USING ERRCODE = '42501';
    END IF;
    IF p_in_pack AND (SELECT count(*) FROM alm_scenarios WHERE in_pack AND id <> p_id) >= 6 THEN
        RAISE EXCEPTION 'six scenarios in the ALCO pack at most; take one out first';
    END IF;
    UPDATE alm_scenarios SET in_pack = p_in_pack, updated_at = now() WHERE id = p_id AND shared;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'only a shared scenario can go in the ALCO pack';
    END IF;
END
$$;
REVOKE EXECUTE ON FUNCTION alm_scenario_pack(uuid, boolean) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION alm_scenario_pack(uuid, boolean) TO treasury_app;

CREATE FUNCTION alm_scenario_delete(p_id uuid) RETURNS void
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
    IF current_app_user() IS NULL OR NOT can_see_alm() THEN
        RAISE EXCEPTION 'not entitled to delete ALM scenarios' USING ERRCODE = '42501';
    END IF;
    DELETE FROM alm_scenarios WHERE id = p_id AND owner = current_app_user();
    IF NOT FOUND THEN
        RAISE EXCEPTION 'no scenario of yours with that id' USING ERRCODE = 'P0002';
    END IF;
END
$$;
REVOKE EXECUTE ON FUNCTION alm_scenario_delete(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION alm_scenario_delete(uuid) TO treasury_app;
