//! Types shared between the treasury server and the desktop client.
//! No I/O, no framework dependencies: only data shapes and their serialisation.

pub mod api;
pub mod deal;
pub mod desk;
pub mod events;
pub mod user;

pub use api::*;
pub use deal::*;
pub use desk::*;
pub use events::*;
pub use user::*;
