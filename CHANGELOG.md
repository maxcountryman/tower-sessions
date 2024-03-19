# Unreleased

# 0.12.0

**Important Security Update**

- Id collision mitigation. #181

This release introduces a new method, `create`, to the `SessionStore` trait to distinguish between creating a new session and updating an existing one. **This distinction is crucial for mitigating the potential for session ID collisions.**

Although the probability of session ID collisions is statistically low, given that IDs are composed of securely-random `i128` values, such collisions pose a significant security risk. A store that does not differentiate between session creation and updates could inadvertently allow an existing session to be accessed, leading to potential session takeovers.

Session store authors are strongly encouraged to update and implement `create` such that potential ID collisions are handled, either by generating a new ID or returning an error.

As a transitional measure, we have provided a default implementation of `create` that wraps the existing `save` method. However, this default is not immune to the original issue. Therefore, it is imperative that stores override the `create` method with an implementation that adheres to the required uniqueness semantics, thereby effectively mitigating the risk of session ID collisions.

# 0.11.1

- Ensure `session.set_expiry` updates record. #175
- Provide `signed` and `private` features, enabling signing and encryption respectively. #157

# 0.11.0

- Uses slices when encoding and decoding `Id`. #159

**Breaking Changes**

- Removes `IdError` type in favor of using `base64::DecodeSliceError`. #159
- Provides the same changes as 0.10.4, without breaking SemVer.
- Updates `base64` to `0.22.0`.

# ~0.10.4~ **Yanked:** SemVer breaking

- Revert introduction of lifetime parameter; use static lifetime directly

This ensures that the changes introduced in `0.10.3` do not break SemVer.

Please note that `0.10.3` has been yanked in accordance with cargo guidelines.

# ~0.10.3~ **Yanked:** SemVer breaking

- Improve session config allocation footprint #158

# 0.10.2

- Ensure "Path" and "Domain" are set on removal cookie #154

# 0.10.1

- Ensure `Expires: Session` #149

# 0.10.0

**Breaking Changes**

- Improve session ID #141
- Relocate previously bundled stores #145
- Move service out of core #146

Session IDs are now represetned as base64-encoded `i128`s, boast 128 bits of entropy, and are shorter, saving network bandwidth and improving the secure nature of sessions.

We no longer bundle session stores via feature flags and as such applications must be updated to require the stores directly. For example, applications that use the `tower-sessions-sqlx-store` should update their `Cargo.toml` like so:

```toml
tower-sessions = "0.10.0"
tower-sessions-sqlx-store = { version = "0.10.0", features = ["sqlite"] }
```

Assuming a SQLite store, as an example.

Furthermore, imports will also need to be updated accordingly. For example:

```rust
use std::net::SocketAddr;

use axum::{response::IntoResponse, routing::get, Router};
use serde::{Deserialize, Serialize};
use time::Duration;
use tower_sessions::{session_store::ExpiredDeletion, Expiry, Session, SessionManagerLayer};
use tower_sessions_sqlx_store::{sqlx::SqlitePool, SqliteStore};

const COUNTER_KEY: &str = "counter";

#[derive(Serialize, Deserialize, Default)]
struct Counter(usize);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = SqlitePool::connect("sqlite::memory:").await?;
    let session_store = SqliteStore::new(pool);
    session_store.migrate().await?;

    let deletion_task = tokio::task::spawn(
        session_store
            .clone()
            .continuously_delete_expired(tokio::time::Duration::from_secs(60)),
    );

    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(false)
        .with_expiry(Expiry::OnInactivity(Duration::seconds(10)));

    let app = Router::new().route("/", get(handler)).layer(session_layer);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app.into_make_service()).await?;

    deletion_task.await??;

    Ok(())
}

async fn handler(session: Session) -> impl IntoResponse {
    let counter: Counter = session.get(COUNTER_KEY).await.unwrap().unwrap_or_default();
    session.insert(COUNTER_KEY, counter.0 + 1).await.unwrap();
    format!("Current count: {}", counter.0)
}
```

Finally, the service itself has been moved out of the core crate, which makes this crate smaller as well as establishes better boundaries between code.

Thank you for bearing with us: we are approaching longer term stability and aim to minimize churn going forward as we begin to move toward a 1.0 release.

# 0.9.1

- Ensure `clear` works before record loading. #134

# 0.9.0

**Breakiung Changes**

- Make service infallible. #132

