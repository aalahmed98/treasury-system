//! Asset and liability management maths over the banking-book cohorts in `domain::alm`. Pure:
//! no I/O, no database, no clock. The server runs these on snapshots it loads; the client runs the
//! ones that need only market data.

pub mod attribution;
pub mod bucket;
pub mod curve;
pub mod eve;
pub mod funding;
pub mod gap;
pub mod ird;
pub mod lcr;
pub mod nii;
pub mod nsfr;
pub mod schedule;
pub mod stress;

pub use lcr::compute as lcr;
