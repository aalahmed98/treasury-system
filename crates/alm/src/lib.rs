//! Asset and liability management maths over the banking-book cohorts in `domain::alm`. Pure:
//! no I/O, no database, no clock. The server runs these on snapshots it loads; the client runs the
//! ones that need only market data.

pub mod attribution;
pub mod behaviour;
pub mod bucket;
pub mod coverage;
pub mod curve;
pub mod eve;
pub mod ewi;
pub mod funding;
pub mod fx;
pub mod gap;
pub mod hedge;
pub mod ird;
pub mod lcr;
pub mod liquidity;
pub mod nii;
pub mod nmd;
pub mod nsfr;
pub mod ratios;
pub mod schedule;
pub mod stress;

pub use lcr::compute as lcr;
