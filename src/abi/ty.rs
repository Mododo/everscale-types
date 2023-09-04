use std::borrow::Cow;
use std::num::NonZeroU8;
use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::error::{ParseAbiTypeError, ParseNamedAbiTypeError};
use crate::cell::{MAX_BIT_LEN, MAX_REF_COUNT};
use crate::models::IntAddr;
use crate::num::Tokens;

/// ABI value type with name.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct NamedAbiType {
    /// Item name.
    pub name: Arc<str>,
    /// ABI value type.
    pub ty: AbiType,
}

impl NamedAbiType {
    /// Creates a named ABI type.
    #[inline]
    pub fn new<T>(name: T, ty: AbiType) -> Self
    where
        T: Into<Arc<str>>,
    {
        Self {
            name: name.into(),
            ty,
        }
    }

    /// Creates a named ABI type with an index name (e.g. `value123`).
    pub fn from_index(index: usize, ty: AbiType) -> Self {
        Self::new(format!("value{index}"), ty)
    }
}

impl Serialize for NamedAbiType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        #[derive(Serialize)]
        struct Helper<'a> {
            name: &'a str,
            #[serde(rename = "type", serialize_with = "collect_str")]
            ty: DisplayAbiTypeSimple<'a>,
            #[serde(skip_serializing_if = "Option::is_none")]
            components: Option<&'a [NamedAbiType]>,
        }

        Helper {
            name: &self.name,
            ty: self.ty.display_simple(),
            components: self.ty.components(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for NamedAbiType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        #[derive(Deserialize)]
        struct Helper<'a> {
            name: String,
            #[serde(rename = "type", borrow)]
            ty: Cow<'a, str>,
            #[serde(default)]
            components: Option<Vec<Helper<'a>>>,
        }

        impl TryFrom<Helper<'_>> for NamedAbiType {
            type Error = ParseNamedAbiTypeError;

            fn try_from(value: Helper<'_>) -> Result<Self, Self::Error> {
                let mut ty = match AbiType::from_simple_str(&value.ty) {
                    Ok(ty) => ty,
                    Err(error) => {
                        return Err(ParseNamedAbiTypeError::InvalidType {
                            ty: value.ty.into(),
                            error,
                        });
                    }
                };

                match (ty.components_mut(), value.components) {
                    (Some(ty), Some(components)) => {
                        *ty = ok!(components
                            .into_iter()
                            .map(Self::try_from)
                            .collect::<Result<Arc<[_]>, _>>());
                    }
                    (Some(_), None) => {
                        return Err(ParseNamedAbiTypeError::ExpectedComponents {
                            ty: value.ty.into(),
                        })
                    }
                    (None, Some(_)) => {
                        return Err(ParseNamedAbiTypeError::UnexpectedComponents {
                            ty: value.ty.into(),
                        });
                    }
                    (None, None) => {}
                }

                Ok(Self {
                    name: value.name.into(),
                    ty,
                })
            }
        }

        let helper = ok!(<Helper as Deserialize>::deserialize(deserializer));
        helper.try_into().map_err(Error::custom)
    }
}

impl From<(String, AbiType)> for NamedAbiType {
    #[inline]
    fn from((name, ty): (String, AbiType)) -> Self {
        Self {
            name: name.into(),
            ty,
        }
    }
}

impl<'a> From<(&'a str, AbiType)> for NamedAbiType {
    #[inline]
    fn from((name, ty): (&'a str, AbiType)) -> Self {
        Self {
            name: Arc::from(name),
            ty,
        }
    }
}

impl From<(usize, AbiType)> for NamedAbiType {
    #[inline]
    fn from((index, ty): (usize, AbiType)) -> Self {
        Self::from_index(index, ty)
    }
}

/// Contract header value type.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum AbiHeaderType {
    /// Time header type. Serialized as `uint64`.
    Time,
    /// Expire header type. Serialized as `uint32`.
    Expire,
    /// Public key type. Serialized as `optional(uint256)`.
    PublicKey,
}

