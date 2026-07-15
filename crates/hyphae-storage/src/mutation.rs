// SPDX-License-Identifier: Apache-2.0

use thiserror::Error;

use crate::log::MAX_OPERATION_BYTES;

const PUT: u8 = 1;
const DELETE: u8 = 2;
const PUT_HEADER_LENGTH: usize = 13;
const DELETE_HEADER_LENGTH: usize = 5;

/// Maximum encoded key length accepted by the embedded KV layer.
pub const MAX_KEY_BYTES: usize = 1024 * 1024;

/// A deterministic mutation persisted inside one transaction operation frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Mutation {
    /// Replaces the value stored under a nonempty binary key.
    Put {
        /// Binary key.
        key: Vec<u8>,
        /// Binary value.
        value: Vec<u8>,
    },
    /// Removes a binary key. Deleting a missing key is idempotent.
    Delete {
        /// Binary key.
        key: Vec<u8>,
    },
}

impl Mutation {
    /// Creates a put mutation.
    pub fn put(key: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) -> Self {
        Self::Put {
            key: key.into(),
            value: value.into(),
        }
    }

    /// Creates a delete mutation.
    pub fn delete(key: impl Into<Vec<u8>>) -> Self {
        Self::Delete { key: key.into() }
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, MutationError> {
        match self {
            Self::Put { key, value } => encode_put(key, value),
            Self::Delete { key } => encode_delete(key),
        }
    }

    pub(crate) fn decode(encoded: &[u8]) -> Result<Self, MutationError> {
        let Some(kind) = encoded.first().copied() else {
            return Err(MutationError::Malformed);
        };
        match kind {
            PUT => decode_put(encoded),
            DELETE => decode_delete(encoded),
            kind => Err(MutationError::UnknownKind { kind }),
        }
    }
}

/// Failure while validating or decoding a persisted mutation.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum MutationError {
    /// Empty keys are forbidden.
    #[error("mutation key must not be empty")]
    EmptyKey,

    /// A key exceeds the stable storage limit.
    #[error("mutation key is {length} bytes; maximum is {maximum}")]
    KeyTooLarge {
        /// Actual key length.
        length: usize,
        /// Maximum key length.
        maximum: usize,
    },

    /// The full encoded operation exceeds one log frame.
    #[error("encoded mutation is {length} bytes; maximum is {maximum}")]
    OperationTooLarge {
        /// Actual encoded length.
        length: usize,
        /// Maximum operation length.
        maximum: usize,
    },

    /// Persisted bytes do not form one canonical mutation.
    #[error("malformed persisted mutation")]
    Malformed,

    /// The mutation kind is not defined by this disk format.
    #[error("unknown persisted mutation kind {kind}")]
    UnknownKind {
        /// Raw kind byte.
        kind: u8,
    },
}

fn encode_put(key: &[u8], value: &[u8]) -> Result<Vec<u8>, MutationError> {
    validate_key(key)?;
    let key_length = u32::try_from(key.len()).map_err(|_| MutationError::KeyTooLarge {
        length: key.len(),
        maximum: MAX_KEY_BYTES,
    })?;
    let value_length =
        u64::try_from(value.len()).map_err(|_| MutationError::OperationTooLarge {
            length: usize::MAX,
            maximum: MAX_OPERATION_BYTES,
        })?;
    let encoded_length = PUT_HEADER_LENGTH
        .checked_add(key.len())
        .and_then(|length| length.checked_add(value.len()))
        .ok_or(MutationError::OperationTooLarge {
            length: usize::MAX,
            maximum: MAX_OPERATION_BYTES,
        })?;
    validate_operation_length(encoded_length)?;

    let mut encoded = Vec::with_capacity(encoded_length);
    encoded.push(PUT);
    encoded.extend_from_slice(&key_length.to_le_bytes());
    encoded.extend_from_slice(&value_length.to_le_bytes());
    encoded.extend_from_slice(key);
    encoded.extend_from_slice(value);
    Ok(encoded)
}