This updates the service such that it always returns a response directly. In practice this means that e.g. `axum` applications no longer need the `HandleErrorLayer` and instead can use the layer directly. Note that if you use other fallible `tower` middleware, you will still need to use `HandleErrorLayer`.

As such we've also remove the `MissingCookies` and `MissingId` variants from the session error enum.

# 0.8.2

- Derive `PartialEq` for `Record`. #125

# 0.8.1

- Allow constructing `RedisStore` from `RedisPool`. #122

# 0.8.0

**Breaking Changes**

- Lazy sessions. #112

Among other things, session methods are now entirely async, meaning applications must be updated to await these methods in order to migrate.

Separately, `SessionStore` has been updated to use a `Record` intermediary. As such, `SessionStore` implementations must be updated accordingly.

Session stores now use a concrete error type that must be used in implementations of `SessionStore`.

The `secure` cookie attribute now defaults to `true`.

# 0.7.0

**Breaking Changes**

- Bump `axum-core` to 0.4.0, `http` to 1.0, `tower-cookies` to 0.10.0. #107

This brings `tower-cookies` up-to-date which includes an update to the `cookies` crate.

# 0.6.0

**Breaking Changes**

- Remove concurrent shared memory access support; this may also address some performance degradations. #91
- Related to shared memory support, we also remove `replace_if_equal`, as it is no longer relevant. #91

**Other Changes**

- Allow setting up table and schema name for Postgres. #93

# 0.5.1

- Only delete from session store if we have a session cookie. #90

# 0.5.0

**Breaking Changes**

- Use a default session name of "id" to avoid fingerprinting, as per https://cheatsheetseries.owasp.org/cheatsheets/Session_Management_Cheat_Sheet.html#session-id-name-fingerprinting.

Note that applications using the old default, "tower.sid", may continue to do so without disruption by specifying [`with_name("tower.sid")`](https://docs.rs/tower-sessions/latest/tower_sessions/service/struct.SessionManagerLayer.html#method.with_name).

# 0.4.3

## **Important Security Fix**

If your application uses `MokaStore` or `MemoryStore`, please update immediately to ensure proper server-side handling of expired sessions.

**Other Changes**

- Make `HttpOnly` configurable. #81

# 0.4.2

- Provide tracing instrumentation.
- Ensure non-negative max-age. #79

# 0.4.1

- Fix lifecycle state persisting in stores when it should not. #71

# 0.4.0

**Breaking Changes**

- Sessions are serialized and deserialized from stores directly and `SessionRecord` is removed.
- Expiration time has been replaced with an expiry type.
- Drop session-prefix from session types.
- The session `modified` methid is renamed to `is_modified`.
- Session active semantic is now defined by stores and the `active` method removed.
- Service now contains session configuration and `CookieConfig` is removed.
- Deletion task is now provided via the `deletion-task` feature flag.

# 0.3.3

- Ensure loaded sessions are removed whenever they can be; do not couple removal with session saving.

# 0.3.2

- Implement reference-counted garbage collection for loaded sessions. #52
- Make `SessionId`'s UUID public. #53

# 0.3.1

- Use `DashMap` entry API to address data race introduced by dashmap. #41

# 0.3.0

**Breaking Changes**

- `tokio` feature flag is now `tokio-rt`.
- Session IDs are returned as references now.

**Other Changes**

- Update `fred` to 7.0.0.
- Track loaded sessions to enable concurrent access. #37

# 0.2.4

- Fix session saving and loading potential data race. #36

# 0.2.3

- Fix setting of modified in `replace_if_equal`.

# 0.2.2

- Lift `Debug` constraint on `CachingSessionStore`.
- Run caching store save and load ops concurrently. #25

# 0.2.1

- Fix clearing session's data is not persisted. #22

# 0.2.0

**Breaking Changes**

- Renamed store error variants for consistency (SqlxStoreError, RedisStoreError). #18
- Moved MySQL `expiration_time` column to `timestamp(6), for microsecond resolution. #14
- Replaced `Session.with_max_age` with `set_expiration_time` and `set_expiration_time_from_max_age`, allowing applications to control session durations dynamically. #7

**Other Changes**

- Provide layered caching via `CachingSessionStore` #8
- Provide a Moka store #6 (Thank you @and-reas-se!)
- Provide a MongoDB store #5 (Thank you @JustMangoT!)

# 0.1.0

- Initial release :tada:
