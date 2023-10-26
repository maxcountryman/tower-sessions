//! # Overview
//!
//! This crate provides sessions, key-value pairs associated with a site
//! visitor, as a [`tower`](https://docs.rs/tower/latest/tower/) middleware.
//!
//! It offers:
//!
//! - **Pluggable Storage Backends:** Arbitrary storage backends are implemented
//!   with the [`SessionStore`] trait, fully decoupling sessions from their
//!   storage.
//! - **An `axum` Extractor for [`Session`]:** Applications built with `axum`
//!   can use `Session` as an extractor directly in their handlers. This makes
//!   using sessions as easy as including `Session` in your handler.
//! - **Common Backends Out-of-the-Box:** [`RedisStore`], SQLx ([`SqliteStore`],
//!   [`PostgresStore`], [`MySqlStore`]), and [`MongoDBStore`] stores are
//!   available via their respective feature flags.
//! - **Layered Caching:** With [`CachingSessionStore`], applications can
//!   leverage a cache, such as [`MokaStore`], to reduce roundtrips to the store
//!   when loading sessions.
//! - **Simple Key-Value Interface:** Sessions offer a key-value interface that
//!   supports native Rust types. So long as these types are `Serialize` and can
//!   be converted to JSON, it's straightforward to insert, get, and remove any
//!   value.
//! - **Strongly-Typed Sessions:** Strong typing guarantees are easy to layer on
//!   top of this foundational key-value interface.
//!
//! # Usage with an `axum` application
//!
//! A common use-case for sessions is when building HTTP servers. Using `axum`,
//! it's straightforward to leverage sessions.
//!
//! ```rust,no_run
//! use std::net::SocketAddr;
//!
//! use axum::{
//!     error_handling::HandleErrorLayer, response::IntoResponse, routing::get, BoxError, Router,
//! };
//! use http::StatusCode;
//! use serde::{Deserialize, Serialize};
//! use time::Duration;
//! use tower::ServiceBuilder;
//! use tower_sessions::{MemoryStore, Session, SessionExpiry, SessionManagerLayer};
//!
//! const COUNTER_KEY: &str = "counter";
//!
//! #[derive(Default, Deserialize, Serialize)]
//! struct Counter(usize);
//!
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
//!                 .with_expiry(SessionExpiry::InactivityDuration(Duration::seconds(10))),
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
//!
//! async fn handler(session: Session) -> impl IntoResponse {
//!     let counter: Counter = session.get(COUNTER_KEY).unwrap().unwrap_or_default();
//!     session.insert(COUNTER_KEY, counter.0 + 1).unwrap();
//!     format!("Current count: {}", counter.0)
//! }
//! ```
//! ## Concurrent data access
//!
//! It's important to note that sessions can be accessed through concurrent
//! processes. What this means is that it's **not safe** to read from a session,
//! alter the read value, and write this modified value back to the session.
//! Making an update to the session in this way will lead to **data loss**.
//!
//! To perform such writes safely, it's important to use
//! [`replace_if_equal`](Session::replace_if_equal), which provides a [compare-and-swap](https://en.wikipedia.org/wiki/Compare-and-swap) interface for updating the session. For example, to safely
//! update our counter, we should instead use `replace_if_equal`.
//!
//! ```rust,no_run
//! # use axum::response::IntoResponse;
//! # use tower_sessions::Session;
//! # use serde::{Serialize, Deserialize};
//! # const COUNTER_KEY: &str = "counter";
//! # #[derive(Clone, Default, Deserialize, Serialize)]
//! # struct Counter(usize);
//! # tokio_test::block_on(async {
//! async fn handler(session: Session) -> impl IntoResponse {
//!     let mut counter: Counter = session.get(COUNTER_KEY).unwrap().unwrap_or_else(|| {
//!         let counter = Counter::default();
//!         session.insert(COUNTER_KEY, counter.clone()).unwrap();
//!         counter
//!     });
//!
//!     let mut new_counter = Counter(counter.0 + 1);
//!
//!     while let Ok(false) = session.replace_if_equal(COUNTER_KEY, counter, new_counter) {
//!         counter = session.get(COUNTER_KEY).unwrap().unwrap();
//!         new_counter = Counter(counter.0 + 1);
//!     }
//! }
//! # });
//! ```
//!
//! ## Session expiry management
//!
//! In cases where you are utilizing stores that lack automatic session expiry
//! functionality, such as SQLx or MongoDB stores, it becomes essential to
//! periodically clean up stale sessions. For instance, both SQLx and MongoDB
//! stores offer
//! [`continuously_delete_expired`](ExpiredDeletion::continuously_delete_expired)
//! which is designed to be executed as a recurring task. This process ensures
//! the removal of expired sessions, maintaining your application's data
//! integrity and performance.
//! ```rust,no_run
//! # #[cfg(all(feature = "sqlite-store", feature = "continuously-delete-expired"))] {
//! # use tower_sessions::{sqlx::SqlitePool, SqliteStore, session_store::ExpiredDeletion};
//! # tokio_test::block_on(async {
//! let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
//! let session_store = SqliteStore::new(pool);
//! let deletion_task = tokio::task::spawn(
//!     session_store
//!         .clone()
//!         .continuously_delete_expired(tokio::time::Duration::from_secs(60)),
//! );
//! deletion_task.await.unwrap().unwrap();
//! # })};
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
//! # use tower_sessions::Session;
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
//!         let counter: Counter = session.get(COUNTER_KEY).unwrap().unwrap_or_default();
//!         session.insert(COUNTER_KEY, counter.0 + 1).unwrap();
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
//! # use tower_sessions::Session;
//! # use uuid::Uuid;
//! #[derive(Clone, Deserialize, Serialize)]
//! struct GuestData {
//!     id: Uuid,
//!     pageviews: usize,
//!     first_seen: OffsetDateTime,
//!     last_seen: OffsetDateTime,
//! }
//!
//! impl Default for GuestData {
//!     fn default() -> Self {
//!         Self {
//!             id: Uuid::new_v4(),
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
//!     fn id(&self) -> Uuid {
//!         self.guest_data.id
//!     }
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
//!     fn mark_pageview(&mut self) {
//!         self.guest_data.pageviews += 1;
//!         Self::update_session(&self.session, &self.guest_data)
//!     }
//!
//!     fn update_session(session: &Session, guest_data: &GuestData) {
//!         session
//!             .insert(Self::GUEST_DATA_KEY, guest_data.clone())
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
//!             .unwrap()
//!             .unwrap_or_default();
//!
//!         guest_data.last_seen = OffsetDateTime::now_utc();
//!
//!         Self::update_session(&session, &guest_data);
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
//! To illustrate, this is how we might use the [`MokaStore`] as a frontend
//! cache to a [`PostgresStore`] backend.
//! ```rust,no_run
//! # #[cfg(all(feature = "moka_store", feature = "postgres_store"))] {
//! # use tower::ServiceBuilder;
//! # use tower_sessions::{
//! #    sqlx::PgPool, CachingSessionStore, MokaStore, PostgresStore, SessionManagerLayer,
//! # };
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
//! # })}
//! ```
//!
//! While this example uses Moka, any implementor of [`SessionStore`] may be
//! used. For instance, we could use the [`RedisStore`] instead of Moka.
//!
//! A cache is most helpful with read-heavy workloads, where the cache hit rate
//! will be high. This is because write-heavy workloads will require a roundtrip
//! to the store and therefore benefit less from caching.
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
//! pointer to the session in the form of an ID. This ID is a [UUID
//! v4](https://docs.rs/uuid/latest/uuid/struct.Uuid.html#method.new_v4).
//!
//! ### Secure nature of cookies
//!
//! Session IDs are considered secure if your platform's
//! [`getrandom`](https://docs.rs/getrandom/latest/getrandom/) is
//! secure[^getrandom], and therefore are not signed or encrypted. Note that
//! this assumption is predicated on the secure nature of the UUID crate and its
//! ability to generate securely-random values. It's also important to note that
//! session cookies **must never** be sent over a public, insecure channel.
//! Doing so is **not** secure.
//!
//! An expiration time determines when the session will be considered expired.
//! This translates to the cookie's `max-age` attribute. By default,
//! [`CookieConfig`] will set this to `None`. When `None`, this means the cookie
//! will be treated as a ["session" cookie][session-cookie], not to be confused
//! with the session itself, which generally means that the cookie will expire
//! once the user closes their browser.
//!
//! ## Key-value API
//!
//! Sessions manage a `HashMap<String, serde_json::Value>` but importantly are
//! transparently persisted to an arbitrary storage backend. Effectively,
//! `HashMap` is an intermediary, in-memory representation. By using a map-like
//! structure, we're able to present a familiar key-value interface for managing
//! sessions. This also allows us to store and retrieve native Rust
//! types, so long as our type is `impl Serialize` and can
//! be represented as JSON.[^json]
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
//! Sessions are directly serialized to arbitrary storage backends.
//! Implementations of `SessionStore` take a session and persist it such that it
//! can later be loaded via the session ID.
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
//! of hydrating a session from the store.
//!
//! This works by first looking for a cookie that matches our configured session
//! cookie name. If no such cookie is found or a cookie is found but the store
//! has no such session or the session is no longer active, we create a new
//! session.
//!
//! It's important to note that creating a session **does not** save the session
//! to the store. In fact, the store is not used at all unless one of two
//! conditions is true:
//!
//! 1. A session cookie was found and we attempt to load it from the store via
//!    the [`load`](SessionStore::load) method or,
//! 2. A session was marked as modified or deleted.
//!
//! In other words, creating a new session is a lightweight process that does
//! not incur the overhead of talking to a store. It's also important to create
//! a session proactively as the middleware will attach the session to the
//! request as a request extension. This allows handlers to extract the cookie
//! from the request and manipulate it.
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
//! [^getrandom]: `uuid` uses `getrandom` which varies by platform; the crucial
//!   assumption `tower-sessions` makes is that your platform is secure.
//! However, you **must** verify this for yourself.
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
//!
//! [session-cookie]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Cookies#define_the_lifetime_of_a_cookie
#![warn(
    clippy::all,
    nonstandard_style,
    future_incompatible,
    missing_debug_implementations
)]
#![deny(missing_docs)]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "redis-store")]
pub use fred;
#[cfg(feature = "mongodb-store")]
pub use mongodb;
#[cfg(feature = "sqlx-store")]
pub use sqlx;
/// Ensure the underlying cookie interface is available.
pub use tower_cookies::cookie;

