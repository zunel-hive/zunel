//! AWS SSO credential auto-refresh helpers used by `zunel gateway`.
//!
//! See [`sso_refresh`] for the public API. The crate intentionally
//! exposes nothing else: it is a thin wrapper around `aws configure
//! export-credentials`, sized to mirror the
//! [`zunel_channels::slack::bot_refresh`] precedent.

pub mod sso_refresh;

pub use sso_refresh::{
    refresh_profile_if_near_expiry, RefreshContext, RefreshError, RefreshOutcome, RefreshResult,
};
