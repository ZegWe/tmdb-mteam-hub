use std::error::Error;
use std::fmt;

use ring::digest::{Context, SHA256};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[cfg(test)]
use crate::subscription::repository::ListSubscriptionsCommand;
use crate::subscription::repository::{ListCursor, SubscriptionListFilter};
use crate::subscription::{
    SubscriptionAttentionTag, SubscriptionLifecycleState, SubscriptionMediaKind,
};

const CURSOR_V2_PREFIX: &str = "v2.";
const CURSOR_SCOPE_BYTES: usize = 32;
pub(super) const MAX_CURSOR_TOKEN_LEN: usize = 4096;
const SCOPE_DOMAIN: &[u8] = b"tmdb-mteam-hub/subscription-list-cursor-scope/v2\0";

/// Canonical account-and-filter scope for one subscription list traversal.
///
/// Limit and the incoming cursor are deliberately excluded so clients may
/// change page size while continuing the same filtered traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ListCursorScope([u8; CURSOR_SCOPE_BYTES]);

impl ListCursorScope {
    pub(crate) fn new(account_key: &str, filter: &SubscriptionListFilter) -> Self {
        let mut context = Context::new(&SHA256);
        context.update(SCOPE_DOMAIN);
        context.update(&(account_key.len() as u64).to_be_bytes());
        context.update(account_key.as_bytes());
        context.update(&[
            active_scope_code(filter.active),
            media_scope_code(filter.media_kind),
            lifecycle_scope_code(filter.lifecycle_state),
            attention_scope_code(filter.attention_tag),
        ]);
        let digest = context.finish();
        let mut bytes = [0; CURSOR_SCOPE_BYTES];
        bytes.copy_from_slice(digest.as_ref());
        Self(bytes)
    }

    #[cfg(test)]
    pub(crate) fn from_command(command: &ListSubscriptionsCommand) -> Self {
        Self::new(&command.account_key, &command.filter)
    }
}

/// Versioned public cursor whose ordering key remains opaque to clients.
///
/// V2 binds the validated `(douban_sort_time, subject_id)` key to a canonical
/// account-and-filter scope. Invalid, unsupported, or oversized external tokens
/// cannot be constructed through deserialization or `TryFrom`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpaqueListCursor {
    token: String,
    scope: ListCursorScope,
    cursor: ListCursor,
}

impl OpaqueListCursor {
    pub(crate) fn encode(
        cursor: &ListCursor,
        scope: ListCursorScope,
    ) -> Result<Self, CursorCodecError> {
        let subject = cursor.subject_id.as_bytes();
        let sort_bytes = if cursor.douban_sort_time.is_some() {
            8
        } else {
            0
        };
        let payload_len = CURSOR_SCOPE_BYTES
            .checked_add(1)
            .and_then(|length| length.checked_add(sort_bytes))
            .and_then(|length| length.checked_add(subject.len()))
            .ok_or(CursorCodecError::Oversized {
                max_length: MAX_CURSOR_TOKEN_LEN,
            })?;
        let token_len = payload_len
            .checked_mul(2)
            .and_then(|length| length.checked_add(CURSOR_V2_PREFIX.len()))
            .ok_or(CursorCodecError::Oversized {
                max_length: MAX_CURSOR_TOKEN_LEN,
            })?;
        if token_len > MAX_CURSOR_TOKEN_LEN {
            return Err(CursorCodecError::Oversized {
                max_length: MAX_CURSOR_TOKEN_LEN,
            });
        }

        let mut payload = Vec::with_capacity(payload_len);
        payload.extend_from_slice(&scope.0);
        match cursor.douban_sort_time {
            Some(sort_time) => {
                payload.push(1);
                payload.extend_from_slice(&sort_time.to_be_bytes());
            }
            None => payload.push(0),
        }
        payload.extend_from_slice(subject);

        let mut token = String::with_capacity(token_len);
        token.push_str(CURSOR_V2_PREFIX);
        encode_hex(&payload, &mut token);
        Self::try_from(token)
    }

    pub(crate) fn decode(
        &self,
        expected_scope: ListCursorScope,
    ) -> Result<ListCursor, CursorCodecError> {
        if self.scope != expected_scope {
            return Err(CursorCodecError::ScopeMismatch);
        }
        Ok(self.cursor.clone())
    }

    #[cfg(test)]
    pub(crate) fn as_str(&self) -> &str {
        &self.token
    }
}

impl TryFrom<String> for OpaqueListCursor {
    type Error = CursorCodecError;

    fn try_from(token: String) -> Result<Self, Self::Error> {
        let decoded = decode_token(&token)?;
        Ok(Self {
            token,
            scope: decoded.scope,
            cursor: decoded.cursor,
        })
    }
}

impl TryFrom<&str> for OpaqueListCursor {
    type Error = CursorCodecError;

    fn try_from(token: &str) -> Result<Self, Self::Error> {
        let decoded = decode_token(token)?;
        Ok(Self {
            token: token.to_string(),
            scope: decoded.scope,
            cursor: decoded.cursor,
        })
    }
}

impl Serialize for OpaqueListCursor {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.token)
    }
}

impl<'de> Deserialize<'de> for OpaqueListCursor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(CursorVisitor)
    }
}

struct CursorVisitor;

