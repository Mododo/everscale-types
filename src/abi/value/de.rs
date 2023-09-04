use std::collections::BTreeMap;
use std::num::NonZeroU8;

use anyhow::Result;
use bytes::Bytes;
use num_bigint::{BigInt, BigUint};

use crate::abi::error::AbiError;
use crate::abi::{
    AbiType, AbiValue, AbiVersion, NamedAbiType, NamedAbiValue, PlainAbiType, PlainAbiValue,
};
use crate::cell::{Cell, CellSlice, Load, MAX_BIT_LEN, MAX_REF_COUNT};
use crate::dict::{self, RawDict};
use crate::error::Error;
use crate::models::IntAddr;
use crate::num::Tokens;

impl NamedAbiValue {
    /// Loads exactly one tuple from the specified slice requiring it to be fully consumed.
    ///
    /// Use [`NamedAbiValue::load_tuple_partial`] if you allow an ABI type to be a prefix.
    pub fn load_tuple(
        items: &[NamedAbiType],
        version: AbiVersion,
        slice: &mut CellSlice,
    ) -> Result<Vec<Self>> {
        let result = ok!(Self::load_tuple_ext(items, version, true, false, slice));
        ok!(ensure_slice_empty(slice, false));
        Ok(result)
    }

    /// Loads a tuple from the specified slice.
    pub fn load_tuple_partial(
        items: &[NamedAbiType],
        version: AbiVersion,
        slice: &mut CellSlice,
    ) -> Result<Vec<Self>> {
        Self::load_tuple_ext(items, version, true, true, slice)
    }

    /// Loads a tuple from the specified slice with explicit decoder params.
    ///
    /// NOTE: this method does not check the remaining bits and refs in the root slice.
    pub fn load_tuple_ext(
        items: &[NamedAbiType],
        version: AbiVersion,
        last: bool,
        allow_partial: bool,
        slice: &mut CellSlice,
    ) -> Result<Vec<Self>> {
        let mut result = Vec::with_capacity(items.len());
        let items_len = items.len();
        for (i, item) in items.iter().enumerate() {
            let last = last && i + 1 == items_len;
            result.push(ok!(Self::load_ext(
                item,
                version,
                last,
                allow_partial,
                slice
            )));
        }
        Ok(result)
    }

    /// Loads exactly one ABI value from the specified slice requiring it to be fully consumed.
    ///
    /// Use [`NamedAbiValue::load_partial`] if you allow an ABI type to be a prefix.
    pub fn load(ty: &NamedAbiType, version: AbiVersion, slice: &mut CellSlice) -> Result<Self> {
        let res = ok!(Self::load_ext(ty, version, true, false, slice));
        ok!(ensure_slice_empty(slice, false));
        Ok(res)
    }

    /// Loads an ABI value from the specified slice.
    pub fn load_partial(
        ty: &NamedAbiType,
        version: AbiVersion,
        slice: &mut CellSlice,
    ) -> Result<Self> {
        Self::load_ext(ty, version, true, true, slice)
    }

    /// Loads an ABI value from the specified slice with explicit decoder params.
    ///
    /// NOTE: this method does not check the remaining bits and refs in the root slice.
    pub fn load_ext(
        ty: &NamedAbiType,
        version: AbiVersion,
        last: bool,
        allow_partial: bool,
        slice: &mut CellSlice,
    ) -> Result<Self> {
        Ok(Self {
            value: ok!(AbiValue::load_ext(
                &ty.ty,
                version,
                last,
                allow_partial,
                slice
            )),
            name: ty.name.clone(),
        })
    }
}

impl AbiValue {
    /// Loads exactly one unnamed tuple from the specified slice requiring it to be fully consumed.
    ///
    /// Use [`AbiValue::load_tuple_partial`] if you allow an ABI type to be a prefix.
    pub fn load_tuple(
        types: &[AbiType],
        version: AbiVersion,
        slice: &mut CellSlice,
    ) -> Result<Vec<Self>> {
        let res = ok!(Self::load_tuple_ext(types, version, true, false, slice));
        ok!(ensure_slice_empty(slice, false));
        Ok(res)
    }

    /// Loads an unnamed tuple from the specified slice.
    pub fn load_tuple_partial(
        types: &[AbiType],
        version: AbiVersion,
        slice: &mut CellSlice,
    ) -> Result<Vec<Self>> {
        Self::load_tuple_ext(types, version, true, true, slice)
    }

