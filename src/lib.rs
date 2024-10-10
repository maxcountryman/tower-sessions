#![doc = include_str!("../README.md")]

#![warn(
    clippy::all,
    nonstandard_style,
    future_incompatible,
    missing_debug_implementations
)]
#![deny(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub use tower_sessions_core::session_store;
#[doc(inline)]
pub use tower_sessions_core::{
    id::Id,
    expires::{Expires, Expiry},
    session_store::{CachingSessionStore, SessionStore},
};
#[cfg(feature = "memory-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "memory-store")))]
#[doc(inline)]
pub use tower_sessions_memory_store::MemoryStore;

pub use crate::middleware::{SessionManager, SessionManagerLayer};
pub use crate::session::{Session, SessionState};

pub mod middleware;
pub mod session;
