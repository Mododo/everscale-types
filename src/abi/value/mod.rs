use std::collections::BTreeMap;
use std::num::NonZeroU8;
use std::sync::Arc;

use bytes::Bytes;
use everscale_crypto::ed25519;
use num_bigint::{BigInt, BigUint};

use super::ty::*;
use crate::cell::Cell;
use crate::models::IntAddr;
use crate::num::Tokens;

mod de;
mod ser;

/// ABI value with name.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NamedAbiValue {
    /// Item name.
    pub name: String,
    /// ABI value.
    pub value: AbiValue,
}

impl NamedAbiValue {
    /// Returns whether all values satisfy the provided types.
    pub fn have_types(items: &[Self], types: &[NamedAbiType]) -> bool {
        items.len() == types.len()
            && items
                .iter()
                .zip(types.iter())
                .all(|(item, t)| item.value.has_type(&t.ty))
    }

    /// Creates a named ABI value with an index name (e.g. `value123`).
    pub fn from_index(index: usize, value: AbiValue) -> Self {
        Self {
            name: format!("value{index}"),
            value,
        }
    }
}

impl From<(String, AbiValue)> for NamedAbiValue {
    #[inline]
    fn from((name, value): (String, AbiValue)) -> Self {
        Self { name, value }
    }
}

impl<'a> From<(&'a str, AbiValue)> for NamedAbiValue {
    #[inline]
    fn from((name, value): (&'a str, AbiValue)) -> Self {
        Self {
            name: name.to_owned(),
            value,
        }
    }
}

impl From<(usize, AbiValue)> for NamedAbiValue {
    #[inline]
    fn from((index, value): (usize, AbiValue)) -> Self {
        Self::from_index(index, value)
    }
}

/// ABI encoded value.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum AbiValue {
    /// Unsigned integer of n bits.
    Uint(u16, BigUint),
    /// Signed integer of n bits.
    Int(u16, BigInt),
    /// Variable-length unsigned integer of maximum n bytes.
    VarUint(NonZeroU8, BigUint),
    /// Variable-length signed integer of maximum n bytes.
    VarInt(NonZeroU8, BigInt),
    /// Boolean.
    Bool(bool),
    /// Tree of cells ([`Cell`]).
    ///
    /// [`Cell`]: crate::cell::Cell
    Cell(Cell),
    /// Internal address ([`IntAddr`]).
    ///
    /// [`IntAddr`]: crate::models::message::IntAddr
    Address(Box<IntAddr>),
    /// Byte array.
    Bytes(Bytes),
    /// Byte array of fixed length.
    FixedBytes(Bytes),
    /// Utf8-encoded string.
    String(String),
    /// Variable length 120-bit integer ([`Tokens`]).
    ///
    /// [`Tokens`]: crate::num::Tokens
    Token(Tokens),
    /// Product type.
    Tuple(Vec<NamedAbiValue>),
    /// Array of ABI values.
    Array(Arc<AbiType>, Vec<Self>),
    /// Fixed-length array of ABI values.
    FixedArray(Arc<AbiType>, Vec<Self>),
    /// Sorted dictionary of ABI values.
    Map(
        PlainAbiType,
        Arc<AbiType>,
        BTreeMap<PlainAbiValue, AbiValue>,
    ),
    /// Optional value.
    Optional(Arc<AbiType>, Option<Box<Self>>),
    /// Value stored in a new cell.
    Ref(Box<Self>),
}

impl AbiValue {
    /// Returns whether this value has the same type as the provided one.
    pub fn has_type(&self, ty: &AbiType) -> bool {
        match (self, ty) {
            (Self::Uint(n, _), AbiType::Uint(t)) => n == t,
            (Self::Int(n, _), AbiType::Int(t)) => n == t,
            (Self::VarUint(n, _), AbiType::VarUint(t)) => n == t,
            (Self::VarInt(n, _), AbiType::VarInt(t)) => n == t,
            (Self::FixedBytes(bytes), AbiType::FixedBytes(len)) => bytes.len() == *len,
            (Self::Tuple(items), AbiType::Tuple(types)) => NamedAbiValue::have_types(items, types),
            (Self::Array(ty, _), AbiType::Array(t)) => ty == t,
            (Self::FixedArray(ty, items), AbiType::FixedArray(t, len)) => {
                items.len() == *len && ty == t
            }
            (Self::Map(key_ty, value_ty, _), AbiType::Map(k, v)) => key_ty == k && value_ty == v,
            (Self::Optional(ty, _), AbiType::Optional(t)) => ty == t,
            (Self::Ref(value), AbiType::Ref(t)) => value.has_type(t),
            (Self::Bool(_), AbiType::Bool)
            | (Self::Cell(_), AbiType::Cell)
            | (Self::Address(_), AbiType::Address)
            | (Self::Bytes(_), AbiType::Bytes)
            | (Self::String(_), AbiType::String)
            | (Self::Token(_), AbiType::Token) => true,
            _ => false,
        }
    }