impl FromStr for AbiHeaderType {
    type Err = ParseAbiTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "time" => Self::Time,
            "expire" => Self::Expire,
            "pubkey" => Self::PublicKey,
            _ => return Err(ParseAbiTypeError::UnknownType),
        })
    }
}

impl std::fmt::Display for AbiHeaderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Time => "time",
            Self::Expire => "expire",
            Self::PublicKey => "pubkey",
        })
    }
}

/// ABI value type.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum AbiType {
    /// Unsigned integer of n bits.
    Uint(u16),
    /// Signed integer of n bits.
    Int(u16),
    /// Variable-length unsigned integer of maximum n bytes.
    VarUint(NonZeroU8),
    /// Variable-length signed integer of maximum n bytes.
    VarInt(NonZeroU8),
    /// Boolean.
    Bool,
    /// Tree of cells ([`Cell`]).
    ///
    /// [`Cell`]: crate::cell::Cell
    Cell,
    /// Internal address ([`IntAddr`]).
    ///
    /// [`IntAddr`]: crate::models::message::IntAddr
    Address,
    /// Byte array.
    Bytes,
    /// Byte array of fixed length.
    FixedBytes(usize),
    /// Utf8-encoded string.
    String,
    /// Variable length 120-bit integer ([`Tokens`]).
    ///
    /// [`Tokens`]: crate::num::Tokens
    Token,
    /// Product type.
    Tuple(Arc<[NamedAbiType]>),
    /// Array of elements of the specified ABI type.
    Array(Arc<Self>),
    /// Fixed-length array of elements of the specified ABI type.
    FixedArray(Arc<Self>, usize),
    /// Dictionary with the specified key and value ABI types.
    Map(PlainAbiType, Arc<Self>),
    /// Optional type.
    Optional(Arc<Self>),
    /// Type stored in a new cell.
    Ref(Arc<Self>),
}

impl AbiType {
    /// Tries to convert a generic ABI type into a plain ABI type.
    pub fn as_plain(&self) -> Option<PlainAbiType> {
        Some(match self {
            Self::Uint(n) => PlainAbiType::Uint(*n),
            Self::Int(n) => PlainAbiType::Int(*n),
            Self::Bool => PlainAbiType::Bool,
            Self::Address => PlainAbiType::Address,
            _ => return None,
        })
    }

    /// Returns the maximum number of bits and refs that this type can occupy.
    pub fn max_size(&self) -> (usize, usize) {
        match self {
            Self::Uint(n) | Self::Int(n) => (*n as usize, 0),
            Self::VarUint(n) | Self::VarInt(n) => {
                let value_bytes: u8 = n.get() - 1;
                let bits = (8 - value_bytes.leading_zeros()) as usize + (value_bytes as usize * 8);
                (bits, 0)
            }
            Self::Bool => (1, 0),
            Self::Cell | Self::Bytes | Self::FixedBytes(_) | Self::String | Self::Ref(_) => (0, 1),
            Self::Address => (IntAddr::BITS_MAX as usize, 0),
            Self::Token => (Tokens::MAX_BITS as usize, 0),
            Self::Array(_) => (33, 1),
            Self::FixedArray(..) | Self::Map(..) => (1, 1),
            Self::Optional(ty) => {
                let (max_ty_bits, max_ty_refs) = ty.max_size();
                if max_ty_bits < MAX_BIT_LEN as usize && max_ty_refs < MAX_REF_COUNT {
                    (1 + max_ty_bits, max_ty_refs)
                } else {
                    (1, 1)
                }
            }
            Self::Tuple(items) => items.iter().fold((0, 0), |(bits, refs), item| {
                let (item_bits, item_refs) = item.ty.max_size();
                (bits + item_bits, refs + item_refs)
            }),
        }
    }

