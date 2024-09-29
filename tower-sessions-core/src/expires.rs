use serde::{Deserialize, Serialize};

pub trait Expires {
    fn expires(&self) -> Expiry {
        Expiry::OnSessionEnd
    }
    #[allow(unused_variables)]
    fn set_expiry(&mut self, expiry: Expiry) {}
}

/// Session expiry configuration.
///
/// # Examples
///
/// ```rust
/// use time::{Duration, OffsetDateTime};
/// use tower_sessions_core::Expiry;
///
/// // Will be expired on "session end".
/// let expiry = Expiry::OnSessionEnd;
///
/// // Will be expired in five minutes from last acitve.
/// let expiry = Expiry::OnInactivity(Duration::minutes(5));
///
/// // Will be expired at the given timestamp.
/// let expired_at = OffsetDateTime::now_utc().saturating_add(Duration::weeks(2));
/// let expiry = Expiry::AtDateTime(expired_at);
/// ```
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Expiry {
    /// Expire on [current session end][current-session-end], as defined by the
    /// browser.
    ///
    /// [current-session-end]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Cookies#removal_defining_the_lifetime_of_a_cookie
    OnSessionEnd,

    /// Expire on inactivity.
    ///
    /// Reading a session is not considered activity for expiration purposes.
    /// [`Session`] expiration is computed from the last time the session was
    /// _modified_.
    OnInactivity(time::Duration),

    /// Expire at a specific date and time.
    ///
    /// This value may be extended manually with
    /// [`set_expiry`](Session::set_expiry).
    AtDateTime(time::OffsetDateTime),
}
