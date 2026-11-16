//! Identifiers. Turn and call IDs increase within a session and are never reused, so a
//! late message can always be recognised as stale by comparing IDs.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A conversation turn. `0` is never issued.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TurnId(pub u64);

/// One provider request, capture window, confirmation window or tool call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CallId(pub u64);

impl fmt::Display for TurnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for CallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Binds a confirmation to one proposal of one turn: 16 hex digits of FNV-1a 64 over the
/// turn and the proposal's fields. A binding identifier, not a cryptographic hash — the
/// proposal it names is held by the session, so the digest only has to tell proposals apart.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Digest(pub String);

impl Digest {
    /// Digest of a sequence of byte fields, each length-prefixed so field boundaries count.
    pub(crate) fn of_fields<'a>(fields: impl IntoIterator<Item = &'a [u8]>) -> Self {
        const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        let mut hash = OFFSET;
        let mut eat = |byte: u8| {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(PRIME);
        };
        for field in fields {
            for byte in (field.len() as u64).to_le_bytes() {
                eat(byte);
            }
            for &byte in field {
                eat(byte);
            }
        }
        Self(format!("{hash:016x}"))
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_stable_and_field_boundaries_count() {
        let a = Digest::of_fields([b"ab".as_slice(), b"c".as_slice()]);
        let b = Digest::of_fields([b"a".as_slice(), b"bc".as_slice()]);
        assert_ne!(a, b, "moving a byte across a field boundary must change the digest");
        assert_eq!(a, Digest::of_fields([b"ab".as_slice(), b"c".as_slice()]));
        assert_eq!(a.0.len(), 16);
    }
}