impl Visitor<'_> for CursorVisitor {
    type Value = OpaqueListCursor;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "a valid subscription cursor no longer than {MAX_CURSOR_TOKEN_LEN} bytes"
        )
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        OpaqueListCursor::try_from(value).map_err(E::custom)
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        OpaqueListCursor::try_from(value).map_err(E::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CursorCodecError {
    UnsupportedVersion { version: String },
    InvalidEncoding,
    InvalidPayload,
    Oversized { max_length: usize },
    ScopeMismatch,
}

impl fmt::Display for CursorCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion { version } => {
                write!(
                    formatter,
                    "unsupported subscription cursor version {version:?}"
                )
            }
            Self::InvalidEncoding => formatter.write_str("subscription cursor encoding is invalid"),
            Self::InvalidPayload => formatter.write_str("subscription cursor payload is invalid"),
            Self::Oversized { max_length } => write!(
                formatter,
                "subscription cursor exceeds the {max_length}-byte limit"
            ),
            Self::ScopeMismatch => {
                formatter.write_str("subscription cursor does not match the requested list scope")
            }
        }
    }
}

impl Error for CursorCodecError {}

struct DecodedCursor {
    scope: ListCursorScope,
    cursor: ListCursor,
}

fn decode_token(token: &str) -> Result<DecodedCursor, CursorCodecError> {
    if token.len() > MAX_CURSOR_TOKEN_LEN {
        return Err(CursorCodecError::Oversized {
            max_length: MAX_CURSOR_TOKEN_LEN,
        });
    }
    let (version, encoded) = token
        .split_once('.')
        .ok_or(CursorCodecError::InvalidEncoding)?;
    if version != "v2" {
        return Err(CursorCodecError::UnsupportedVersion {
            version: version.to_string(),
        });
    }
    let payload = decode_hex(encoded)?;
    let scope_bytes: [u8; CURSOR_SCOPE_BYTES] = payload
        .get(..CURSOR_SCOPE_BYTES)
        .ok_or(CursorCodecError::InvalidPayload)?
        .try_into()
        .map_err(|_| CursorCodecError::InvalidPayload)?;
    let remainder = &payload[CURSOR_SCOPE_BYTES..];
    let (&tag, remainder) = remainder
        .split_first()
        .ok_or(CursorCodecError::InvalidPayload)?;
    let (douban_sort_time, subject_bytes) = match tag {
        0 => (None, remainder),
        1 => {
            let sort_bytes: [u8; 8] = remainder
                .get(..8)
                .ok_or(CursorCodecError::InvalidPayload)?
                .try_into()
                .map_err(|_| CursorCodecError::InvalidPayload)?;
            (Some(u64::from_be_bytes(sort_bytes)), &remainder[8..])
        }
        _ => return Err(CursorCodecError::InvalidPayload),
    };
    let subject_id = std::str::from_utf8(subject_bytes)
        .map_err(|_| CursorCodecError::InvalidPayload)?
        .to_string();
    let cursor = ListCursor::try_new(douban_sort_time, subject_id)
        .map_err(|_| CursorCodecError::InvalidPayload)?;
    Ok(DecodedCursor {
        scope: ListCursorScope(scope_bytes),
        cursor,
    })
}

fn encode_hex(bytes: &[u8], output: &mut String) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
}

fn decode_hex(encoded: &str) -> Result<Vec<u8>, CursorCodecError> {
    if encoded.is_empty() || !encoded.len().is_multiple_of(2) {
        return Err(CursorCodecError::InvalidEncoding);
    }
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let high = decode_hex_nibble(pair[0]).ok_or(CursorCodecError::InvalidEncoding)?;
        let low = decode_hex_nibble(pair[1]).ok_or(CursorCodecError::InvalidEncoding)?;
        decoded.push((high << 4) | low);
    }
    Ok(decoded)
}

const fn decode_hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

const fn active_scope_code(value: Option<bool>) -> u8 {
    match value {
        None => 0,
        Some(false) => 1,
        Some(true) => 2,
    }
}

const fn media_scope_code(value: Option<SubscriptionMediaKind>) -> u8 {
    match value {
        None => 0,
        Some(SubscriptionMediaKind::Movie) => 1,
        Some(SubscriptionMediaKind::Tv) => 2,
    }
}

const fn lifecycle_scope_code(value: Option<SubscriptionLifecycleState>) -> u8 {
    match value {
        None => 0,
        Some(SubscriptionLifecycleState::Queued) => 1,
        Some(SubscriptionLifecycleState::Meta) => 2,
        Some(SubscriptionLifecycleState::Searching) => 3,
        Some(SubscriptionLifecycleState::Downloading) => 4,
        Some(SubscriptionLifecycleState::Linking) => 5,
        Some(SubscriptionLifecycleState::Completed) => 6,
    }
}

const fn attention_scope_code(value: Option<SubscriptionAttentionTag>) -> u8 {
    match value {
        None => 0,
        Some(SubscriptionAttentionTag::WaitingRelease) => 1,
        Some(SubscriptionAttentionTag::Failed) => 2,
        Some(SubscriptionAttentionTag::RetryBlocked) => 3,
        Some(SubscriptionAttentionTag::Skipped) => 4,
        Some(SubscriptionAttentionTag::NeedsReconciliation) => 5,
    }
}