    /// Loads an unnamed tuple from the specified slice with explicit decoder params.
    ///
    /// NOTE: this method does not check the remaining bits and refs in the root slice.
    pub fn load_tuple_ext(
        types: &[AbiType],
        version: AbiVersion,
        last: bool,
        allow_partial: bool,
        slice: &mut CellSlice,
    ) -> Result<Vec<Self>> {
        let mut result = Vec::with_capacity(types.len());
        let types_len = types.len();
        for (i, ty) in types.iter().enumerate() {
            let last = last && i + 1 == types_len;
            result.push(ok!(Self::load_ext(ty, version, last, allow_partial, slice)));
        }
        Ok(result)
    }

    /// Loads exactly one ABI value from the specified slice requiring it to be fully consumed.
    ///
    /// Use [`AbiValue::load_partial`] if you allow an ABI type to be a prefix.
    pub fn load(ty: &AbiType, version: AbiVersion, slice: &mut CellSlice) -> Result<Self> {
        let res = ok!(Self::load_ext(ty, version, true, false, slice));
        ok!(ensure_slice_empty(slice, false));
        Ok(res)
    }

    /// Loads an ABI value from the specified slice.
    pub fn load_partial(ty: &AbiType, version: AbiVersion, slice: &mut CellSlice) -> Result<Self> {
        Self::load_ext(ty, version, true, true, slice)
    }

    /// Loads an ABI value from the specified slice with explicit decoder params.
    ///
    /// NOTE: this method does not check the remaining bits and refs in the root slice.
    pub fn load_ext(
        ty: &AbiType,
        version: AbiVersion,
        last: bool,
        allow_partial: bool,
        slice: &mut CellSlice,
    ) -> Result<Self> {
        match ty {
            AbiType::Uint(bits) => load_uint(*bits, slice).map(|value| Self::Uint(*bits, value)),
            AbiType::Int(bits) => load_int(*bits, slice).map(|value| Self::Int(*bits, value)),
            AbiType::VarUint(size) => {
                load_varuint(*size, slice).map(|value| Self::VarUint(*size, value))
            }
            AbiType::VarInt(size) => {
                load_varint(*size, slice).map(|value| Self::VarInt(*size, value))
            }
            AbiType::Bool => {
                ok!(preload_bits(1, slice));
                Ok(Self::Bool(slice.load_bit()?))
            }
            AbiType::Cell => load_cell(version, last, slice).map(Self::Cell),
            AbiType::Address => {
                ok!(preload_bits(1, slice));
                Ok(Self::Address(IntAddr::load_from(slice).map(Box::new)?))
            }
            AbiType::Bytes => load_bytes(version, last, slice).map(Self::Bytes),
            AbiType::FixedBytes(len) => {
                load_fixed_bytes(*len, version, last, slice).map(Self::FixedBytes)
            }
            AbiType::String => load_string(version, last, slice).map(Self::String),
            AbiType::Token => {
                ok!(preload_bits(1, slice));
                Ok(Self::Token(Tokens::load_from(slice)?))
            }
            AbiType::Tuple(items) => {
                let values = ok!(NamedAbiValue::load_tuple_ext(
                    items,
                    version,
                    last,
                    allow_partial,
                    slice
                ));
                Ok(Self::Tuple(values))
            }
            AbiType::Array(ty) => load_array(ty, version, allow_partial, slice)
                .map(|values| Self::Array(ty.clone(), values)),
            AbiType::FixedArray(ty, len) => {
                load_fixed_array(ty, *len, version, allow_partial, slice)
                    .map(|values| Self::FixedArray(ty.clone(), values))
            }
            AbiType::Map(key_ty, value_ty) => {
                load_map(*key_ty, value_ty, version, allow_partial, slice)
                    .map(|value| Self::Map(*key_ty, value_ty.clone(), value))
            }
            AbiType::Optional(ty) => load_optional(ty, version, last, allow_partial, slice)
                .map(|value| Self::Optional(ty.clone(), value)),
            AbiType::Ref(ty) => load_ref(ty, version, last, allow_partial, slice).map(Self::Ref),
        }
    }
}

