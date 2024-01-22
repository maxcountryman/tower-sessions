use std::{fmt, net::SocketAddr};

use async_trait::async_trait;
use axum::{extract::FromRequestParts, response::IntoResponse, routing::get, Router};
use http::{request::Parts, StatusCode};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tower_sessions::{MemoryStore, Session, SessionManagerLayer};

#[derive(Clone, Deserialize, Serialize)]
struct GuestData {
    pageviews: usize,
    first_seen: OffsetDateTime,
    last_seen: OffsetDateTime,
}

impl Default for GuestData {
    fn default() -> Self {
        Self {
            pageviews: 0,
            first_seen: OffsetDateTime::now_utc(),
            last_seen: OffsetDateTime::now_utc(),
        }
    }
}

struct Guest {
    session: Session,
    guest_data: GuestData,
}

impl Guest {
    const GUEST_DATA_KEY: &'static str = "guest.data";

    fn first_seen(&self) -> OffsetDateTime {
        self.guest_data.first_seen
    }

    fn last_seen(&self) -> OffsetDateTime {
        self.guest_data.last_seen
    }

    fn pageviews(&self) -> usize {
        self.guest_data.pageviews
    }

    async fn mark_pageview(&mut self) {
        self.guest_data.pageviews += 1;
        Self::update_session(&self.session, &self.guest_data).await
    }

    async fn update_session(session: &Session, guest_data: &GuestData) {
        session
            .insert(Self::GUEST_DATA_KEY, guest_data.clone())
            .await
            .unwrap()
    }
}

impl fmt::Display for Guest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Guest")
            .field("pageviews", &self.pageviews())
            .field("first_seen", &self.first_seen())
            .field("last_seen", &self.last_seen())
            .finish()
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for Guest
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(req: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let session = Session::from_request_parts(req, state).await?;

        let mut guest_data: GuestData = session
            .get(Self::GUEST_DATA_KEY)
            .await
            .unwrap()
            .unwrap_or_default();

        guest_data.last_seen = OffsetDateTime::now_utc();

        Self::update_session(&session, &guest_data).await;

        Ok(Self {
            session,
            guest_data,
        })
    }
}

// This demonstrates a `Guest` extractor, but we could have any number of
// namespaced, strongly-typed "buckets" like `Guest` in the same session.
//
// Use cases could include buckets for site preferences, analytics,
// feature flags, etc.
async fn handler(mut guest: Guest) -> impl IntoResponse {
    guest.mark_pageview().await;
    format!("{}", guest)
}

#[tokio::main]
async fn main() {
    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store).with_secure(false);

    let app = Router::new().route("/", get(handler)).layer(session_layer);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}