    /// Returns the maximum number of bits that this type can occupy.
    pub fn max_bits(&self) -> usize {
        self.max_size().0
    }

    /// Returns the maximum number of cells that this type can occupy.
    pub fn max_refs(&self) -> usize {
        self.max_size().1
    }

    fn components(&self) -> Option<&[NamedAbiType]> {
        match self {
            Self::Tuple(types) => Some(types),
            Self::Array(ty) => ty.components(),
            Self::FixedArray(ty, _) => ty.components(),
            Self::Map(_, value_ty) => value_ty.components(),
            Self::Optional(ty) => ty.components(),
            Self::Ref(ty) => ty.components(),
            _ => None,
        }
    }

    fn components_mut(&mut self) -> Option<&mut Arc<[NamedAbiType]>> {
        match self {
            Self::Tuple(types) => Some(types),
            Self::Array(ty) => Arc::make_mut(ty).components_mut(),
            Self::FixedArray(ty, _) => Arc::make_mut(ty).components_mut(),
            Self::Map(_, value_ty) => Arc::make_mut(value_ty).components_mut(),
            Self::Optional(ty) => Arc::make_mut(ty).components_mut(),
            Self::Ref(ty) => Arc::make_mut(ty).components_mut(),
            _ => None,
        }
    }

    /// Simple `varuintN` type constructor.
    #[inline]
    pub fn varuint(size: u8) -> Self {
        Self::VarUint(NonZeroU8::new(size).unwrap())
    }

    /// Simple `varintN` type constructor.
    #[inline]
    pub fn varint(size: u8) -> Self {
        Self::VarInt(NonZeroU8::new(size).unwrap())
    }

    /// Simple `tuple` type constructor.
    #[inline]
    pub fn tuple<I, T>(values: I) -> Self
    where
        I: IntoIterator<Item = T>,
        NamedAbiType: From<T>,
    {
        Self::Tuple(values.into_iter().map(NamedAbiType::from).collect())
    }

    /// Simple `tuple` type constructor.
    #[inline]
    pub fn unnamed_tuple<I>(values: I) -> Self
    where
        I: IntoIterator<Item = AbiType>,
    {
        Self::Tuple(
            values
                .into_iter()
                .enumerate()
                .map(|(i, ty)| NamedAbiType::from_index(i, ty))
                .collect(),
        )
    }

    /// Simple `array` type constructor.
    #[inline]
    pub fn array<T>(ty: T) -> Self
    where
        Arc<AbiType>: From<T>,
    {
        Self::Array(Arc::<AbiType>::from(ty))
    }

    /// Simple `fixedarrayN` type constructor.
    #[inline]
    pub fn fixedarray<T>(ty: T, len: usize) -> Self
    where
        Arc<AbiType>: From<T>,
    {
        Self::FixedArray(Arc::<AbiType>::from(ty), len)
    }

    /// Simple `tuple` type constructor.
    #[inline]
    pub fn map<V>(key_ty: PlainAbiType, value_ty: V) -> Self
    where
        Arc<AbiType>: From<V>,
    {
        Self::Map(key_ty, Arc::<AbiType>::from(value_ty))
    }

    /// Simple `optional` type constructor.
    #[inline]
    pub fn optional<T>(ty: T) -> Self
    where
        Arc<AbiType>: From<T>,
    {
        Self::Optional(Arc::<AbiType>::from(ty))
    }

    /// Simple `ref` type constructor.
    #[inline]
    pub fn reference<T>(ty: T) -> Self
    where
        Arc<AbiType>: From<T>,
    {
        Self::Ref(Arc::<AbiType>::from(ty))
    }