impl PlainAbiValue {
    /// Loads a corresponding value from the slice.
    pub fn load(ty: PlainAbiType, slice: &mut CellSlice) -> Result<Self, Error> {
        match ty {
            PlainAbiType::Uint(bits) => {
                load_uint_plain(bits, slice).map(|value| Self::Uint(bits, value))
            }
            PlainAbiType::Int(bits) => {
                load_int_plain(bits, slice).map(|value| Self::Int(bits, value))
            }
            PlainAbiType::Bool => slice.load_bit().map(Self::Bool),
            PlainAbiType::Address => IntAddr::load_from(slice).map(Box::new).map(Self::Address),
        }
    }
}

fn preload_bits(bits: u16, slice: &mut CellSlice) -> Result<()> {
    if bits == 0 {
        return Ok(());
    }

    let remaining_bits = slice.remaining_bits();
    if remaining_bits == 0 {
        let first_ref = slice.load_reference_as_slice()?;

        // TODO: why `allow_partial` is not used here in a reference impl?
        anyhow::ensure!(slice.is_refs_empty(), AbiError::IncompleteDeserialization);

        *slice = first_ref;
    } else if remaining_bits < bits {
        anyhow::bail!(Error::CellUnderflow);
    }

    Ok(())
}

fn ensure_slice_empty(slice: &mut CellSlice, allow_partial: bool) -> Result<()> {
    anyhow::ensure!(
        allow_partial || slice.is_data_empty() && slice.is_refs_empty(),
        AbiError::IncompleteDeserialization
    );
    Ok(())
}

fn load_uint(bits: u16, slice: &mut CellSlice) -> Result<BigUint> {
    ok!(preload_bits(bits, slice));
    load_uint_plain(bits, slice).map_err(From::from)
}

fn load_int(bits: u16, slice: &mut CellSlice) -> Result<BigInt> {
    ok!(preload_bits(bits, slice));
    load_int_plain(bits, slice).map_err(From::from)
}

fn load_uint_plain(bits: u16, slice: &mut CellSlice) -> Result<BigUint, Error> {
    match bits {
        0 => Ok(BigUint::default()),
        1..=64 => slice.load_uint(bits).map(BigUint::from),
        _ => {
            let rem = bits % 8;
            let mut buffer = [0u8; 33];
            slice.load_raw(&mut buffer, bits).map(|buffer| {
                let mut int = BigUint::from_bytes_be(buffer);
                if rem != 0 {
                    int >>= 8 - rem;
                }
                int
            })
        }
    }
}

fn load_int_plain(bits: u16, slice: &mut CellSlice) -> Result<BigInt, Error> {
    match bits {
        0 => Ok(BigInt::default()),
        1..=64 => slice.load_uint(bits).map(|mut int| {
            if bits < 64 {
                // Clone sign bit into all high bits
                int |= ((int >> (bits - 1)) * u64::MAX) << (bits - 1);
            }
            BigInt::from(int as i64)
        }),
        _ => {
            let rem = bits % 8;
            let mut buffer = [0u8; 33];
            slice.load_raw(&mut buffer, bits).map(|buffer| {
                let mut int = BigInt::from_signed_bytes_be(buffer);
                if rem != 0 {
                    int >>= 8 - rem;
                }
                int
            })
        }
    }
}

fn load_varuint(size: NonZeroU8, slice: &mut CellSlice) -> Result<BigUint> {
    let bytes = ok!(load_varuint_raw(size, slice));
    Ok(BigUint::from_bytes_le(&bytes))
}

fn load_varint(size: NonZeroU8, slice: &mut CellSlice) -> Result<BigInt> {
    // TODO: manually use `twos_complement_le` to prevent useless cloning in `from_signed_bytes_le`
    let bytes = ok!(load_varuint_raw(size, slice));
    Ok(BigInt::from_signed_bytes_le(&bytes))
}

/// Loads a varuint as bytes in LE (!) order.
fn load_varuint_raw(size: NonZeroU8, slice: &mut CellSlice) -> Result<Vec<u8>> {
    let value_size = size.get() - 1;

    let len_bits = (8 - value_size.leading_zeros()) as u16;
    ok!(preload_bits(len_bits, slice));

    let value_bytes = slice.load_small_uint(len_bits)? as usize;
    let value_bits = (value_bytes * 8) as u16;
    ok!(preload_bits(value_bits, slice));

    let mut bytes = vec![0u8; value_bytes];
    slice.load_raw(&mut bytes, value_bits)?;

    // NOTE: reverse and use `from_bytes_le` to prevent useless cloning in `from_bytes_be`
    bytes.reverse();
    Ok(bytes)
}

