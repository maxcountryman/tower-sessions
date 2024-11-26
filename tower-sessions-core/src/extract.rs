use axum_core::extract::FromRequestParts;
use http::{request::Parts, StatusCode};
use std::future::{ready, Future};

use crate::session::Session;

impl<S> FromRequestParts<S> for Session
where
    S: Sync + Send,
{
    type Rejection = (http::StatusCode, &'static str);

    fn from_request_parts(parts: &mut Parts, _state: &S) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        ready(parts.extensions.get::<Session>().cloned().ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Can't extract session. Is `SessionManagerLayer` enabled?",
        )))
    }
}
