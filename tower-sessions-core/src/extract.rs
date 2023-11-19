use async_trait::async_trait;
use axum_core::extract::FromRequestParts;
use http::{request::Parts, StatusCode};

use crate::{session::Session, SessionStore};

#[async_trait]
impl<S, Store: SessionStore> FromRequestParts<S> for Session<Store>
where
    S: Sync + Send,
{
    type Rejection = (http::StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<Session<_>>().cloned().ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Can't extract session. Is `SessionManagerLayer` enabled?",
        ))
    }
}