#[cfg(feature = "memory-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "memory-store")))]
pub use self::memory_store::MemoryStore;
#[cfg(feature = "moka-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "moka-store")))]
pub use self::moka_store::MokaStore;
#[cfg(feature = "mongodb-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "mongodb-store")))]
pub use self::mongodb_store::MongoDBStore;
#[cfg(feature = "redis-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "redis-store")))]
pub use self::redis_store::RedisStore;
#[cfg(feature = "mysql-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "mysql-store")))]
pub use self::sqlx_store::MySqlStore;
#[cfg(feature = "postgres-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "postgres-store")))]
pub use self::sqlx_store::PostgresStore;
#[cfg(feature = "sqlite-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "sqlite-store")))]
pub use self::sqlx_store::SqliteStore;
#[cfg(feature = "sqlx-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "sqlx-store")))]
pub use self::sqlx_store::SqlxStoreError;
#[doc(inline)]
pub use self::{
    cookie_config::CookieConfig,
    service::{SessionManager, SessionManagerLayer},
    session::{Session, SessionExpiry},
    session_store::{CachingSessionStore, ExpiredDeletion, SessionStore},
};

#[cfg(feature = "axum-core")]
#[cfg_attr(docsrs, doc(cfg(feature = "axum-core")))]
mod extract;

#[cfg(feature = "memory-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "memory-store")))]
mod memory_store;

#[cfg(feature = "moka-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "moka-store")))]
mod moka_store;

#[cfg(feature = "redis-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "redis-store")))]
mod redis_store;

#[cfg(feature = "mongodb-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "mongodb-store")))]
mod mongodb_store;

#[cfg(feature = "sqlx-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "sqlx-store")))]
mod sqlx_store;

#[cfg(feature = "diesel-store")]
#[cfg_attr(docsrs, doc(cfg(feature = "diesel-store")))]
pub mod diesel_store;

pub mod cookie_config;
pub mod service;
pub mod session;
pub mod session_store;
