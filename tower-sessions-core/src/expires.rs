use serde::{Deserialize, Serialize};

/// Trait for types that can expire.
///
/// If a [`SessionStore`][crate::SessionStore] does session expiration management,
/// it should rely on this trait to access a record's expiration.
///
/// If a [`SessionStore`][crate::SessionStore] implementation relies on this trait, then it should
/// also check the expiration of a record every time it is saved, and it should update the
/// record's expiration on the backend accordingly.
///
/// # Examples
/// - A record that should not expire:
/// ```
/// use tower_sessions_core::{Expires, Expiry};
///
/// struct NeverExpires;
///
/// impl Expires for NeverExpires {}
/// ```
///
/// - A record that should expire after 5 minutes of inactivity:
/// ```
/// use time::{Duration, OffsetDateTime};
/// use tower_sessions_core::{Expires, Expiry};
///
/// struct ExpiresAfter5Minutes;
///
/// impl Expires for ExpiresAfter5Minutes {
///     fn expires(&self) -> Expiry {
///         Expiry::OnInactivity(Duration::minutes(5))
///         // OR
///         // Expiry::OnInactivity(OffsetDateTime::now_utc() + Duration::minutes(5));
///     }
/// }
/// ```
///
/// - A record that keeps track of its own expiration:
/// ```
/// use time::{Duration, OffsetDateTime};
/// use tower_sessions_core::{Expires, Expiry};
///
/// struct CustomExpiry {
///     expiry: Expiry,
/// }
///
/// impl Expires for CustomExpiry {
///     fn expires(&self) -> Expiry {
///         self.expiry
///     }
/// }
/// ```
pub trait Expires {
    /// Returns the expiration of the record.
    ///
    /// By default, this always returns [`Expiry::OnSessionEnd`]. If the record should expire, then
    /// one needs to implement this method.
    fn expires(&self) -> Expiry {
        Expiry::OnSessionEnd
    }
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
#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Expiry {
    /// __Browser:__ Expire on [current session end][current-session-end], as defined by the
    /// browser.
    ///
    /// __Server:__ No expiration is set.
    ///
    /// [current-session-end]: https://developer.mozilla.org/en-US/docs/Web/HTTP/Cookies#removal_defining_the_lifetime_of_a_cookie
    OnSessionEnd,

    /// Expire on inactivity.
    ///
    /// Reading a session is not considered activity for expiration purposes. Expiration
    /// is computed from the last time the session was modified. That is, when
    /// the session is created ([`SessionStore::create`]), when it is saved
    /// ([`SessionStore::save`]/[`SessionStore::save_or_create`]), and when its [`Id`] is cycled
    /// ([`SessionStore::cycle_id`]).
    ///
    /// [`Id`]: crate::Id
    /// [`SessionStore::create`]: crate::SessionStore::create
    /// [`SessionStore::save`]: crate::SessionStore::save
    /// [`SessionStore::save_or_create`]: crate::SessionStore::save_or_create
    /// [`SessionStore::cycle_id`]: crate::SessionStore::cycle_id
    OnInactivity(time::Duration),

    /// Expire at a specific date and time.
    AtDateTime(time::OffsetDateTime),
}
