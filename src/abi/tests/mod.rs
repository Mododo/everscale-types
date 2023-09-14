use bytes::Bytes;

use crate::abi::*;
use crate::models::StdAddr;
use crate::prelude::{Cell, CellBuilder, DefaultFinalizer, HashBytes, RawDict, Store};

const DEPOOL_ABI: &str = include_str!("depool.abi.json");

#[test]
fn decode_json_abi() {
    let contract = serde_json::from_str::<Contract>(DEPOOL_ABI).unwrap();
    assert_eq!(contract.abi_version, AbiVersion::V2_0);
    assert_eq!(contract.functions.len(), 28);
    assert_eq!(contract.events.len(), 10);
    assert_eq!(contract.fields.len(), 0);
    assert_eq!(contract.init_data.len(), 0);

    let function = contract.find_function_by_id(0x4e73744b, true).unwrap();
    assert_eq!(function.input_id, 0x4e73744b);
    assert_eq!(function.name.as_ref(), "participateInElections");
}

#[test]
fn encode_internal_input() {
    let contract = serde_json::from_str::<Contract>(DEPOOL_ABI).unwrap();
    let function = contract.find_function_by_id(0x4e73744b, true).unwrap();

    let expected = {
        let mut builder = CellBuilder::new();
        builder.store_u32(function.input_id).unwrap();
        builder.store_u64(123).unwrap();
        builder.store_zeros(256).unwrap();
        builder.store_u32(321).unwrap();
        builder.store_u32(16123).unwrap();
        builder.store_zeros(256).unwrap();
        builder
            .store_reference({
                let builder = CellBuilder::from_raw_data(&[0; 64], 512).unwrap();
                builder.build().unwrap()
            })
            .unwrap();
        builder.build().unwrap()
    };

    let body = function
        .encode_internal_input(&[
            123u64.into_abi().named("queryId"),
            HashBytes::default().into_abi().named("validatorKey"),
            321u32.into_abi().named("stakeAt"),
            16123u32.into_abi().named("maxFactor"),
            HashBytes::default().into_abi().named("adnlAddr"),
            Bytes::from(vec![0; 64]).into_abi().named("signature"),
        ])
        .unwrap()
        .build()
        .unwrap();

    assert_eq!(body, expected);
}

#[test]
fn decode_internal_input() {
    let contract = serde_json::from_str::<Contract>(DEPOOL_ABI).unwrap();
    let function = contract.find_function_by_id(0x4e73744b, true).unwrap();

    let body = {
        let mut builder = CellBuilder::new();
        builder.store_u32(function.input_id).unwrap();
        builder.store_u64(123).unwrap();
        builder.store_zeros(256).unwrap();
        builder.store_u32(321).unwrap();
        builder.store_u32(16123).unwrap();
        builder.store_zeros(256).unwrap();
        builder
            .store_reference({
                let builder = CellBuilder::from_raw_data(&[0; 64], 512).unwrap();
                builder.build().unwrap()
            })
            .unwrap();
        builder.build().unwrap()
    };

    let tokens = function
        .decode_internal_input(body.as_slice().unwrap())
        .unwrap();

    NamedAbiValue::check_types(&tokens, &function.inputs).unwrap();
}

#[test]
fn encode_external_input() {
    let contract = serde_json::from_str::<Contract>(DEPOOL_ABI).unwrap();
    let function = contract.functions.get("constructor").unwrap();

    let expected = {
        let mut builder = CellBuilder::new();
        builder.store_bit_one().unwrap();
        builder.store_zeros(512).unwrap();
        builder.store_u64(10000).unwrap();
        builder.store_u32(10).unwrap();
        builder.store_u32(function.input_id).unwrap();
        builder.store_u64(123).unwrap();
        builder.store_u64(321).unwrap();
        builder.store_reference(Cell::default()).unwrap();
        builder
            .store_reference({
                let mut builder = CellBuilder::new();
                StdAddr::default()
                    .store_into(&mut builder, &mut Cell::default_finalizer())
                    .unwrap();
                builder.store_u8(1).unwrap();
                builder.build().unwrap()
            })
            .unwrap();
        builder.build().unwrap()
    };

    let body = function
        .encode_external(&[
            123u64.into_abi().named("minStake"),
            321u64.into_abi().named("validatorAssurance"),
            Cell::default().into_abi().named("proxyCode"),
            StdAddr::default().into_abi().named("validatorWallet"),
            1u8.into_abi().named("participantRewardFraction"),
        ])
        .with_time(10000)
        .with_expire_at(10)
        .build_input()
        .unwrap()
        .with_fake_signature()
        .unwrap();

    assert_eq!(body, expected);
}