fn load_cell(version: AbiVersion, last: bool, slice: &mut CellSlice) -> Result<Cell> {
    if slice.remaining_refs() == 1
        && (version.major == 1 && slice.cell().reference_count() as usize == MAX_REF_COUNT
            || version.major > 1 && !last && slice.remaining_bits() == 0)
    {
        *slice = slice.load_reference_as_slice()?;
    }
    slice.load_reference_cloned().map_err(From::from)
}

fn load_bytes_raw(version: AbiVersion, last: bool, slice: &mut CellSlice) -> Result<Vec<u8>> {
    let cell = ok!(load_cell(version, last, slice));
    let mut cell = cell.as_ref();

    let mut bytes = Vec::new();
    loop {
        anyhow::ensure!(cell.bit_len() % 8 == 0, AbiError::ExpectedCellWithBytes);
        bytes.extend_from_slice(cell.data());

        match cell.reference(0) {
            Some(child) => cell = child,
            None => break Ok(bytes),
        };
    }
}

fn load_bytes(version: AbiVersion, last: bool, slice: &mut CellSlice) -> Result<Bytes> {
    load_bytes_raw(version, last, slice).map(Bytes::from)
}

fn load_fixed_bytes(
    len: usize,
    version: AbiVersion,
    last: bool,
    slice: &mut CellSlice,
) -> Result<Bytes> {
    let bytes = ok!(load_bytes(version, last, slice));
    anyhow::ensure!(
        bytes.len() == len,
        AbiError::BytesSizeMismatch {
            expected: len,
            len: bytes.len()
        }
    );
    Ok(bytes)
}

fn load_string(version: AbiVersion, last: bool, slice: &mut CellSlice) -> Result<String> {
    let bytes = ok!(load_bytes_raw(version, last, slice));
    match String::from_utf8(bytes) {
        Ok(str) => Ok(str),
        Err(e) => Err(AbiError::InvalidString(e.utf8_error()).into()),
    }
}

fn load_array_raw(
    ty: &AbiType,
    len: usize,
    version: AbiVersion,
    allow_partial: bool,
    slice: &mut CellSlice,
) -> Result<Vec<AbiValue>> {
    ok!(preload_bits(1, slice));
    let dict = RawDict::<32>::load_from(slice)?;

    let mut result = Vec::with_capacity(len);
    for value in dict.values().take(len) {
        let slice = &mut value?;
        let value = ok!(AbiValue::load_ext(ty, version, true, allow_partial, slice));
        ok!(ensure_slice_empty(slice, allow_partial));
        result.push(value);
    }

    Ok(result)
}

fn load_array(
    ty: &AbiType,
    version: AbiVersion,
    allow_partial: bool,
    slice: &mut CellSlice,
) -> Result<Vec<AbiValue>> {
    ok!(preload_bits(32, slice));
    let len = slice.load_u32()?;
    load_array_raw(ty, len as usize, version, allow_partial, slice)
}

fn load_fixed_array(
    ty: &AbiType,
    len: usize,
    version: AbiVersion,
    allow_partial: bool,
    slice: &mut CellSlice,
) -> Result<Vec<AbiValue>> {
    let values = ok!(load_array_raw(ty, len, version, allow_partial, slice));
    anyhow::ensure!(
        values.len() == len,
        AbiError::ArraySizeMismatch {
            expected: len,
            len: values.len()
        }
    );
    Ok(values)
}

fn load_map(
    key_ty: PlainAbiType,
    value_ty: &AbiType,
    version: AbiVersion,
    allow_partial: bool,
    slice: &mut CellSlice,
) -> Result<BTreeMap<PlainAbiValue, AbiValue>> {
    ok!(preload_bits(1, slice));

    let key_bits = key_ty.max_bits();
    let dict = Option::<Cell>::load_from(slice)?;

    let mut result = BTreeMap::new();
    for entry in dict::RawIter::new(&dict, key_bits) {
        let (key, mut slice) = entry?;
        let key = PlainAbiValue::load(key_ty, &mut key.as_data_slice())?;
        let value = ok!(AbiValue::load_ext(
            value_ty,
            version,
            true,
            allow_partial,
            &mut slice
        ));
        result.insert(key, value);
    }

    Ok(result)
}