    /// Returns an ABI type of the value.
    pub fn get_type(&self) -> AbiType {
        match self {
            AbiValue::Uint(n, _) => AbiType::Uint(*n),
            AbiValue::Int(n, _) => AbiType::Int(*n),
            AbiValue::VarUint(n, _) => AbiType::VarUint(*n),
            AbiValue::VarInt(n, _) => AbiType::VarInt(*n),
            AbiValue::Bool(_) => AbiType::Bool,
            AbiValue::Cell(_) => AbiType::Cell,
            AbiValue::Address(_) => AbiType::Address,
            AbiValue::Bytes(_) => AbiType::Bytes,
            AbiValue::FixedBytes(bytes) => AbiType::FixedBytes(bytes.len()),
            AbiValue::String(_) => AbiType::String,
            AbiValue::Token(_) => AbiType::Token,
            AbiValue::Tuple(items) => AbiType::Tuple(
                items
                    .iter()
                    .map(|item| NamedAbiType::new(item.name.clone(), item.value.get_type()))
                    .collect(),
            ),
            AbiValue::Array(ty, _) => AbiType::Array(ty.clone()),
            AbiValue::FixedArray(ty, items) => AbiType::FixedArray(ty.clone(), items.len()),
            AbiValue::Map(key_ty, value_ty, _) => AbiType::Map(*key_ty, value_ty.clone()),
            AbiValue::Optional(ty, _) => AbiType::Optional(ty.clone()),
            AbiValue::Ref(value) => AbiType::Ref(Arc::new(value.get_type())),
        }
    }

    /// Simple `uintN` constructor.
    #[inline]
    pub fn uint<T>(bits: u16, value: T) -> Self
    where
        BigUint: From<T>,
    {
        Self::Uint(bits, BigUint::from(value))
    }

    /// Simple `intN` constructor.
    #[inline]
    pub fn int<T>(bits: u16, value: T) -> Self
    where
        BigInt: From<T>,
    {
        Self::Int(bits, BigInt::from(value))
    }

    /// Simple `address` constructor.
    #[inline]
    pub fn address<T>(value: T) -> Self
    where
        IntAddr: From<T>,
    {
        Self::Address(Box::new(IntAddr::from(value)))
    }

    /// Simple `bytes` constructor.
    #[inline]
    pub fn bytes<T>(value: T) -> Self
    where
        Bytes: From<T>,
    {
        Self::Bytes(Bytes::from(value))
    }

    /// Simple `bytes` constructor.
    #[inline]
    pub fn fixedbytes<T>(value: T) -> Self
    where
        Bytes: From<T>,
    {
        Self::FixedBytes(Bytes::from(value))
    }

    /// Simple `tuple` constructor.
    #[inline]
    pub fn tuple<I, T>(values: I) -> Self
    where
        I: IntoIterator<Item = T>,
        NamedAbiValue: From<T>,
    {
        Self::Tuple(values.into_iter().map(NamedAbiValue::from).collect())
    }

    /// Simple `tuple` constructor.
    #[inline]
    pub fn unnamed_tuple<I>(values: I) -> Self
    where
        I: IntoIterator<Item = AbiValue>,
    {
        Self::Tuple(
            values
                .into_iter()
                .enumerate()
                .map(|(i, value)| NamedAbiValue::from_index(i, value))
                .collect(),
        )
    }
}

/// ABI value which has a fixed bits representation
/// and therefore can be used as a map key.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub enum PlainAbiValue {
    /// Unsigned integer of n bits.
    Uint(u16, BigUint),
    /// Signed integer of n bits.
    Int(u16, BigInt),
    /// Boolean.
    Bool(bool),
    /// Internal address ([`IntAddr`]).
    ///
    /// [`IntAddr`]: crate::models::message::IntAddr
    Address(Box<IntAddr>),
}

impl PlainAbiValue {
    /// Returns whether this value has the same type as the provided one.
    pub fn has_type(&self, ty: &PlainAbiType) -> bool {
        match (self, ty) {
            (Self::Uint(n, _), PlainAbiType::Uint(t)) => n == t,
            (Self::Int(n, _), PlainAbiType::Int(t)) => n == t,
            (Self::Bool(_), PlainAbiType::Bool) | (Self::Address(_), PlainAbiType::Address) => true,
            _ => false,
        }
    }
}

impl From<PlainAbiValue> for AbiValue {
    fn from(value: PlainAbiValue) -> Self {
        match value {
            PlainAbiValue::Uint(n, value) => Self::Uint(n, value),
            PlainAbiValue::Int(n, value) => Self::Int(n, value),
            PlainAbiValue::Bool(value) => Self::Bool(value),
            PlainAbiValue::Address(value) => Self::Address(value),
        }
    }
}

/// ABI header value.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum AbiHeader {
    /// `time` header.
    Time(u64),
    /// `expire` header.
    Expire(u32),
    /// `pubkey` header.
    PublicKey(Option<Box<ed25519::PublicKey>>),
}

impl AbiHeader {
    /// Returns whether this value has the same type as the provided one.
    pub fn has_type(&self, ty: &AbiHeaderType) -> bool {
        matches!(
            (self, ty),
            (Self::Time(_), AbiHeaderType::Time)
                | (Self::Expire(_), AbiHeaderType::Expire)
                | (Self::PublicKey(_), AbiHeaderType::PublicKey)
        )
    }
}
