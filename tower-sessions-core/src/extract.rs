use std::convert::Infallible;

use axum_core::extract::{FromRef, FromRequestParts};
use http::request::Parts;

use crate::{session::LazySession, SessionStore};

#[async_trait::async_trait]
impl<State, Record, Store> FromRequestParts<State> for LazySession<Record, Store>
where
    State: Send + Sync,
    Record: Send + Sync,
    Store: SessionStore<Record> + FromRef<State>,
{
    // TODO: use the never type `!` when it becomes stable
    type Rejection = Infallible;

    async fn from_request_parts(_parts: &mut Parts, state: &State) -> Result<Self, Self::Rejection> {
        let store = Store::from_ref(state);
        // TODO: extract the session from the request? or should a middleware do this? Because in
        // the end we also need to set the session cookie in the response, which is not possible
        // with an extractor.
        
        Ok(LazySession::new(store, None))
    }
}
