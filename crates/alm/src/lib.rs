//! Asset and liability management maths over the banking-book cohorts in `domain::alm`. Pure:
//! no I/O, no database, no clock. The server runs these on snapshots it loads; the client runs the
//! ones that need only market data.

pub mod lcr;
pub mod schedule;

pub use lcr::compute as lcr;
