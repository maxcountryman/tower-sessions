pub use tower_cookies::cookie;

#[doc(inline)]
pub use self::{
    service::{SessionManager, SessionManagerLayer},
    session::{Expiry, Session},
    session_store::{CachingSessionStore, ExpiredDeletion, SessionStore},
};

#[cfg(feature = "axum-core")]
#[cfg_attr(docsrs, doc(cfg(feature = "axum-core")))]
pub mod extract;
pub mod service;
pub mod session;
pub mod session_store;