fn encode_delete(key: &[u8]) -> Result<Vec<u8>, MutationError> {
    validate_key(key)?;
    let key_length = u32::try_from(key.len()).map_err(|_| MutationError::KeyTooLarge {
        length: key.len(),
        maximum: MAX_KEY_BYTES,
    })?;
    let encoded_length =
        DELETE_HEADER_LENGTH
            .checked_add(key.len())
            .ok_or(MutationError::OperationTooLarge {
                length: usize::MAX,
                maximum: MAX_OPERATION_BYTES,
            })?;
    validate_operation_length(encoded_length)?;

    let mut encoded = Vec::with_capacity(encoded_length);
    encoded.push(DELETE);
    encoded.extend_from_slice(&key_length.to_le_bytes());
    encoded.extend_from_slice(key);
    Ok(encoded)
}

fn decode_put(encoded: &[u8]) -> Result<Mutation, MutationError> {
    if encoded.len() < PUT_HEADER_LENGTH {
        return Err(MutationError::Malformed);
    }
    let key_length = usize::try_from(u32::from_le_bytes(copy_array(&encoded[1..5])))
        .map_err(|_| MutationError::Malformed)?;
    let value_length = usize::try_from(u64::from_le_bytes(copy_array(&encoded[5..13])))
        .map_err(|_| MutationError::Malformed)?;
    let key_end = PUT_HEADER_LENGTH
        .checked_add(key_length)
        .ok_or(MutationError::Malformed)?;
    let value_end = key_end
        .checked_add(value_length)
        .ok_or(MutationError::Malformed)?;
    if value_end != encoded.len() {
        return Err(MutationError::Malformed);
    }
    let key = encoded[PUT_HEADER_LENGTH..key_end].to_vec();
    validate_key(&key)?;
    validate_operation_length(encoded.len())?;
    Ok(Mutation::Put {
        key,
        value: encoded[key_end..value_end].to_vec(),
    })
}

fn decode_delete(encoded: &[u8]) -> Result<Mutation, MutationError> {
    if encoded.len() < DELETE_HEADER_LENGTH {
        return Err(MutationError::Malformed);
    }
    let key_length = usize::try_from(u32::from_le_bytes(copy_array(&encoded[1..5])))
        .map_err(|_| MutationError::Malformed)?;
    let key_end = DELETE_HEADER_LENGTH
        .checked_add(key_length)
        .ok_or(MutationError::Malformed)?;
    if key_end != encoded.len() {
        return Err(MutationError::Malformed);
    }
    let key = encoded[DELETE_HEADER_LENGTH..key_end].to_vec();
    validate_key(&key)?;
    validate_operation_length(encoded.len())?;
    Ok(Mutation::Delete { key })
}

pub(crate) fn validate_key(key: &[u8]) -> Result<(), MutationError> {
    if key.is_empty() {
        return Err(MutationError::EmptyKey);
    }
    if key.len() > MAX_KEY_BYTES {
        return Err(MutationError::KeyTooLarge {
            length: key.len(),
            maximum: MAX_KEY_BYTES,
        });
    }
    Ok(())
}

fn validate_operation_length(length: usize) -> Result<(), MutationError> {
    if length > MAX_OPERATION_BYTES {
        return Err(MutationError::OperationTooLarge {
            length,
            maximum: MAX_OPERATION_BYTES,
        });
    }
    Ok(())
}

fn copy_array<const N: usize>(source: &[u8]) -> [u8; N] {
    let mut output = [0_u8; N];
    output.copy_from_slice(source);
    output
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::{MAX_KEY_BYTES, Mutation, MutationError};

    #[test]
    fn mutation_codec_round_trips_binary_values() -> Result<(), Box<dyn Error>> {
        let mutations = [
            Mutation::put([0, 1, 2], [255, 0, 3]),
            Mutation::delete([7, 8, 9]),
        ];
        for mutation in mutations {
            assert_eq!(Mutation::decode(&mutation.encode()?)?, mutation);
        }
        Ok(())
    }

    #[test]
    fn mutation_codec_rejects_noncanonical_lengths() -> Result<(), Box<dyn Error>> {
        let mut encoded = Mutation::put(b"key", b"value").encode()?;
        encoded.push(0);
        assert_eq!(Mutation::decode(&encoded), Err(MutationError::Malformed));
        Ok(())
    }

    #[test]
    fn keys_are_bounded_before_encoding() {
        let result = Mutation::delete(vec![0; MAX_KEY_BYTES + 1]).encode();
        assert!(matches!(result, Err(MutationError::KeyTooLarge { .. })));
    }
}
