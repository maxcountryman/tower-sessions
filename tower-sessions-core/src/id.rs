//! Module for session IDs.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, DecodeError, Engine as _};
use serde::{Deserialize, Serialize};
use std::{
    fmt::{self, Display},
    str::FromStr,
};

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

#[cfg(feature = "id-access")]
impl Id {
    /// Create an ID from the default random source provided by the `rand` crate ([`rand::rngs::ThreadRng`]).
    #[cfg(feature = "random-id")]
    pub fn random() -> Self {
        Id(rand::random())
    }

    /// Create an ID from the provided random number generator.
    #[cfg(feature = "random-id")]
    pub fn random_with_rng<R: rand::Rng>(rng: &mut R) -> Self {
        Id(rng.gen())
    }
}

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

#[cfg(feature = "id-access")]
impl FromStr for Id {
    type Err = base64::DecodeSliceError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut decoded = [0; 16];
        let bytes_decoded = URL_SAFE_NO_PAD.decode_slice(s.as_bytes(), &mut decoded)?;
        if bytes_decoded != 16 {
            let err = DecodeError::InvalidLength(bytes_decoded);
            return Err(base64::DecodeSliceError::DecodeError(err));
        }

        Ok(Self(i128::from_le_bytes(decoded)))
    }
}
