//! Deposits with no contractual maturity: how the balance is taken to behave when rates are
//! measured. A non-core share can leave or reprice at once and sits overnight; the core share
//! stays, running off over the product's behavioural life, evenly or by a yearly decay with
//! whatever is left at the end of the life falling due then. With no core share and no decay set
//! the whole balance runs off evenly, which is how the gap and EVE have always slotted it.
//!
//! Under a rate rise part of the balance may also migrate into term deposits: that share stops
//! behaving like an account and becomes a fixed deposit at the migration term.

use domain::{AlmParams, BsPosition, BsSide, RateType};

/// The behavioural life of a cohort in months: the assumption for its product, else the cohort's
/// own, else one month.
pub fn life_months(pos: &BsPosition, params: &AlmParams) -> i64 {
    params.gap.nmd_behavioural_months.get(&pos.product).copied().or(pos.behavioural_months.map(|m| m as f64)).unwrap_or(1.0).max(1.0).round() as i64
}

/// The balance by when it reprices: `None` is overnight (the non-core share), `Some(m)` the
/// month `m` from the snapshot. The amounts add up to the balance.
pub fn profile(pos: &BsPosition, params: &AlmParams) -> Vec<(Option<i64>, f64)> {
    let months = life_months(pos, params);
    let core_share = params.gap.nmd_core_pct.get(&pos.product).copied().unwrap_or(100.0).clamp(0.0, 100.0) / 100.0;
    let decay = params.gap.nmd_decay_pct.get(&pos.product).copied().unwrap_or(0.0).clamp(0.0, 99.0) / 100.0;
    let core = pos.balance * core_share;
    let mut out = Vec::with_capacity(months as usize + 1);
    if core_share < 1.0 {
        out.push((None, pos.balance - core));
    }
    if core_share <= 0.0 {
        return out;
    }
    if decay <= 0.0 {
        let slice = core / months as f64;
        out.extend((0..months).map(|m| (Some(m), slice)));
        return out;
    }
    let left = |m: i64| (1.0 - decay).powf(m as f64 / 12.0);
    for m in 0..months {
        let amount = if m + 1 == months { core * left(m) } else { core * (left(m) - left(m + 1)) };
        out.push((Some(m), amount));
    }
    out
}

/// The tenor in years at which a move in short rates is read, for betas and for migration.
pub const SHORT_YEARS: f64 = 1.0 / 12.0;

/// Deposit migration: the share of a non-maturity deposit that moves into term deposits when the
/// short end rises by `short_move_pct` percent. So much per 100 basis points for its product, up
/// to the cap; nothing for a fall, for assets, or for a product with no rate set.
pub fn migrated_share(pos: &BsPosition, params: &AlmParams, short_move_pct: f64) -> f64 {
    if pos.rate_type != RateType::Admin || pos.side != BsSide::Liability || short_move_pct <= 0.0 {
        return 0.0;
    }
    let per_100bp = params.behaviour.migration_pct_per_100bp.get(&pos.product).copied().unwrap_or(0.0) / 100.0;
    (per_100bp * short_move_pct).clamp(0.0, params.behaviour.migration_cap_pct.clamp(0.0, 100.0) / 100.0)
}

/// The term the moved balance is placed for, in years.
pub fn migration_term_years(params: &AlmParams) -> f64 {
    params.behaviour.migration_term_months.clamp(1, 120) as f64 / 12.0
}