#[test]
fn decode_external_input() {
    let contract = serde_json::from_str::<Contract>(DEPOOL_ABI).unwrap();
    let function = contract.functions.get("constructor").unwrap();

    let body = {
        let mut builder = CellBuilder::new();
        builder.store_bit_one().unwrap();
        builder.store_zeros(512).unwrap();
        builder.store_u64(10000).unwrap();
        builder.store_u32(10).unwrap();
        builder.store_u32(function.input_id).unwrap();
        builder.store_u64(123).unwrap();
        builder.store_u64(321).unwrap();
        builder.store_reference(Cell::default()).unwrap();
        builder
            .store_reference({
                let mut builder = CellBuilder::new();
                StdAddr::default()
                    .store_into(&mut builder, &mut Cell::default_finalizer())
                    .unwrap();
                builder.store_u8(1).unwrap();
                builder.build().unwrap()
            })
            .unwrap();
        builder.build().unwrap()
    };

    let tokens = function
        .decode_external_input(body.as_slice().unwrap())
        .unwrap();

    NamedAbiValue::check_types(&tokens, &function.inputs).unwrap();
}

#[test]
fn encode_unsigned_external_input() {
    let contract = serde_json::from_str::<Contract>(DEPOOL_ABI).unwrap();
    let function = contract.functions.get("constructor").unwrap();

    let expected = {
        let mut builder = CellBuilder::new();
        builder.store_bit_zero().unwrap();
        builder.store_u64(10000).unwrap();
        builder.store_u32(10).unwrap();
        builder.store_u32(function.input_id).unwrap();
        builder.store_u64(123).unwrap();
        builder.store_u64(321).unwrap();
        builder.store_reference(Cell::default()).unwrap();
        StdAddr::default()
            .store_into(&mut builder, &mut Cell::default_finalizer())
            .unwrap();
        builder.store_u8(1).unwrap();
        builder.build().unwrap()
    };

    let (_, body) = function
        .encode_external(&[
            123u64.into_abi().named("minStake"),
            321u64.into_abi().named("validatorAssurance"),
            Cell::default().into_abi().named("proxyCode"),
            StdAddr::default().into_abi().named("validatorWallet"),
            1u8.into_abi().named("participantRewardFraction"),
        ])
        .with_time(10000)
        .with_expire_at(10)
        .build_input_without_signature()
        .unwrap();

    println!("{}", expected.display_tree());
    println!("{}", body.display_tree());

    assert_eq!(body, expected);
}

#[test]
fn decode_unsigned_external_input() {
    let contract = serde_json::from_str::<Contract>(DEPOOL_ABI).unwrap();
    let function = contract.functions.get("constructor").unwrap();

    let body = {
        let mut builder = CellBuilder::new();
        builder.store_bit_zero().unwrap();
        builder.store_u64(10000).unwrap();
        builder.store_u32(10).unwrap();
        builder.store_u32(function.input_id).unwrap();
        builder.store_u64(123).unwrap();
        builder.store_u64(321).unwrap();
        builder.store_reference(Cell::default()).unwrap();
        StdAddr::default()
            .store_into(&mut builder, &mut Cell::default_finalizer())
            .unwrap();
        builder.store_u8(1).unwrap();
        builder.build().unwrap()
    };

    let tokens = function
        .decode_external_input(body.as_slice().unwrap())
        .unwrap();

    NamedAbiValue::check_types(&tokens, &function.inputs).unwrap();
}

#[test]
fn encode_empty_init_data() {
    let contract = serde_json::from_str::<Contract>(DEPOOL_ABI).unwrap();

    let key = ed25519_dalek::SigningKey::from([0u8; 32]);
    let pubkey = ed25519_dalek::VerifyingKey::from(&key);

    let expected = {
        let mut dict = RawDict::<64>::new();

        let mut key = CellBuilder::new();
        key.store_u64(0).unwrap();

        let value = CellBuilder::from_raw_data(pubkey.as_bytes(), 256).unwrap();
        dict.set(key.as_data_slice(), value.as_data_slice())
            .unwrap();

        CellBuilder::build_from(dict).unwrap()
    };

    let init_data = contract.encode_init_data(&pubkey, &[]).unwrap();

    assert_eq!(init_data, expected);
}
