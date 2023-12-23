# Unreleased

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
