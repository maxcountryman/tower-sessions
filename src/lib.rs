//! # Overview
//!
//! This crate provides sessions, key-value pairs associated with a site
//! visitor, as a [`tower`](https://docs.rs/tower/latest/tower/) middleware.
//!
//! It offers:
//!
//! - **Pluggable Storage Backends:** Bring your own backend simply by
//!   implementing the [`SessionStore`] trait, fully decoupling sessions from
//!   their storage.
//! - **Minimal Overhead**: Sessions are only loaded from their backing stores
//!   when they're actually used and only in e.g. the handler they're used in.
//!   That means this middleware can be installed at any point in your route
//!   graph with minimal overhead.
//! - **An `axum` Extractor for [`Session`]:** Applications built with `axum`
//!   can use `Session` as an extractor directly in their handlers. This makes
//!   using sessions as easy as including `Session` in your handler.
//! - **Simple Key-Value Interface:** Sessions offer a key-value interface that
//!   supports native Rust types. So long as these types are `Serialize` and can
//!   be converted to JSON, it's straightforward to insert, get, and remove any
//!   value.
//! - **Strongly-Typed Sessions:** Strong typing guarantees are easy to layer on
//!   top of this foundational key-value interface.
//!
//! This crate's session implementation is inspired by the [Django sessions middleware](https://docs.djangoproject.com/en/4.2/topics/http/sessions) and it provides a transliteration of those semantics.
//! ### Session stores
//!
//! Session data persistence is managed by user-provided types that implement
//! [`SessionStore`]. What this means is that applications can and should
//! implement session stores to fit their specific needs.
//!
//! That said, a number of session store implmentations already exist and may be
//! useful starting points.
//!
//! | Crate                                                                                                            | Persistent | Description                                |
//! | ---------------------------------------------------------------------------------------------------------------- | ---------- | ------------------------------------------ |
//! | [`tower-sessions-dynamodb-store`](https://github.com/necrobious/tower-sessions-dynamodb-store)                   | Yes        | DynamoDB session store                     |
//! | [`tower-sessions-firestore-store`](https://github.com/AtTheTavern/tower-sessions-firestore-store)                | Yes        | Firestore session store                    |
//! | [`tower-sessions-libsql-store`](https://github.com/daybowbow-dev/tower-sessions-libsql-store)                    | Yes        | libSQL session store                       |
//! | [`tower-sessions-mongodb-store`](https://github.com/maxcountryman/tower-sessions-stores/tree/main/mongodb-store) | Yes        | MongoDB session store                      |
//! | [`tower-sessions-moka-store`](https://github.com/maxcountryman/tower-sessions-stores/tree/main/moka-store)       | No         | Moka session store                         |
//! | [`tower-sessions-redis-store`](https://github.com/maxcountryman/tower-sessions-stores/tree/main/redis-store)     | Yes        | Redis via `fred` session store             |
//! | [`tower-sessions-rusqlite-store`](https://github.com/patte/tower-sessions-rusqlite-store)                        | Yes        | Rusqlite session store                     |
//! | [`tower-sessions-sled-store`](https://github.com/Zatzou/tower-sessions-sled-store)                               | Yes        | Sled session store                         |
//! | [`tower-sessions-sqlx-store`](https://github.com/maxcountryman/tower-sessions-stores/tree/main/sqlx-store)       | Yes        | SQLite, Postgres, and MySQL session stores |
//! | [`tower-sessions-surrealdb-store`](https://github.com/rynoV/tower-sessions-surrealdb-store)                      | Yes        | SurrealDB session store                    |
//!
//! Have a store to add? Please open a PR adding it.
//!
//! ### User session management
//!
//! To facilitate authentication and authorization, we've built [`axum-login`](https://github.com/maxcountryman/axum-login) on top of this crate. Please check it out if you're looking for a generalized auth solution.
//!
//! # Usage with an `axum` application
//!
//! A common use-case for sessions is when building HTTP servers. Using `axum`,
//! it's straightforward to leverage sessions.
//!
//! ```rust,no_run
//! use std::net::SocketAddr;
//!
//! use axum::{response::IntoResponse, routing::get, Router};
//! use serde::{Deserialize, Serialize};
//! use time::Duration;
//! use tower_sessions::{Expiry, MemoryStore, Session, SessionManagerLayer};
//!
//! const COUNTER_KEY: &str = "counter";
//!
//! #[derive(Default, Deserialize, Serialize)]
//! struct Counter(usize);
//!
//! async fn handler(session: Session) -> impl IntoResponse {
//!     let counter: Counter = session.get(COUNTER_KEY).await.unwrap().unwrap_or_default();
//!     session.insert(COUNTER_KEY, counter.0 + 1).await.unwrap();
//!     format!("Current count: {}", counter.0)
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     let session_store = MemoryStore::default();
//!     let session_layer = SessionManagerLayer::new(session_store)
//!         .with_secure(false)
//!         .with_expiry(Expiry::OnInactivity(Duration::seconds(10)));
//!
//!     let app = Router::new().route("/", get(handler)).layer(session_layer);
//!
//!     let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
//!     let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
//!     axum::serve(listener, app.into_make_service())
//!         .await
//!         .unwrap();
//! }
//! ```
//!
//! ## Session expiry management
//!
//! In cases where you are utilizing stores that lack automatic session expiry
//! functionality, such as SQLx or MongoDB stores, it becomes essential to
//! periodically clean up stale sessions. For instance, both SQLx and MongoDB
//! stores offer
//! `continuously_delete_expired`
//! which is designed to be executed as a recurring task. This process ensures
//! the removal of expired sessions, maintaining your application's data
//! integrity and performance.
//! ```rust,no_run,ignore
//! # use tower_sessions::{session_store::ExpiredDeletion};
//! # use tower_sessions_sqlx_store::{sqlx::SqlitePool, SqliteStore};
//! # tokio_test::block_on(async {
//! let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
//! let session_store = SqliteStore::new(pool);
//! let deletion_task = tokio::task::spawn(
//!     session_store
//!         .clone()
//!         .continuously_delete_expired(tokio::time::Duration::from_secs(60)),
//! );
//! deletion_task.await.unwrap().unwrap();
//! # });
//! ```
//!
//! Note that by default or when using browser session expiration, sessions are
//! considered expired after two weeks.
//!
//! # Extractor pattern
//!
//! When using `axum`, the [`Session`] will already function as an extractor.
//! It's possible to build further on this to create extractors of custom types.
//! ```rust,no_run
//! # use async_trait::async_trait;
//! # use axum::extract::FromRequestParts;
//! # use http::{request::Parts, StatusCode};
//! # use serde::{Deserialize, Serialize};
//! # use tower_sessions::{SessionStore, Session, MemoryStore};
//! const COUNTER_KEY: &str = "counter";
//!
//! #[derive(Default, Deserialize, Serialize)]
//! struct Counter(usize);
//!
//! #[async_trait]
//! impl<S> FromRequestParts<S> for Counter
//! where
//!     S: Send + Sync,
//! {
//!     type Rejection = (http::StatusCode, &'static str);
//!
//!     async fn from_request_parts(req: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
//!         let session = Session::from_request_parts(req, state).await?;
//!         let counter: Counter = session.get(COUNTER_KEY).await.unwrap().unwrap_or_default();
//!         session.insert(COUNTER_KEY, counter.0 + 1).await.unwrap();
//!
//!         Ok(counter)
//!     }
//! }
//! ```
//!
//! Now in our handler, we can use `Counter` directly to read its fields.
//!
//! A complete example can be found in [`examples/counter-extractor.rs`](https://github.com/maxcountryman/tower-sessions/blob/main/examples/counter-extractor.rs).
//!
//! # Strongly-typed sessions
//!
//! The extractor pattern can be extended further to provide strong typing
//! guarantees over the key-value substrate. Whereas our previous extractor
//! example was effectively read-only. This pattern enables mutability of the
//! underlying structure while also leveraging the full power of the type
//! system.
//! ```rust,no_run
//! # use async_trait::async_trait;
//! # use axum::extract::FromRequestParts;
//! # use http::{request::Parts, StatusCode};
//! # use serde::{Deserialize, Serialize};
//! # use time::OffsetDateTime;
//! # use tower_sessions::{SessionStore, Session};
//! #[derive(Clone, Deserialize, Serialize)]
//! struct GuestData {
//!     pageviews: usize,
//!     first_seen: OffsetDateTime,
//!     last_seen: OffsetDateTime,
//! }
//!
//! impl Default for GuestData {
//!     fn default() -> Self {
//!         Self {
//!             pageviews: 0,
//!             first_seen: OffsetDateTime::now_utc(),
//!             last_seen: OffsetDateTime::now_utc(),
//!         }
//!     }
//! }
//!
//! struct Guest {
//!     session: Session,
//!     guest_data: GuestData,
//! }
//!
//! impl Guest {
//!     const GUEST_DATA_KEY: &'static str = "guest_data";
//!
//!     fn first_seen(&self) -> OffsetDateTime {
//!         self.guest_data.first_seen
//!     }
//!
//!     fn last_seen(&self) -> OffsetDateTime {
//!         self.guest_data.last_seen
//!     }
//!
//!     fn pageviews(&self) -> usize {
//!         self.guest_data.pageviews
//!     }
//!
//!     async fn mark_pageview(&mut self) {
//!         self.guest_data.pageviews += 1;
//!         Self::update_session(&self.session, &self.guest_data).await
//!     }
//!
//!     async fn update_session(session: &Session, guest_data: &GuestData) {
//!         session
//!             .insert(Self::GUEST_DATA_KEY, guest_data.clone())
//!             .await
//!             .unwrap()
//!     }
//! }
//!
//! #[async_trait]
//! impl<S> FromRequestParts<S> for Guest
//! where
//!     S: Send + Sync,
//! {
//!     type Rejection = (StatusCode, &'static str);
//!
//!     async fn from_request_parts(req: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
//!         let session = Session::from_request_parts(req, state).await?;
//!
//!         let mut guest_data: GuestData = session
//!             .get(Self::GUEST_DATA_KEY)
//!             .await
//!             .unwrap()
//!             .unwrap_or_default();
//!
//!         guest_data.last_seen = OffsetDateTime::now_utc();
//!
//!         Self::update_session(&session, &guest_data).await;
//!
//!         Ok(Self {
//!             session,
//!             guest_data,
//!         })
//!     }
//! }
//! ```
//!
//! Here we can use `Guest` as an extractor in our handler. We'll be able to
//! read values, like the ID as well as update the pageview count with our
//! `mark_pageview` method.
//!
//! A complete example can be found in [`examples/strongly-typed.rs`](https://github.com/maxcountryman/tower-sessions/blob/main/examples/strongly-typed.rs)
//!
//! ## Name-spaced and strongly-typed buckets
//!
//! Our example demonstrates a single extractor, but in a real application we
//! might imagine a set of common extractors, all living in the same session.
//! Each extractor forms a kind of bucketed name-space with a typed structure.
//! Importantly, each is self-contained by its own name-space.
//!
//! For instance, we might also have a site preferences bucket, an analytics
//! bucket, a feature flag bucket and so on. All these together would live in
//! the same session, but would be segmented by their own name-space, avoiding
//! the mixing of domains unnecessarily.[^data-domains]
//!
//! # Layered caching
//!
//! In some cases, the canonical store for a session may benefit from a cache.
//! For example, rather than loading a session from a store on every request,
//! this roundtrip can be mitigated by placing a cache in front of the storage
//! backend. A specialized session store, [`CachingSessionStore`], is provided
//! for exactly this purpose.
//!
//! This store manages a cache and a store. Where the cache acts as a frontend
//! and the store a backend. When a session is loaded, the store first attempts
//! to load the session from the cache, if that fails only then does it try to
//! load from the store. By doing so, read-heavy workloads will incur far fewer
//! roundtrips to the store itself.
//!
//! To illustrate, this is how we might use the
//! `MokaStore` as a frontend cache to a
//! `PostgresStore` backend.
//! ```rust,no_run,ignore
//! # use tower::ServiceBuilder;
//! # use tower_sessions::{CachingSessionStore, SessionManagerLayer};
//! # use tower_sessions_sqlx_store::{sqlx::PgPool, PostgresStore};
//! # use tower_sessions_moka_store::MokaStore;
//! # use time::Duration;
//! # tokio_test::block_on(async {
//! let database_url = std::option_env!("DATABASE_URL").unwrap();
//! let pool = PgPool::connect(database_url).await.unwrap();
//!
//! let postgres_store = PostgresStore::new(pool);
//! postgres_store.migrate().await.unwrap();
//!
//! let moka_store = MokaStore::new(Some(10_000));
//! let caching_store = CachingSessionStore::new(moka_store, postgres_store);
//!
//! let session_service = ServiceBuilder::new()
//!     .layer(SessionManagerLayer::new(caching_store).with_max_age(Duration::days(1)));
//! # })
//! ```
//!
//! While this example uses Moka, any implementor of [`SessionStore`] may be
//! used. For instance, we could use the `RedisStore` instead of Moka.
//!
//! A cache is most helpful with read-heavy workloads, where the cache hit rate
//! will be high. This is because write-heavy workloads will require a roundtrip
//! to the store and therefore benefit less from caching.
//!
//! ## Data races under concurrent conditions
//!
//! Please note that it is **not safe** to access and mutate session state
//! concurrently: this will result in data loss if your mutations are dependent
//! on the state of the session.
//!
//! This is because a session is loaded first from its backing store. Once
//! loaded it's possible for a second request to load the same session, but
//! without the inflight changes the first request may have made.
//!
//! # Implementation
//!
//! Sessions are composed of three pieces:
//!
//! 1. A cookie that holds the session ID as its value,
//! 2. An in-memory hash-map, which underpins the key-value API,
//! 3. A pluggable persistence layer, the session store, where session data is
//!    housed.
//!
//! Together, these pieces form the basis of this crate and allow `tower` and
//! `axum` applications to use a familiar session interface.
//!
//! ## Cookie
//!
//! Sessions manifest to clients as cookies. These cookies have a configurable
//! name and a value that is the session ID. In other words, cookies hold a
//! pointer to the session in the form of an ID. This ID is an i128 generated by
//! the [`rand`](https://docs.rs/rand/latest/rand) crate.
//!
//! ### Secure nature of cookies
//!
//! Session IDs are considered secure if sent over encrypted channels. Note that
//! this assumption is predicated on the secure nature of the [`rand`](https://docs.rs/rand/latest/rand) crate
//! and its ability to generate securely-random values using the ChaCha block
//! cipher with 12 rounds. It's also important to note that session cookies
//! **must never** be sent over a public, insecure channel. Doing so is **not**
//! secure and will lead to compromised sessions!
//!
//! Additionally, sessions may be optionally signed or encrypted by enabling the
//! `signed` and `private` feature flags, respectively. When enabled, the
//! [`with_signed`](SessionManagerLayer::with_signed) and
//! [`with_private`](SessionManagerLayer::with_private) methods become
//! available. These methods take a cryptographic key which allows the session
//! manager to leverage ciphertext as opposed to the default of plaintext. Note
//! that no data is stored in the session ID beyond the session identifier
//! itself and so this measure should be considered primarily effective as a
//! defense in depth tactic.
//!
//! ## Key-value API
//!
//! Sessions manage a `HashMap<String, serde_json::Value>` but importantly are
//! transparently persisted to an arbitrary storage backend. Effectively,
//! `HashMap` is an intermediary, in-memory representation. By using a map-like
//! structure, we're able to present a familiar key-value interface for managing
//! sessions. This allows us to store and retrieve native Rust types, so long as
//! our type is `impl Serialize` and can be represented as JSON.[^json]
//!
//! Internally, this hash map state is protected by a lock in the form of
//! `Mutex`. This allows us to safely share mutable state across thread
//! boundaries. Note that this lock is only acquired when we read from or write
//! to this inner session state and not used when the session is provided to the
//! request. This means that lock contention is minimized for most use
//! cases.[^lock-contention]
//!
//! ## Session store
//!
//! Sessions are serialized to arbitrary storage backends via a session record
//! intermediary. Implementations of `SessionStore` take a record and persist
//! it such that it can later be loaded via the session ID.
//!
//! Three components are needed for storing a session:
//!
//! 1. The session ID.
//! 2. The session expiry.
//! 3. The session data itself.
//!
//! Together, these compose the session record and are enough to both encode and
//! decode a session from any backend.
//!
//! ## Session life cycle
//!
//! Cookies hold a pointer to the session, rather than the session's data, and
//! because of this, the `tower` middleware is focused on managing the process
//! of hydrating a session from the store and managing its life cycle.
//!
//! We load a session by looking for a cookie that matches our configured
//! session cookie name. If no such cookie is found or a cookie is found but the
//! store has no such session or the session is no longer active, we create a
//! new session.
//!
//! It's important to note that creating a session **does not** save the session
//! to the store. In fact, the session store is not used at all unless the
//! session is read from or written to. In other words, the middleware only
//! introduces session store overhead when the session is actually used.
//!  
//! Modified sessions will invoke the session store's
//! [`save`](SessionStore::save) method as well as send a `Set-Cookie` header.
//! While deleted sessions will either be:
//!
//! 1. Deleted, invoking the [`delete`](SessionStore::delete) method and setting
//!    a removal cookie or,
//! 2. Cycled, invoking the `delete` method but setting a new ID on the session;
//!    the session will have been marked as modified and so this will also set a
//!    `Set-Cookie` header on the response.
//!
//! Empty sessions are considered to be deleted and are removed from the session
//! store as well as the user agent.
//!
//! Sessions also carry with them a configurable expiry and will be deleted in
//! accordance with this.
//!
//! [^json]: Using JSON allows us to translate arbitrary types to virtually
//! any backend and gives us a nice interface with which to interact with the
//! session.
//!
//! [^lock-contention]: We might consider replacing `Mutex` with `RwLock` if
//! this proves to be a better fit in practice. Another alternative might be
//! `dashmap` or a different approach entirely. Future iterations should be
//! based on real-world use cases.
//!
//! [^data-domains]: This is particularly useful when we may have data
//! domains that only belong with ! users in certain states: we can pull these
//! into our handlers where we need a particular domain. In this way, we
//! minimize data pollution via self-contained domains in the form of buckets.
#![warn(
    clippy::all,
    nonstandard_style,
    future_incompatible,
    missing_debug_implementations
)]
#![deny(missing_docs)]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub use tower_cookies::cookie;
pub use tower_sessions_core::{session, session_store};
#[doc(inline)]
pub use tower_sessions_core::{
    session::{Expiry, Session},
    session_store::{CachingSessionStore, ExpiredDeletion, SessionStore},
};
#[cfg(feature = "memory-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "memory-store")))]
#[doc(inline)]
pub use tower_sessions_memory_store::MemoryStore;

pub use crate::service::{SessionManager, SessionManagerLayer};

pub mod service;