fn load_optional(
    ty: &AbiType,
    version: AbiVersion,
    last: bool,
    allow_partial: bool,
    slice: &mut CellSlice,
) -> Result<Option<Box<AbiValue>>> {
    ok!(preload_bits(1, slice));

    if !slice.load_bit()? {
        return Ok(None);
    }

    let (max_ty_bits, max_ty_refs) = ty.max_size();
    if max_ty_bits < MAX_BIT_LEN as usize && max_ty_refs < MAX_REF_COUNT {
        let cell = ok!(load_cell(version, last, slice));
        let slice = &mut cell.as_slice()?;
        let value = ok!(AbiValue::load_ext(ty, version, true, allow_partial, slice));
        ok!(ensure_slice_empty(slice, allow_partial));
        Ok(Some(Box::new(value)))
    } else {
        let value = ok!(AbiValue::load_ext(ty, version, last, allow_partial, slice));
        Ok(Some(Box::new(value)))
    }
}

fn load_ref(
    ty: &AbiType,
    version: AbiVersion,
    last: bool,
    allow_partial: bool,
    slice: &mut CellSlice,
) -> Result<Box<AbiValue>> {
    let cell = ok!(load_cell(version, last, slice));
    let slice = &mut cell.as_slice()?;
    let value = ok!(AbiValue::load_ext(ty, version, true, allow_partial, slice));
    ok!(ensure_slice_empty(slice, allow_partial));
    Ok(Box::new(value))
}

#[cfg(test)]
mod tests {
    use crate::boc::Boc;
    use crate::models::{StdAddr, VarAddr};
    use crate::num::{Uint9, VarUint24, VarUint56};
    use crate::prelude::{CellBuilder, CellFamily, Store};

    use super::*;

    trait BuildCell {
        fn build_cell(&self) -> Result<Cell>;
    }

    impl<T: Store> BuildCell for T {
        fn build_cell(&self) -> Result<Cell> {
            CellBuilder::build_from(self).map_err(From::from)
        }
    }

    impl BuildCell for &str {
        fn build_cell(&self) -> Result<Cell> {
            Boc::decode_base64(self).map_err(From::from)
        }
    }

    fn load_simple<T>(version: AbiVersion, boc: T, expected: AbiValue) -> Result<()>
    where
        T: BuildCell,
    {
        let cell = boc.build_cell()?;
        let ty = expected.get_type();
        assert_eq!(
            AbiValue::load(&ty, version, &mut cell.as_slice()?)?,
            expected
        );
        Ok(())
    }

    fn load_tuple<T>(version: AbiVersion, boc: T, expected: &[AbiValue]) -> Result<()>
    where
        T: BuildCell,
    {
        let cell = boc.build_cell()?;
        let ty = expected.iter().map(AbiValue::get_type).collect::<Vec<_>>();
        assert_eq!(
            AbiValue::load_tuple(&ty, version, &mut cell.as_slice()?)?,
            expected
        );
        Ok(())
    }

    macro_rules! assert_basic_err {
        ($expr:expr, $err:expr) => {{
            match $expr {
                Ok(_) => panic!("Expected basic error: {:?}, got success", $err),
                Err(e) => {
                    if let Some(e) = e.downcast_ref::<Error>() {
                        assert_eq!(e, &($err));
                    } else {
                        panic!("Unexpected error: {e:?}");
                    }
                }
            }
        }};
    }

    macro_rules! assert_abi_err {
        ($expr:expr, $err:expr) => {{
            match $expr {
                Ok(_) => panic!("Expected ABI error: {:?}, got success", $err),
                Err(e) => {
                    if let Some(e) = e.downcast_ref::<AbiError>() {
                        assert_eq!(e, &($err));
                    } else {
                        panic!("Unexpected error: {e:?}");
                    }
                }
            }
        }};
    }

    const VX_X: [AbiVersion; 5] = [
        AbiVersion::V1_0,
        AbiVersion::V2_0,
        AbiVersion::V2_1,
        AbiVersion::V2_2,
        AbiVersion::V2_3,
    ];
    const V2_X: [AbiVersion; 4] = [
        AbiVersion::V2_0,
        AbiVersion::V2_1,
        AbiVersion::V2_2,
        AbiVersion::V2_3,
    ];

