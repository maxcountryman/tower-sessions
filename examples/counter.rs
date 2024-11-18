use std::net::SocketAddr;

use axum::{response::IntoResponse, routing::get, Router};
use time::Duration;
use tower_sesh::{Expires, Expiry, MemoryStore, Session, SessionManagerLayer};

#[derive(Clone, Copy, Debug)]
struct Counter(usize);

impl Expires for Counter {
    fn expires(&self) -> Expiry {
        Expiry::OnInactivity(Duration::seconds(10))
    }
}

async fn handler(session: Session<MemoryStore<Counter>>) -> impl IntoResponse {
    let value = if let Some(counter_state) = session.clone().load::<Counter>().await.unwrap() {
        // We loaded the session, let's update the counter.
        match counter_state
            .update(|counter| counter.0 += 1)
            .await
            .unwrap()
        {
            Some(new_state) => new_state.data().0,
            None => {
                // The session has expired while we were updating it, let's create a new one.
                session.create(Counter(0)).await.unwrap();
                0
            }
        }
    } else {
        // No session found, let's create a new one.
        session.create(Counter(0)).await.unwrap();
        0
    };

    format!("Current count: {}", value)
}

#[tokio::main]
async fn main() {
    let session_store: MemoryStore<Counter> = MemoryStore::default();
    let session_layer = SessionManagerLayer {
        store: session_store,
        config: Default::default(),
    };

    let app = Router::new().route("/", get(handler)).layer(session_layer);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}
