use std::{fmt::Display, net::SocketAddr};

use async_trait::async_trait;
use axum::{
    error_handling::HandleErrorLayer, extract::FromRequestParts, response::IntoResponse,
    routing::get, BoxError, Router,
};
use http::{request::Parts, StatusCode};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};
use tower::ServiceBuilder;
use tower_sessions::{Expiry, MemoryStore, Session, SessionManagerLayer};
use uuid::Uuid;

#[derive(Clone, Deserialize, Serialize)]
struct GuestData {
    id: Uuid,
    pageviews: usize,
    first_seen: OffsetDateTime,
    last_seen: OffsetDateTime,
}

impl Default for GuestData {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
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

    fn id(&self) -> Uuid {
        self.guest_data.id
    }

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

impl Display for Guest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let now = OffsetDateTime::now_utc();
        write!(
            f,
            "Guest ID {}\n\nPageviews {}\n\nFirst seen {} ago\n\nLast seen {} ago\n\n",
            self.id().as_hyphenated(),
            self.pageviews(),
            now - self.first_seen(),
            now - self.last_seen()
        )
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
                .with_expiry(Expiry::OnInactivity(Duration::seconds(10))),
        );

    let app = Router::new()
        .route("/", get(handler))
        .layer(session_service);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
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