    #[test]
    fn failed_decode() -> Result<()> {
        for v in VX_X {
            assert_basic_err!(
                load_simple(v, false, AbiValue::uint(32, 0u32)),
                Error::CellUnderflow
            );

            assert_abi_err!(
                load_simple(v, u64::MAX, AbiValue::uint(32, 0u32)),
                AbiError::IncompleteDeserialization
            );

            assert_abi_err!(
                load_tuple(v, u64::MAX, &[AbiValue::uint(32, u32::MAX)]),
                AbiError::IncompleteDeserialization
            );
        }

        Ok(())
    }

    #[test]
    fn decode_int() -> Result<()> {
        macro_rules! define_tests {
            ($v:ident, { $($abi:ident($bits:literal) => [$($expr:expr),*$(,)?]),*$(,)? }) => {$(
                $(load_simple($v, $expr, AbiValue::$abi($bits, $expr))?;)*
            )*};
        }

        for v in VX_X {
            define_tests!(v, {
                uint(8) => [0u8, 123u8, u8::MAX],
                uint(16) => [0u16, 1234u16, u16::MAX],
                uint(32) => [0u32, 123456u32, u32::MAX],
                uint(64) => [0u64, 123456789u64, u64::MAX],
                uint(128) => [0u128, 123456789123123123123u128, u128::MAX],

                int(8) => [0i8, 123i8, i8::MIN, i8::MAX],
                int(16) => [0i16, 1234i16, i16::MIN, i16::MAX],
                int(32) => [0i32, 123456i32, i32::MIN, i32::MAX],
                int(64) => [0i64, 123456789i64, i64::MIN, i64::MAX],
                int(128) => [0i128, 123456789123123123123i128, i128::MIN, i128::MAX],
            });
        }

        Ok(())
    }

    #[test]
    fn decode_varint() -> Result<()> {
        for v in VX_X {
            load_simple(v, VarUint24::ZERO, AbiValue::varuint(4, 0u32))?;
            load_simple(v, VarUint24::MAX, AbiValue::varuint(4, u32::MAX >> 8))?;
            load_simple(v, VarUint24::new(123321), AbiValue::varuint(4, 123321u32))?;

            load_simple(v, VarUint56::ZERO, AbiValue::varuint(8, 0u32))?;
            load_simple(v, VarUint56::MAX, AbiValue::varuint(8, u64::MAX >> 8))?;
            load_simple(
                v,
                VarUint56::new(1233213213123123),
                AbiValue::varuint(8, 1233213213123123u64),
            )?;

            load_simple(
                v,
                "te6ccgEBAQEABgAABzAeG5g=",
                AbiValue::varuint(16, 123321u32),
            )?;
            load_simple(v, "te6ccgEBAQEABgAABzAeG5g=", AbiValue::varint(16, 123321))?;

            load_simple(v, Tokens::ZERO, AbiValue::varuint(16, 0u32))?;
            load_simple(v, Tokens::ZERO, AbiValue::Token(Tokens::ZERO))?;

            let mut prev_value = 0;
            for byte in 0..15 {
                let value = (0xffu128 << (byte * 8)) | prev_value;
                prev_value = value;
                load_simple(v, Tokens::new(value), AbiValue::varuint(16, value))?;
                load_simple(v, Tokens::new(value), AbiValue::Token(Tokens::new(value)))?;
            }
        }

        Ok(())
    }

    #[test]
    fn decode_bool() -> Result<()> {
        for v in VX_X {
            load_simple(v, false, AbiValue::Bool(false))?;
            load_simple(v, true, AbiValue::Bool(true))?;
        }
        Ok(())
    }

