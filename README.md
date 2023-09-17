<h1 align="center">
tower-sessions
</h1>

<p align="center">
🥠 Cookie-based sessions as a `tower` middleware.
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
</div>

## 🎨 Overview

This crate provides cookie-based sessions as a `tower` middleware.

- Wraps `tower-cookies` for cookie management
- Decouples sessions from their storage (`SessionStore`)
- `Session` works as an extractor when using `axum`
- Redis and SQLx stores provided via feature flags
- Works directly with types that implement `Serialize` and `Deserialize`

## 📦 Install

To use the crate in your project, add the following to your `Cargo.toml` file:

```toml
[dependencies]
tower-sessions = "0.0.0"
```

## 🤸 Usage

### `axum` Example

```rust
use std::net::SocketAddr;

use axum::{
    error_handling::HandleErrorLayer, response::IntoResponse, routing::get, BoxError, Router,
};
use http::StatusCode;
use serde::{Deserialize, Serialize};
use tower::ServiceBuilder;
use tower_sessions::{time::Duration, MemoryStore, Session, SessionManagerLayer};

#[derive(Default, Deserialize, Serialize)]
struct Counter(usize);

#[tokio::main]
async fn main() {
    let session_store = MemoryStore::default();
    let session_service = ServiceBuilder::new()
        .layer(HandleErrorLayer::new(|_: BoxError| async {
            StatusCode::BAD_REQUEST
        }))
        .layer(
            SessionManagerLayer::new(session_store)
                .with_secure(false)
                .with_max_age(Duration::seconds(10)),
        );

    let app = Router::new()
        .route("/", get(handler))
        .layer(session_service);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn handler(session: Session) -> impl IntoResponse {
    let counter: Counter = session
        .get("counter")
        .expect("Could not deserialize.")
        .unwrap_or_default();

    session
        .insert("counter", counter.0 + 1)
        .expect("Could not serialize.");

    format!("Current count: {}", counter.0)
}
```

You can find this [example][counter-example] as well as other example projects in the [example directory][examples].

See the [crate documentation][docs] for more usage information.

[counter-example]: https://github.com/maxcountryman/tower-sessions/tree/main/examples/counter.rs
[examples]: https://github.com/maxcountryman/tower-sessions/tree/main/examples
[docs]: https://docs.rs/tower-sessions
