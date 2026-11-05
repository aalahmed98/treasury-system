//! Reconciliation core. Pure modules first (no I/O): calendar, clock, tolerances, normaliser,
//! matcher decision and SLA maths. `pipeline` applies decisions to Postgres; `synth` generates a
//! world and per-source observations for development and tests.

pub mod alm_synth;
pub mod calendar;
pub mod clock;
pub mod ingest;
pub mod matcher;
pub mod normalise;
pub mod nostro;
pub mod pipeline;
pub mod pmu_synth;
pub mod sla;
pub mod store;
pub mod synth;
pub mod tolerance;
