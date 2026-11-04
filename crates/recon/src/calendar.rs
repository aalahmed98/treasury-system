//! Business-day arithmetic lives in `domain::calendar` so the server and the ALM crate can share it.
//! Re-exported here so existing `recon::calendar` paths keep working.

pub use domain::calendar::*;