    fn from_simple_str(s: &str) -> Result<Self, ParseAbiTypeError> {
        if let Some(arr_ty) = s.strip_suffix(']') {
            let (ty, len) = ok!(arr_ty
                .rsplit_once('[')
                .ok_or(ParseAbiTypeError::InvalidArrayType));

            let len = if len.is_empty() {
                None
            } else {
                Some(ok!(len
                    .parse::<usize>()
                    .map_err(ParseAbiTypeError::InvalidArrayLength)))
            };

            let ty = ok!(Self::from_simple_str(ty).map(Arc::new));
            return Ok(match len {
                None => Self::Array(ty),
                Some(len) => Self::FixedArray(ty, len),
            });
        }

        Ok(match s {
            "bool" => Self::Bool,
            "cell" => Self::Cell,
            "address" => Self::Address,
            "bytes" => Self::Bytes,
            "string" => Self::String,
            "gram" | "token" => Self::Token,
            "tuple" => Self::Tuple(Arc::from([].as_slice())),
            _ => {
                if let Some(s) = s.strip_prefix("uint") {
                    Self::Uint(ok!(s
                        .parse::<u16>()
                        .map_err(ParseAbiTypeError::InvalidBitLen)))
                } else if let Some(s) = s.strip_prefix("int") {
                    Self::Int(ok!(s
                        .parse::<u16>()
                        .map_err(ParseAbiTypeError::InvalidBitLen)))
                } else if let Some(s) = s.strip_prefix("varuint") {
                    Self::VarUint(ok!(s
                        .parse::<NonZeroU8>()
                        .map_err(ParseAbiTypeError::InvalidByteLen)))
                } else if let Some(s) = s.strip_prefix("varint") {
                    Self::VarInt(ok!(s
                        .parse::<NonZeroU8>()
                        .map_err(ParseAbiTypeError::InvalidByteLen)))
                } else if let Some(s) = s.strip_prefix("fixedbytes") {
                    Self::FixedBytes(ok!(s
                        .parse::<usize>()
                        .map_err(ParseAbiTypeError::InvalidByteLen)))
                } else if let Some(s) = s.strip_prefix("map(") {
                    let s = ok!(s
                        .strip_suffix(')')
                        .ok_or(ParseAbiTypeError::UnterminatedInnerType));
                    let (key_ty, value_ty) = ok!(s
                        .split_once(',')
                        .ok_or(ParseAbiTypeError::ValueTypeNotFound));

                    Self::Map(
                        ok!(PlainAbiType::from_str(key_ty)),
                        ok!(Self::from_simple_str(value_ty).map(Arc::new)),
                    )
                } else if let Some(s) = s.strip_prefix("optional(") {
                    let s = ok!(s
                        .strip_suffix(')')
                        .ok_or(ParseAbiTypeError::UnterminatedInnerType));

                    Self::Optional(ok!(Self::from_simple_str(s).map(Arc::new)))
                } else if let Some(s) = s.strip_prefix("ref(") {
                    let s = ok!(s
                        .strip_suffix(')')
                        .ok_or(ParseAbiTypeError::UnterminatedInnerType));

                    Self::Ref(ok!(Self::from_simple_str(s).map(Arc::new)))
                } else {
                    return Err(ParseAbiTypeError::UnknownType);
                }
            }
        })
    }

    fn display_simple(&self) -> DisplayAbiTypeSimple<'_> {
        DisplayAbiTypeSimple(self)
    }
}

impl std::fmt::Display for AbiType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Uint(n) => return write!(f, "uint{n}"),
            Self::Int(n) => return write!(f, "int{n}"),
            Self::VarUint(n) => return write!(f, "varuint{n}"),
            Self::VarInt(n) => return write!(f, "varint{n}"),
            Self::Bool => "bool",
            Self::Cell => "cell",
            Self::Address => "address",
            Self::Bytes => "bytes",
            Self::FixedBytes(n) => return write!(f, "fixedbytes{n}"),
            Self::String => "string",
            Self::Token => "gram",
            Self::Tuple(items) => {
                if items.is_empty() {
                    "()"
                } else {
                    let mut first = true;
                    ok!(f.write_str("("));
                    for item in items.as_ref() {
                        if !std::mem::take(&mut first) {
                            ok!(f.write_str(","));
                        }
                        ok!(std::fmt::Display::fmt(&item.ty, f));
                    }
                    ")"
                }
            }
            Self::Array(ty) => return write!(f, "{ty}[]"),
            Self::FixedArray(ty, n) => return write!(f, "{ty}[{n}]"),
            Self::Map(key_ty, value_ty) => return write!(f, "map({key_ty},{value_ty})"),
            Self::Optional(ty) => return write!(f, "optional({ty})"),
            Self::Ref(ty) => return write!(f, "ref({ty})"),
        };
        f.write_str(s)
    }
}

