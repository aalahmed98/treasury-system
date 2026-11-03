//! Types shared between the treasury server, the reconciler and the desktop client.
//! No I/O, no framework dependencies: only data shapes and their serialisation.

#[macro_use]
mod macros;

pub mod api;
pub mod deal;
pub mod desk;
pub mod events;
pub mod recon;
pub mod user;

pub use api::*;
pub use deal::*;
pub use desk::*;
pub use events::*;
pub use recon::*;
pub use user::*;
