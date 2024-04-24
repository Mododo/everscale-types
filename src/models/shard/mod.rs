//! Shard state models.

use crate::cell::*;
use crate::dict::Dict;
use crate::error::*;

use crate::models::block::{BlockRef, ShardIdent};
use crate::models::currency::CurrencyCollection;
use crate::models::Lazy;

pub use self::shard_accounts::*;
pub use self::shard_extra::*;

#[cfg(feature = "venom")]
use super::ShardBlockRefs;

mod shard_accounts;
mod shard_extra;

#[cfg(test)]
mod tests;

/// Applied shard state.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ShardState {
    /// The next indivisible state in the shard.
    Unsplit(ShardStateUnsplit),
    /// Next indivisible states after shard split.
    Split(ShardStateSplit),
}

impl Store for ShardState {
    fn store_into(
        &self,
        builder: &mut CellBuilder,
        context: &mut dyn CellContext,
    ) -> Result<(), Error> {
        match self {
            Self::Unsplit(state) => state.store_into(builder, context),
            Self::Split(state) => state.store_into(builder, context),
        }
    }
}

impl<'a> Load<'a> for ShardState {
    fn load_from(slice: &mut CellSlice<'a>) -> Result<Self, Error> {
        Ok(if ok!(slice.get_bit(0)) {
            match ShardStateUnsplit::load_from(slice) {
                Ok(state) => Self::Unsplit(state),
                Err(e) => return Err(e),
            }
        } else {
            match ShardStateSplit::load_from(slice) {
                Ok(state) => Self::Split(state),
                Err(e) => return Err(e),
            }
        })
    }
}

/// State of single shard builder.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ShardStateUnsplitBuilder<T> {
    inner: ShardStateUnsplit,
    phantom_data: std::marker::PhantomData<T>,
}

impl ShardStateUnsplitBuilder<()> {
    /// Creates a new single shard state builder.
    pub fn new(shard_ident: ShardIdent, accounts: Lazy<ShardAccounts>) -> Self {
        Self {
            inner: ShardStateUnsplit {
                global_id: 0,
                shard_ident,
                seqno: 0,
                vert_seqno: 0,
                gen_utime: 0,
                #[cfg(feature = "venom")]
                gen_utime_ms: 0,
                gen_lt: 0,
                min_ref_mc_seqno: 0,
                out_msg_queue_info: Default::default(),
                #[cfg(feature = "tycho")]
                externals_processed_upto: Default::default(),
                before_split: false,
                accounts,
                overload_history: 0,
                underload_history: 0,
                total_balance: Default::default(),
                total_validator_fees: Default::default(),
                libraries: Default::default(),
                master_ref: None,
                custom: None,
                #[cfg(feature = "venom")]
                shard_block_refs: None,
            },
            phantom_data: std::marker::PhantomData,
        }
    }

    /// Builds the single shard state.
    pub fn build(self) -> ShardStateUnsplit {
        self.inner
    }
}

/// State of the single shard.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ShardStateUnsplit {
    /// Global network id.
    pub global_id: i32,
    /// Id of the shard.
    pub shard_ident: ShardIdent,
    /// Sequence number of the corresponding block.
    pub seqno: u32,
    /// Vertical sequcent number of the corresponding block.
    pub vert_seqno: u32,
    /// Unix timestamp when the block was created.
    pub gen_utime: u32,
    /// Milliseconds part of the timestamp when the block was created.
    #[cfg(feature = "venom")]
    pub gen_utime_ms: u16,
    /// Logical time when the state was created.
    pub gen_lt: u64,
    /// Minimal referenced seqno of the masterchain block.
    pub min_ref_mc_seqno: u32,
    /// Output messages queue info.
    pub out_msg_queue_info: Cell,
    #[cfg(feature = "tycho")]
    /// Index of the highest externals processed from the anchors: (anchor, index)
    pub externals_processed_upto: Dict<u32, u64>,
    /// Whether this state was produced before the shards split.
    pub before_split: bool,
    /// Reference to the dictionary with shard accounts.
    pub accounts: Lazy<ShardAccounts>,
    /// Mask for the overloaded blocks.
    pub overload_history: u64,
    /// Mask for the underloaded blocks.
    pub underload_history: u64,
    /// Total balance for all currencies.
    pub total_balance: CurrencyCollection,
    /// Total pending validator fees.
    pub total_validator_fees: CurrencyCollection,
    /// Dictionary with all libraries and its providers.
    pub libraries: Dict<HashBytes, LibDescr>,
    /// Optional reference to the masterchain block.
    pub master_ref: Option<BlockRef>,
    /// Shard state additional info.
    pub custom: Option<Lazy<McStateExtra>>,
    /// References to the latest known blocks from all shards.
    #[cfg(feature = "venom")]
    pub shard_block_refs: Option<ShardBlockRefs>,
}

