//! Types shared between the treasury server, the reconciler and the desktop client.
//! No I/O, no framework dependencies: only data shapes and their serialisation.

#[macro_use]
mod macros;

pub mod activity;
pub mod alm;
pub mod api;
pub mod calendar;
pub mod deal;
pub mod desk;
pub mod events;
pub mod liquidity;
pub mod recon;
pub mod risk;
pub mod user;

pub use activity::*;
pub use alm::*;
pub use api::*;
pub use calendar::*;
pub use deal::*;
pub use desk::*;
pub use events::*;
pub use liquidity::*;
pub use recon::*;
pub use risk::*;
pub use user::*;
