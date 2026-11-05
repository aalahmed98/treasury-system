-- Deals are booked on the dealing desks only. The FX desk books FX products and the money-market
-- desk money-market products; the PMU desk's purchases, swaps and repos are its own records
-- (0007), and ALM and the head do not deal. Before this, a user with booking rights on any desk
-- (pmu1 on PMU, for its future tickets) could put an FX spot on that desk.
--
-- Rows from the sources (origin other than INTERNAL) keep the desk the mapping gave them and are
-- checked only for the non-dealing desks, so a feed can never land a deal on PMU, ALM or HEAD.

CREATE FUNCTION deals_desk_guard() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE
    code text;
BEGIN
    SELECT k.code INTO code FROM desks k WHERE k.id = NEW.desk_id;
    IF code IN ('PMU', 'ALM', 'HEAD') THEN
        RAISE EXCEPTION 'not entitled to book deals on the % desk: deals are booked on the FX and money-market desks', code
            USING ERRCODE = '42501';
    END IF;
    IF NEW.origin = 'INTERNAL' AND (
           (code = 'FX' AND NEW.product NOT IN ('FX_SPOT', 'FX_FWD'))
        OR (code = 'MM' AND NEW.product NOT IN ('MM_DEPO', 'MM_LOAN'))) THEN
        RAISE EXCEPTION 'the % desk does not book %', code, NEW.product;
    END IF;
    RETURN NEW;
END
$$;

CREATE TRIGGER deals_desk_guard BEFORE INSERT OR UPDATE OF desk_id, product ON deals
    FOR EACH ROW EXECUTE FUNCTION deals_desk_guard();