impl ShardStateUnsplit {
    const TAG_V1: u32 = 0x9023afe2;
    #[cfg(feature = "venom")]
    const TAG_V2: u32 = 0x9023aeee;

    /// Tries to load shard accounts dictionary.
    pub fn load_accounts(&self) -> Result<ShardAccounts, Error> {
        self.accounts.load()
    }

    /// Tries to load additional masterchain data.
    pub fn load_custom(&self) -> Result<Option<McStateExtra>, Error> {
        match &self.custom {
            Some(custom) => match custom.load() {
                Ok(custom) => Ok(Some(custom)),
                Err(e) => Err(e),
            },
            None => Ok(None),
        }
    }

    #[cfg(feature = "tycho")]
    /// Tries to load OutMsgQueueInfo data.
    pub fn load_out_msg_queue_info(&self) -> Result<OutMsgQueueInfo, Error> {
        self.out_msg_queue_info.as_ref().parse()
    }
    #[cfg(feature = "tycho")]
    /// Tries to store OutMsgQueueInfo data to cell.
    pub fn set_out_msg_queue_info(&mut self, value: &OutMsgQueueInfo) -> Result<(), Error> {
        let cell = ok!(CellBuilder::build_from(value));
        self.out_msg_queue_info = cell;
        Ok(())
    }
}

impl Store for ShardStateUnsplit {
    fn store_into(
        &self,
        builder: &mut CellBuilder,
        context: &mut dyn CellContext,
    ) -> Result<(), Error> {
        let child_cell = {
            let mut builder = CellBuilder::new();
            ok!(builder.store_u64(self.overload_history));
            ok!(builder.store_u64(self.underload_history));
            ok!(self.total_balance.store_into(&mut builder, context));
            ok!(self.total_validator_fees.store_into(&mut builder, context));
            ok!(self.libraries.store_into(&mut builder, context));
            ok!(self.master_ref.store_into(&mut builder, context));
            #[cfg(feature = "tycho")]
            ok!(self
                .externals_processed_upto
                .store_into(&mut builder, context));
            ok!(builder.build_ext(context))
        };

        #[cfg(not(feature = "venom"))]
        ok!(builder.store_u32(Self::TAG_V1));
        #[cfg(feature = "venom")]
        ok!(builder.store_u32(Self::TAG_V2));

        ok!(builder.store_u32(self.global_id as u32));
        ok!(self.shard_ident.store_into(builder, context));
        ok!(builder.store_u32(self.seqno));
        ok!(builder.store_u32(self.vert_seqno));
        ok!(builder.store_u32(self.gen_utime));
        ok!(builder.store_u64(self.gen_lt));
        ok!(builder.store_u32(self.min_ref_mc_seqno));
        ok!(builder.store_reference(self.out_msg_queue_info.clone()));
        ok!(builder.store_bit(self.before_split));
        ok!(builder.store_reference(self.accounts.cell.clone()));
        ok!(builder.store_reference(child_cell));

        #[cfg(not(feature = "venom"))]
        ok!(self.custom.store_into(builder, context));

        #[cfg(feature = "venom")]
        if self.custom.is_some() && self.shard_block_refs.is_some() {
            return Err(Error::InvalidData);
        } else if let Some(refs) = &self.shard_block_refs {
            ok!(refs.store_into(builder, context));
        } else {
            ok!(self.custom.store_into(builder, context));
        }

        Ok(())
    }
}