    #[test]
    fn decode_cell() -> Result<()> {
        // ABI v1
        {
            load_simple(
                AbiVersion::V1_0,
                "te6ccgEBAgEABQABAAEAAA==", // one ref with empty cell
                AbiValue::Cell(Cell::empty_cell()),
            )?;

            // 4 refs with empty cells
            let cell = Boc::decode_base64("te6ccgEBAgEACAAEAAEBAQEAAA==")?;
            let slice = &mut cell.as_slice()?;
            slice.try_advance(0, 3);

            assert_basic_err!(
                AbiValue::load(&AbiType::Cell, AbiVersion::V1_0, slice),
                Error::CellUnderflow
            );

            // 3 refs with empty cells + last ref with a cell with 1 ref
            let cell = Boc::decode_base64("te6ccgEBAwEACwAEAAICAgEBAAIAAA==")?;
            let slice = &mut cell.as_slice()?;
            slice.try_advance(0, 3);

            assert_eq!(
                AbiValue::load(&AbiType::Cell, AbiVersion::V1_0, slice)?,
                AbiValue::Cell(Cell::empty_cell())
            );
        }

        for v in V2_X {
            load_simple(
                v,
                "te6ccgEBAgEABQABAAEAAA==", // one ref with empty cell
                AbiValue::Cell(Cell::empty_cell()),
            )?;

            // 4 refs with empty cells
            let cell = Boc::decode_base64("te6ccgEBAgEACAAEAAEBAQEAAA==")?;
            let slice = &mut cell.as_slice()?;
            slice.try_advance(0, 3);

            assert_eq!(
                AbiValue::load(&AbiType::Cell, v, slice)?,
                AbiValue::Cell(Cell::empty_cell())
            );
        }

        Ok(())
    }

    #[test]
    fn decode_address() -> Result<()> {
        for v in VX_X {
            let addr: StdAddr = StdAddr::from((0i8, [0xffu8; 32]));
            load_simple(
                v,
                addr.clone(),
                AbiValue::Address(Box::new(IntAddr::Std(addr))),
            )?;

            let addr: VarAddr = VarAddr {
                address_len: Uint9::new(10 * 8),
                anycast: None,
                workchain: 123456,
                address: vec![0xffu8; 10],
            };
            load_simple(
                v,
                addr.clone(),
                AbiValue::Address(Box::new(IntAddr::Var(addr))),
            )?;
        }

        Ok(())
    }

    #[test]
    fn decode_bytes() -> Result<()> {
        for v in VX_X {
            for len in 0..20 {
                let mut bytes = vec![0xffu8; 1usize << len];
                bytes[0] = 0x55; // mark start
                let bytes = Bytes::from(bytes);
                let serialized = AbiValue::Bytes(bytes.clone()).make_cell(v)?;

                load_simple(v, serialized.clone(), AbiValue::Bytes(bytes.clone()))?;
                load_simple(v, serialized.clone(), AbiValue::FixedBytes(bytes.clone()))?;

                let original = bytes.len();
                assert_abi_err!(
                    load_simple(
                        v,
                        serialized.clone(),
                        AbiValue::FixedBytes(bytes.slice(..original / 2))
                    ),
                    AbiError::BytesSizeMismatch {
                        expected: original / 2,
                        len: original
                    }
                )
            }
        }

        Ok(())
    }

    #[test]
    fn decode_string() -> Result<()> {
        for v in VX_X {
            for len in 0..20 {
                let mut bytes = vec![b'a'; 1usize << len];
                bytes[0] = b'f'; // mark start
                let string = String::from_utf8(bytes)?;

                let serialized = AbiValue::String(string.clone()).make_cell(v)?;
                load_simple(v, serialized.clone(), AbiValue::String(string))?;
            }
        }

        Ok(())
    }

    #[test]
    fn decode_nested_simple_tuple() -> Result<()> {
        let cell = {
            let mut builder = CellBuilder::new();
            builder.store_u32(0)?;
            builder.store_reference(Cell::empty_cell())?;
            builder.store_bit_zero()?;
            builder.store_u8(-15_i8 as _)?;
            builder.store_u16(-9845_i16 as _)?;
            builder.store_u32(-1_i32 as _)?;
            builder.store_u64(12345678_i64 as _)?;
            builder.store_u128(-12345678_i128 as _)?;
            builder.store_u8(255)?;
            builder.store_u16(0)?;
            builder.store_u32(256)?;
            builder.store_u64(123)?;
            builder.store_u128(1234567890)?;
            builder.build()?
        };

        let value = AbiValue::unnamed_tuple([
            AbiValue::uint(32, 0u32),
            AbiValue::Cell(Cell::empty_cell()),
            AbiValue::Bool(false),
            AbiValue::unnamed_tuple([
                AbiValue::int(8, -15),
                AbiValue::int(16, -9845),
                AbiValue::unnamed_tuple([
                    AbiValue::int(32, -1),
                    AbiValue::int(64, 12345678),
                    AbiValue::int(128, -12345678),
                ]),
            ]),
            AbiValue::unnamed_tuple([
                AbiValue::uint(8, 255_u8),
                AbiValue::uint(16, 0_u16),
                AbiValue::unnamed_tuple([
                    AbiValue::uint(32, 256_u32),
                    AbiValue::uint(64, 123_u64),
                    AbiValue::uint(128, 1234567890_u128),
                ]),
            ]),
        ]);

        for v in VX_X {
            load_simple(v, cell.clone(), value.clone())?;
        }

        Ok(())
    }