#[derive(Clone, Copy)]
struct DisplayAbiTypeSimple<'a>(&'a AbiType);

impl std::fmt::Display for DisplayAbiTypeSimple<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            AbiType::Tuple(_) => f.write_str("tuple"),
            AbiType::Array(ty) => write!(f, "{}[]", ty.display_simple()),
            AbiType::FixedArray(ty, n) => write!(f, "{}[{n}]", ty.display_simple()),
            AbiType::Map(key_ty, value_ty) => {
                write!(f, "map({key_ty},{})", value_ty.display_simple())
            }
            AbiType::Optional(ty) => write!(f, "optional({})", ty.display_simple()),
            AbiType::Ref(ty) => write!(f, "ref({})", ty.display_simple()),
            ty => std::fmt::Display::fmt(ty, f),
        }
    }
}

/// ABI type which has a fixed bits representation
/// and therefore can be used as a map key.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum PlainAbiType {
    /// Unsigned integer of n bits.
    Uint(u16),
    /// Signed integer of n bits.
    Int(u16),
    /// Boolean.
    Bool,
    /// Internal address ([`IntAddr`]).
    ///
    /// [`IntAddr`]: crate::models::message::IntAddr
    Address,
}

impl PlainAbiType {
    /// Returns the maximum number of bits that this type can occupy.
    pub fn max_bits(&self) -> u16 {
        match self {
            Self::Uint(n) | Self::Int(n) => *n,
            Self::Bool => 1,
            Self::Address => IntAddr::BITS_MAX,
        }
    }
}

impl From<PlainAbiType> for AbiType {
    fn from(value: PlainAbiType) -> Self {
        match value {
            PlainAbiType::Uint(n) => Self::Uint(n),
            PlainAbiType::Int(n) => Self::Int(n),
            PlainAbiType::Bool => Self::Bool,
            PlainAbiType::Address => Self::Address,
        }
    }
}

impl FromStr for PlainAbiType {
    type Err = ParseAbiTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "bool" => Self::Bool,
            "address" => Self::Address,
            s => {
                if let Some(s) = s.strip_prefix("uint") {
                    Self::Uint(ok!(s
                        .parse::<u16>()
                        .map_err(ParseAbiTypeError::InvalidBitLen)))
                } else if let Some(s) = s.strip_prefix("int") {
                    Self::Int(ok!(s
                        .parse::<u16>()
                        .map_err(ParseAbiTypeError::InvalidBitLen)))
                } else {
                    return Err(ParseAbiTypeError::UnknownType);
                }
            }
        })
    }
}

impl std::fmt::Display for PlainAbiType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Uint(n) => return write!(f, "uint{n}"),
            Self::Int(n) => return write!(f, "int{n}"),
            Self::Bool => "bool",
            Self::Address => "address",
        };
        f.write_str(s)
    }
}