impl<'a> Load<'a> for ShardStateUnsplit {
    fn load_from(slice: &mut CellSlice<'a>) -> Result<Self, Error> {
        let fast_finality = match slice.load_u32() {
            Ok(Self::TAG_V1) => false,
            #[cfg(feature = "venom")]
            Ok(Self::TAG_V2) => true,
            Ok(_) => return Err(Error::InvalidTag),
            Err(e) => return Err(e),
        };

        #[cfg(not(feature = "venom"))]
        let _ = fast_finality;

        let out_msg_queue_info = ok!(slice.load_reference_cloned());
        let accounts = ok!(Lazy::load_from(slice));

        let child_slice = &mut ok!(slice.load_reference_as_slice());

        let global_id = ok!(slice.load_u32()) as i32;
        let shard_ident = ok!(ShardIdent::load_from(slice));

        Ok(Self {
            global_id,
            shard_ident,
            seqno: ok!(slice.load_u32()),
            vert_seqno: ok!(slice.load_u32()),
            gen_utime: ok!(slice.load_u32()),
            #[cfg(feature = "venom")]
            gen_utime_ms: if fast_finality {
                ok!(slice.load_u16())
            } else {
                0
            },
            gen_lt: ok!(slice.load_u64()),
            min_ref_mc_seqno: ok!(slice.load_u32()),
            before_split: ok!(slice.load_bit()),
            accounts,
            overload_history: ok!(child_slice.load_u64()),
            underload_history: ok!(child_slice.load_u64()),
            total_balance: ok!(CurrencyCollection::load_from(child_slice)),
            total_validator_fees: ok!(CurrencyCollection::load_from(child_slice)),
            libraries: ok!(Dict::load_from(child_slice)),
            master_ref: ok!(Option::<BlockRef>::load_from(child_slice)),
            #[cfg(feature = "tycho")]
            externals_processed_upto: ok!(Dict::load_from(child_slice)),
            out_msg_queue_info,
            #[allow(unused_labels)]
            custom: 'custom: {
                #[cfg(feature = "venom")]
                if !shard_ident.is_masterchain() {
                    break 'custom None;
                }
                ok!(Option::<Lazy<McStateExtra>>::load_from(slice))
            },
            #[cfg(feature = "venom")]
            shard_block_refs: if shard_ident.is_masterchain() {
                None
            } else {
                Some(ok!(ShardBlockRefs::load_from(slice)))
            },
        })
    }
}

/// Next indivisible states after shard split.
#[derive(Debug, Clone, Eq, PartialEq, Store, Load)]
#[tlb(tag = "#5f327da5")]
pub struct ShardStateSplit {
    /// Reference to the state of the left shard.
    pub left: Lazy<ShardStateUnsplit>,
    /// Reference to the state of the right shard.
    pub right: Lazy<ShardStateUnsplit>,
}

/// Shared libraries currently can be present only in masterchain blocks.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LibDescr {
    /// Library code.
    pub lib: Cell,
    /// Accounts in the masterchain that store this library.
    pub publishers: Dict<HashBytes, ()>,
}

impl Store for LibDescr {
    fn store_into(&self, builder: &mut CellBuilder, _: &mut dyn CellContext) -> Result<(), Error> {
        ok!(builder.store_small_uint(0, 2));
        ok!(builder.store_reference(self.lib.clone()));
        match self.publishers.root() {
            Some(root) => builder.store_reference(root.clone()),
            None => Err(Error::InvalidData),
        }
    }
}

impl<'a> Load<'a> for LibDescr {
    fn load_from(slice: &mut CellSlice<'a>) -> Result<Self, Error> {
        if ok!(slice.load_small_uint(2)) != 0 {
            return Err(Error::InvalidTag);
        }
        Ok(Self {
            lib: ok!(slice.load_reference_cloned()),
            publishers: ok!(Dict::load_from_root_ext(slice, &mut Cell::empty_context())),
        })
    }
}

/// Out message queue info
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct OutMsgQueueInfo {
    ///  Dict (shard, seq_no): processed up to info
    pub proc_info: Dict<(u64, u32), ProcessedUpto>,
}

/// Processed up to info
#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct ProcessedUpto {
    ///  Last msg lt
    pub last_msg_lt: u64,
    /// Last msg hash
    pub last_msg_hash: HashBytes,
    /// Original shard
    pub original_shard: Option<u64>,
}

impl<'a> Load<'a> for OutMsgQueueInfo {
    fn load_from(slice: &mut CellSlice<'a>) -> Result<Self, Error> {
        Ok(Self {
            proc_info: ok!(Dict::load_from_root_ext(slice, &mut Cell::empty_context())),
        })
    }
}

impl Store for OutMsgQueueInfo {
    fn store_into(
        &self,
        builder: &mut CellBuilder,
        context: &mut dyn CellContext,
    ) -> Result<(), Error> {
        self.proc_info.store_into(builder, context)
    }
}