    #[test]
    fn decode_tuple_four_refs_and_four_uint256() -> Result<()> {
        let bytes = Bytes::from(vec![0xff; 32]);
        let mut bytes_builder = CellBuilder::new();
        bytes_builder.store_raw(&bytes, 256)?;

        let cell = {
            let mut builder = CellBuilder::new();
            builder.store_u32(0)?;
            builder.store_reference(Cell::empty_cell())?;

            builder.store_reference(bytes_builder.clone().build()?)?;
            builder.store_reference(bytes_builder.clone().build()?)?;

            let mut second_builder = CellBuilder::new();
            second_builder.store_reference(bytes_builder.clone().build()?)?;
            second_builder.store_builder(&bytes_builder)?;
            second_builder.store_builder(&bytes_builder)?;
            second_builder.store_builder(&bytes_builder)?;

            let mut third_builder = CellBuilder::new();
            third_builder.store_builder(&bytes_builder)?;

            second_builder.store_reference(third_builder.build()?)?;
            builder.store_reference(second_builder.build()?)?;

            builder.build()?
        };

        let value = AbiValue::unnamed_tuple([
            AbiValue::uint(32, 0_u32),
            AbiValue::Cell(Cell::empty_cell()),
            AbiValue::Cell(bytes_builder.clone().build()?),
            AbiValue::Bytes(bytes.clone()),
            AbiValue::Cell(bytes_builder.build()?),
            AbiValue::uint(256, BigUint::from_bytes_be(&bytes)),
            AbiValue::uint(256, BigUint::from_bytes_be(&bytes)),
            AbiValue::uint(256, BigUint::from_bytes_be(&bytes)),
            AbiValue::uint(256, BigUint::from_bytes_be(&bytes)),
        ]);

        for v in VX_X {
            load_simple(v, cell.clone(), value.clone())?;
        }

        Ok(())
    }

    #[test]
    fn decode_tuple_four_refs_and_one_uint256() -> Result<()> {
        let bytes = Bytes::from(vec![0xff; 32]);
        let mut bytes_builder = CellBuilder::new();
        bytes_builder.store_raw(&bytes, 256)?;

        let mut builder = CellBuilder::new();
        builder.store_u32(0)?;
        builder.store_reference(Cell::empty_cell())?;

        builder.store_reference(bytes_builder.clone().build()?)?;
        builder.store_reference(bytes_builder.clone().build()?)?;

        let cell_v2 = {
            let mut builder = builder.clone();
            builder.store_reference(bytes_builder.clone().build()?)?;
            builder.store_builder(&bytes_builder)?;
            builder.build()?
        };

        let cell_v1 = {
            let mut second_builder = CellBuilder::new();
            second_builder.store_reference(bytes_builder.clone().build()?)?;
            second_builder.store_builder(&bytes_builder)?;

            builder.store_reference(second_builder.build()?)?;
            builder.build()?
        };

        let value = AbiValue::unnamed_tuple([
            AbiValue::uint(32, 0_u32),
            AbiValue::Cell(Cell::empty_cell()),
            AbiValue::Cell(bytes_builder.clone().build()?),
            AbiValue::Bytes(bytes.clone()),
            AbiValue::Cell(bytes_builder.build()?),
            AbiValue::uint(256, BigUint::from_bytes_be(&bytes)),
        ]);

        load_simple(AbiVersion::V1_0, cell_v1, value.clone())?;
        for v in V2_X {
            load_simple(v, cell_v2.clone(), value.clone())?;
        }

        Ok(())
    }
}