#[inline]
fn collect_str<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
where
    T: std::fmt::Display,
    S: serde::ser::Serializer,
{
    serializer.collect_str(value)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::abi::traits::{WithAbiType, WithPlainAbiType};

    #[test]
    fn correct_full_signature() {
        macro_rules! assert_eq_sig {
            ($expr:expr, $signature:literal) => {
                assert_eq!(($expr).to_string(), $signature)
            };
        }

        assert_eq_sig!(AbiType::Uint(100), "uint100");
        assert_eq_sig!(AbiType::Int(100), "int100");
        assert_eq_sig!(AbiType::varuint(16), "varuint16");
        assert_eq_sig!(AbiType::varint(16), "varint16");
        assert_eq_sig!(AbiType::Bool, "bool");
        assert_eq_sig!(AbiType::Cell, "cell");
        assert_eq_sig!(AbiType::Address, "address");
        assert_eq_sig!(AbiType::Bytes, "bytes");
        assert_eq_sig!(AbiType::FixedBytes(123), "fixedbytes123");
        assert_eq_sig!(AbiType::String, "string");
        assert_eq_sig!(AbiType::Token, "gram");

        assert_eq_sig!(AbiType::unnamed_tuple([]), "()");
        assert_eq_sig!(AbiType::unnamed_tuple([AbiType::Uint(321)]), "(uint321)");
        assert_eq_sig!(
            AbiType::unnamed_tuple([AbiType::Uint(123), AbiType::Address]),
            "(uint123,address)"
        );

        assert_eq_sig!(AbiType::array(AbiType::Address), "address[]");
        assert_eq_sig!(
            AbiType::array(AbiType::array(AbiType::Address)),
            "address[][]"
        );
        assert_eq_sig!(AbiType::array(AbiType::unnamed_tuple([])), "()[]");
        assert_eq_sig!(
            AbiType::array(AbiType::unnamed_tuple([AbiType::Address, AbiType::Bool])),
            "(address,bool)[]"
        );

        assert_eq_sig!(AbiType::fixedarray(AbiType::Address, 10), "address[10]");
        assert_eq_sig!(
            AbiType::fixedarray(AbiType::fixedarray(AbiType::Address, 123), 321),
            "address[123][321]"
        );
        assert_eq_sig!(
            AbiType::fixedarray(AbiType::unnamed_tuple([]), 100),
            "()[100]"
        );
        assert_eq_sig!(
            AbiType::fixedarray(
                AbiType::unnamed_tuple([AbiType::Address, AbiType::Bool]),
                1000
            ),
            "(address,bool)[1000]"
        );

        assert_eq_sig!(
            AbiType::map(PlainAbiType::Uint(123), AbiType::Address),
            "map(uint123,address)"
        );
        assert_eq_sig!(
            AbiType::map(PlainAbiType::Uint(123), AbiType::unnamed_tuple([])),
            "map(uint123,())"
        );
        assert_eq_sig!(
            AbiType::map(
                PlainAbiType::Uint(123),
                AbiType::unnamed_tuple([AbiType::Address, AbiType::Bool])
            ),
            "map(uint123,(address,bool))"
        );
        assert_eq_sig!(
            AbiType::map(
                PlainAbiType::Uint(123),
                AbiType::fixedarray(AbiType::Address, 123)
            ),
            "map(uint123,address[123])"
        );

        assert_eq_sig!(AbiType::optional(AbiType::Address), "optional(address)");
        assert_eq_sig!(
            AbiType::optional(AbiType::unnamed_tuple([])),
            "optional(())"
        );
        assert_eq_sig!(
            AbiType::optional(AbiType::unnamed_tuple([AbiType::Address, AbiType::Bool])),
            "optional((address,bool))"
        );
        assert_eq_sig!(
            AbiType::optional(AbiType::fixedarray(AbiType::Address, 123)),
            "optional(address[123])"
        );

        assert_eq_sig!(AbiType::reference(AbiType::Address), "ref(address)");
        assert_eq_sig!(AbiType::reference(AbiType::unnamed_tuple([])), "ref(())");
        assert_eq_sig!(
            AbiType::reference(AbiType::unnamed_tuple([AbiType::Address, AbiType::Bool])),
            "ref((address,bool))"
        );
        assert_eq_sig!(
            AbiType::reference(AbiType::fixedarray(AbiType::Address, 123)),
            "ref(address[123])"
        );

        assert_eq_sig!(
            AbiType::array(AbiType::unnamed_tuple([
                AbiType::Bool,
                AbiType::Uint(123),
                AbiType::array(AbiType::map(
                    PlainAbiType::Address,
                    AbiType::unnamed_tuple([AbiType::Uint(32), AbiType::String]),
                )),
            ])),
            "(bool,uint123,map(address,(uint32,string))[])[]"
        );
    }

    #[test]
    fn correct_simple_signature() {
        macro_rules! assert_eq_sig {
            ($expr:expr, $signature:literal) => {
                assert_eq!(format!("{}", ($expr).display_simple()), $signature)
            };
        }

        assert_eq_sig!(AbiType::Uint(100), "uint100");
        assert_eq_sig!(AbiType::Int(100), "int100");
        assert_eq_sig!(AbiType::varuint(16), "varuint16");
        assert_eq_sig!(AbiType::varint(16), "varint16");
        assert_eq_sig!(AbiType::Bool, "bool");
        assert_eq_sig!(AbiType::Cell, "cell");
        assert_eq_sig!(AbiType::Address, "address");
        assert_eq_sig!(AbiType::Bytes, "bytes");
        assert_eq_sig!(AbiType::FixedBytes(123), "fixedbytes123");
        assert_eq_sig!(AbiType::String, "string");
        assert_eq_sig!(AbiType::Token, "gram");

        assert_eq_sig!(AbiType::unnamed_tuple([]), "tuple");
        assert_eq_sig!(AbiType::unnamed_tuple([AbiType::Uint(321)]), "tuple");
        assert_eq_sig!(
            AbiType::unnamed_tuple([AbiType::Uint(123), AbiType::Address]),
            "tuple"
        );

        assert_eq_sig!(AbiType::array(AbiType::Address), "address[]");
        assert_eq_sig!(
            AbiType::array(AbiType::array(AbiType::Address)),
            "address[][]"
        );
        assert_eq_sig!(AbiType::array(AbiType::unnamed_tuple([])), "tuple[]");
        assert_eq_sig!(
            AbiType::array(AbiType::unnamed_tuple([AbiType::Address, AbiType::Bool])),
            "tuple[]"
        );

        assert_eq_sig!(AbiType::fixedarray(AbiType::Address, 10), "address[10]");
        assert_eq_sig!(
            AbiType::fixedarray(AbiType::fixedarray(AbiType::Address, 123), 321),
            "address[123][321]"
        );
        assert_eq_sig!(
            AbiType::fixedarray(AbiType::unnamed_tuple([]), 100),
            "tuple[100]"
        );
        assert_eq_sig!(
            AbiType::fixedarray(
                AbiType::unnamed_tuple([AbiType::Address, AbiType::Bool]),
                1000
            ),
            "tuple[1000]"
        );

        assert_eq_sig!(
            AbiType::map(PlainAbiType::Uint(123), AbiType::Address),
            "map(uint123,address)"
        );
        assert_eq_sig!(
            AbiType::map(PlainAbiType::Uint(123), AbiType::unnamed_tuple([])),
            "map(uint123,tuple)"
        );
        assert_eq_sig!(
            AbiType::map(
                PlainAbiType::Uint(123),
                AbiType::unnamed_tuple([AbiType::Address, AbiType::Bool])
            ),
            "map(uint123,tuple)"
        );
        assert_eq_sig!(
            AbiType::map(
                PlainAbiType::Uint(123),
                AbiType::fixedarray(AbiType::Address, 123)
            ),
            "map(uint123,address[123])"
        );

        assert_eq_sig!(AbiType::optional(AbiType::Address), "optional(address)");
        assert_eq_sig!(
            AbiType::optional(AbiType::unnamed_tuple([])),
            "optional(tuple)"
        );
        assert_eq_sig!(
            AbiType::optional(AbiType::unnamed_tuple([AbiType::Address, AbiType::Bool])),
            "optional(tuple)"
        );
        assert_eq_sig!(
            AbiType::optional(AbiType::fixedarray(AbiType::Address, 123)),
            "optional(address[123])"
        );

        assert_eq_sig!(AbiType::reference(AbiType::Address), "ref(address)");
        assert_eq_sig!(AbiType::reference(AbiType::unnamed_tuple([])), "ref(tuple)");
        assert_eq_sig!(
            AbiType::reference(AbiType::unnamed_tuple([AbiType::Address, AbiType::Bool])),
            "ref(tuple)"
        );
        assert_eq_sig!(
            AbiType::reference(AbiType::fixedarray(AbiType::Address, 123)),
            "ref(address[123])"
        );

        assert_eq_sig!(
            AbiType::array(AbiType::unnamed_tuple([
                AbiType::Bool,
                AbiType::Uint(123),
                AbiType::array(AbiType::map(
                    PlainAbiType::Address,
                    AbiType::unnamed_tuple([AbiType::Uint(32), AbiType::String]),
                )),
            ])),
            "tuple[]"
        );
    }

    #[test]
    fn from_to_json() {
        const RAW: &str = r#"{
            "name":"info",
            "type":"tuple",
            "components": [
                {"name":"total","type":"uint64"},
                {"name":"withdrawValue","type":"uint64"},
                {"name":"reinvest","type":"bool"},
                {"name":"reward","type":"uint64"},
                {"name":"stakes","type":"map(uint64,uint64)"},
                {"components":[{"name":"remainingAmount","type":"uint64"},{"name":"lastWithdrawalTime","type":"uint64"},{"name":"withdrawalPeriod","type":"uint32"},{"name":"withdrawalValue","type":"uint64"},{"name":"owner","type":"address"}],"name":"vestings","type":"map(uint64,tuple)"},
                {"components":[{"name":"remainingAmount","type":"uint64"},{"name":"lastWithdrawalTime","type":"uint64"},{"name":"withdrawalPeriod","type":"uint32"},{"name":"withdrawalValue","type":"uint64"},{"name":"owner","type":"address"}],"name":"locks","type":"map(uint64,tuple)"},
                {"name":"vestingDonor","type":"address"},
                {"name":"lockDonor","type":"address"}
            ]
        }"#;

        let ty = serde_json::from_str::<NamedAbiType>(RAW).unwrap();

        let complex_item_ty = AbiType::tuple([
            ("remainingAmount", u64::abi_type()),
            ("lastWithdrawalTime", u64::abi_type()),
            ("withdrawalPeriod", u32::abi_type()),
            ("withdrawalValue", u64::abi_type()),
            ("owner", IntAddr::abi_type()),
        ]);

        assert_eq!(
            ty,
            NamedAbiType::new(
                "info",
                AbiType::Tuple(Arc::from(vec![
                    NamedAbiType::new("total", u64::abi_type()),
                    NamedAbiType::new("withdrawValue", u64::abi_type()),
                    NamedAbiType::new("reinvest", bool::abi_type()),
                    NamedAbiType::new("reward", u64::abi_type()),
                    NamedAbiType::new("stakes", BTreeMap::<u64, u64>::abi_type()),
                    NamedAbiType::new(
                        "vestings",
                        AbiType::map(u64::plain_abi_type(), complex_item_ty.clone())
                    ),
                    NamedAbiType::new(
                        "locks",
                        AbiType::map(u64::plain_abi_type(), complex_item_ty)
                    ),
                    NamedAbiType::new("vestingDonor", IntAddr::abi_type()),
                    NamedAbiType::new("lockDonor", IntAddr::abi_type()),
                ]))
            )
        );

        let normalized = serde_json::from_str::<serde_json::Value>(RAW).unwrap();
        let serialized = serde_json::to_value(ty).unwrap();
        assert_eq!(serialized, normalized);
    }
}
