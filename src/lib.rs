//! # Overview
//!
//! This crate provides cookie-based sessions as a [`tower`] middleware.
//!
//! Session data is stored in a session store backend, which can be anything
//! that implements [`SessionStore`]. A pointer to this record is kept in the
//! cookie in the form of a UUID v4 identifier.
//!
//! A [`Session`] is provided as a request extension and applications may make
//! use of its interface by inserting, getting, and removing data associated
//! with a visitor. When using [`axum`] an extractor is provided, making
//! session retrieval in the route straightforward.
//!
//! ## Session Life Cycle
//!
//! Sessions are only saved when their internal state has been changed or their
//! life cycle has progressed, such as upon deletion or ID cycling. This
//! helps reduce unnecessary overhead.
//!
//! Further an expiration or no expiration may be provided. In the latter case,
//! the session will be treated as a "session cookie", meaning that the cookie
//! is meant to expire once the browser is closed.
//!
//! ## Backend Stores
//!
//! Stores persist the session's data. Many production use cases will require
//! this be a database of some kind. Redis and SQL stores are provided by
//! enabling the corresponding feature flags.
//!
//! However, custom stores may be implemented and indeed anything that
//! implements `SessionStore` may be used to house the backing session data.
//!
//! For testing, an in-memory store is also provided. Please note, this should
//! generally not be used in production applications.
//!
//! # Example
//!
//! This example demonstrates how you use the middleware with `axum`.
//!
//! ```rust,no_run
//! use std::net::SocketAddr;
//!
//! use axum::{
//!     error_handling::HandleErrorLayer, response::IntoResponse, routing::get, BoxError, Router,
//! };
//! use http::StatusCode;
//! use serde::{Deserialize, Serialize};
//! use tower::ServiceBuilder;
//! use tower_sessions::{time::Duration, MemoryStore, Session, SessionManagerLayer};
//!
//! #[derive(Default, Deserialize, Serialize)]
//! struct Counter(usize);
//!
//! # #[cfg(feature = "axum-core")]
//! #[tokio::main]
//! async fn main() {
//!     let session_store = MemoryStore::default();
//!     let session_service = ServiceBuilder::new()
//!         .layer(HandleErrorLayer::new(|_: BoxError| async {
//!             StatusCode::BAD_REQUEST
//!         }))
//!         .layer(
//!             SessionManagerLayer::new(session_store)
//!                 .with_secure(false)
//!                 .with_max_age(Duration::seconds(10)),
//!         );
//!
//!     let app = Router::new()
//!         .route("/", get(handler))
//!         .layer(session_service);
//!
//!     let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
//!     axum::Server::bind(&addr)
//!         .serve(app.into_make_service())
//!         .await
//!         .unwrap();
//! }
//! # #[cfg(not(feature = "axum-core"))]
//! # fn main() {}
//!
//! async fn handler(session: Session) -> impl IntoResponse {
//!     let counter: Counter = session
//!         .get("counter")
//!         .expect("Could not deserialize")
//!         .unwrap_or_default();
//!
//!     session
//!         .insert("counter", counter.0 + 1)
//!         .expect("Could not serialize.");
//!
//!     format!("Current count: {}", counter.0)
//! }
//! ```
//!
//! [`tower`]: https://docs.rs/tower/latest/tower/
//! [`axum`]: https://docs.rs/axum/latest/axum/

#![warn(clippy::all, missing_docs, nonstandard_style, future_incompatible)]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "redis-store")]
pub use fred;
#[cfg(feature = "sqlite-store")]
pub use sqlx;
pub use time;

#[cfg(feature = "memory-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "memory-store")))]
pub use self::memory_store::MemoryStore;
#[cfg(feature = "redis-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "redis-store")))]
pub use self::redis_store::RedisStore;
#[cfg(feature = "sqlite-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "sqlite-store")))]
pub use self::sqlite_store::SqliteStore;
#[doc(inline)]
pub use self::{
    cookie_config::CookieConfig,
    service::{SessionManager, SessionManagerLayer},
    session::Session,
    session_store::SessionStore,
};

#[cfg(feature = "axum-core")]
#[cfg_attr(docsrs, doc(cfg(feature = "axum-core")))]
mod extract;

#[cfg(feature = "memory-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "memory-store")))]
mod memory_store;

#[cfg(feature = "redis-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "redis-store")))]
mod redis_store;

#[cfg(feature = "sqlite-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "sqlite-store")))]
mod sqlite_store;

pub mod cookie_config;
pub mod service;
pub mod session;
pub mod session_store;
