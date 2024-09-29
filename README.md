<h1 align="center">
    tower-sessions
</h1>

<p align="center">
    🥠 Sessions as a `tower` and `axum` middleware.
</p>

<div align="center">
    <a href="https://crates.io/crates/tower-sessions">
        <img src="https://img.shields.io/crates/v/tower-sessions.svg" />
    </a>
    <a href="https://docs.rs/tower-sessions">
        <img src="https://docs.rs/tower-sessions/badge.svg" />
    </a>
    <a href="https://github.com/maxcountryman/tower-sessions/actions/workflows/rust.yml">
        <img src="https://github.com/maxcountryman/tower-sessions/actions/workflows/rust.yml/badge.svg" />
    </a>
    <a href="https://codecov.io/gh/maxcountryman/tower-sessions" > 
        <img src="https://codecov.io/gh/maxcountryman/tower-sessions/graph/badge.svg?token=74POF0TJDN"/> 
    </a>
</div>

## TODOs
- Only update the cookie if the session was extracted.
- [X] Complete middleware implementation.
- [ ] Add tracing.
- [ ] Add signed cookies functionality.
- [ ] Document what session-store should implement, and what the `Record` type should implement.
- [ ] Add examples everywhere.
- [X] Rewrite the in memory store.
- [ ] Rewrite all the tests.

## 🎨 Overview

This crate provides sessions, key-value pairs associated with a site
visitor, as a `tower` middleware.

It offers:

- **Pluggable Storage Backends:** Bring your own backend simply by
  implementing the `SessionStore` trait, fully decoupling sessions from their
  storage.
- **Minimal Overhead**: Sessions are only loaded from their backing stores
  when they're actually used and only in e.g. the handler they're used in.
  That means this middleware can be installed anywhere in your route
  graph with minimal overhead.
- **An `axum` Extractor for `Session`:** Applications built with `axum`
  can use `Session` as an extractor directly in their handlers. This makes
  using sessions as easy as including `Session` in your handler.
- **Simple Key-Value Interface:** Sessions offer a key-value interface that
  supports native Rust types. So long as these types are `Serialize` and can
  be converted to JSON, it's straightforward to insert, get, and remove any
  value.
- **Strongly-Typed Sessions:** Strong typing guarantees are easy to layer on
  top of this foundational key-value interface.

This crate's session implementation is inspired by the [Django sessions middleware](https://docs.djangoproject.com/en/4.2/topics/http/sessions) and it provides a transliteration of those semantics.

### Session stores

Session data persistence is managed by user-provided types that implement
`SessionStore`. What this means is that applications can and should
implement session stores to fit their specific needs.

That said, a number of session store implmentations already exist and may be
useful starting points.

| Crate                                                                                                            | Persistent | Description                                                 |
| ---------------------------------------------------------------------------------------------------------------- | ---------- | ----------------------------------------------------------- |
| [`tower-sessions-dynamodb-store`](https://github.com/necrobious/tower-sessions-dynamodb-store)                   | Yes        | DynamoDB session store                                      |
| [`tower-sessions-firestore-store`](https://github.com/AtTheTavern/tower-sessions-firestore-store)                | Yes        | Firestore session store                                     |
| [`tower-sessions-libsql-store`](https://github.com/daybowbow-dev/tower-sessions-libsql-store)                    | Yes        | libSQL session store                                        |
| [`tower-sessions-mongodb-store`](https://github.com/maxcountryman/tower-sessions-stores/tree/main/mongodb-store) | Yes        | MongoDB session store                                       |
| [`tower-sessions-moka-store`](https://github.com/maxcountryman/tower-sessions-stores/tree/main/moka-store)       | No         | Moka session store                                          |
| [`tower-sessions-redis-store`](https://github.com/maxcountryman/tower-sessions-stores/tree/main/redis-store)     | Yes        | Redis via `fred` session store                              |
| [`tower-sessions-rorm-store`](https://github.com/rorm-orm/tower-sessions-rorm-store)                             | Yes        | SQLite, Postgres and Mysql session store provided by `rorm` |
| [`tower-sessions-rusqlite-store`](https://github.com/patte/tower-sessions-rusqlite-store)                        | Yes        | Rusqlite session store                                      |
| [`tower-sessions-sled-store`](https://github.com/Zatzou/tower-sessions-sled-store)                               | Yes        | Sled session store                                          |
| [`tower-sessions-sqlx-store`](https://github.com/maxcountryman/tower-sessions-stores/tree/main/sqlx-store)       | Yes        | SQLite, Postgres, and MySQL session stores                  |
| [`tower-sessions-surrealdb-store`](https://github.com/rynoV/tower-sessions-surrealdb-store)                      | Yes        | SurrealDB session store                                     |

Have a store to add? Please open a PR adding it.

### User session management

To facilitate authentication and authorization, we've built [`axum-login`](https://github.com/maxcountryman/axum-login) on top of this crate. Please check it out if you're looking for a generalized auth solution.

## 📦 Install

To use the crate in your project, add the following to your `Cargo.toml` file:

```toml
[dependencies]
tower-sessions = "0.13.0"
```

## 🤸 Usage

### `axum` Example

```rust
use std::net::SocketAddr;

use axum::{response::IntoResponse, routing::get, Router};
use serde::{Deserialize, Serialize};
use time::Duration;
use tower_sessions::{Expiry, MemoryStore, Session, SessionManagerLayer};

const COUNTER_KEY: &str = "counter";

#[derive(Default, Deserialize, Serialize)]
struct Counter(usize);

async fn handler(session: Session) -> impl IntoResponse {
    let counter: Counter = session.get(COUNTER_KEY).await.unwrap().unwrap_or_default();
    session.insert(COUNTER_KEY, counter.0 + 1).await.unwrap();
    format!("Current count: {}", counter.0)
}

#[tokio::main]
async fn main() {
    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(false)
        .with_expiry(Expiry::OnInactivity(Duration::seconds(10)));

    let app = Router::new().route("/", get(handler)).layer(session_layer);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}
```

You can find this [example][counter-example] as well as other example projects in the [example directory][examples].

> [!NOTE]
> See the [crate documentation][docs] for more usage information.

## 🛟 Getting Help

We've put together a number of [examples][examples] to help get you started. You're also welcome to [open a discussion](https://github.com/maxcountryman/tower-sessions/discussions/new?category=q-a) and ask additional questions you might have.

## 👯 Contributing

We appreciate all kinds of contributions, thank you!

[counter-example]: https://github.com/maxcountryman/tower-sessions/tree/main/examples/counter.rs
[examples]: https://github.com/maxcountryman/tower-sessions/tree/main/examples
[docs]: https://docs.rs/tower-sessions
