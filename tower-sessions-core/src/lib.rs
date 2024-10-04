//! An abstraction over session storage and retrieval through [`SessionStore`].
//!
//! Sessions are identified by a unique [`Id`] and can have an [`Expiry`] with the [`Expires`]
//! trait.
#[doc(inline)]
pub use self::session_store::SessionStore;
pub use self::id::Id;
pub use self::expires::{Expires, Expiry};

/// A trait for session storage and retrieval.
pub mod session_store;
/// Session expiry configuration.
pub mod expires;
/// Session IDs.
pub mod id;
