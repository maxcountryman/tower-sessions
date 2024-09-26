//! Module for session IDs.

use std::fmt::{self, Display};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};


/// ID type for sessions.
///
/// Wraps an array of 16 bytes.
///
/// # Examples
///
/// ```rust
/// use tower_sessions::session::Id;
///
/// Id::default();
/// ```
#[cfg(feature = "id-access")]
#[derive(Copy, Clone, Debug, Deserialize, Serialize, Eq, Hash, PartialEq)]
pub struct Id(pub i128);

/// ID type for sessions.
///
/// Wraps an array of 16 bytes.
///
/// # Examples
///
/// ```rust
/// use tower_sessions::session::Id;
///
/// Id::default();
/// ```
#[derive(Copy, Clone, Debug, Deserialize, Serialize, Eq, Hash, PartialEq)]
#[cfg(not(feature = "id-access"))]
pub struct Id(i128);

impl Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut encoded = [0; 22];
        URL_SAFE_NO_PAD
            .encode_slice(self.0.to_le_bytes(), &mut encoded)
            .expect("Encoded ID must be exactly 22 bytes");
        let encoded = std::str::from_utf8(&encoded).expect("Encoded ID must be valid UTF-8");

        f.write_str(encoded)
    }
}
