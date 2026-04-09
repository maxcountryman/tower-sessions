use axum_core::extract::FromRequestParts;
use http::{request::Parts, StatusCode};

use crate::session::Session;

impl<Id, S> FromRequestParts<S> for Session<Id>
where
    Id: Clone + Sync + Send + 'static,
    S: Sync + Send,
{
    type Rejection = (http::StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<Session<Id>>().cloned().ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Can't extract session. Is `SessionManagerLayer` enabled?",
        ))
    }
}
