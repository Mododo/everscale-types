use std::borrow::Borrow;
use std::marker::PhantomData;

use super::{aug_dict_insert, aug_dict_remove_owned, SetMode};
use crate::cell::*;
use crate::error::*;
use crate::util::*;

use super::raw::*;
use super::typed::*;
use super::{read_label, AugDictFn, DictKey};

// TODO: Just use load instead?
pub(crate) trait AugDictSkipValue<'a> {
    fn skip_value(slice: &mut CellSlice<'a>) -> bool;
}

impl<'a> AugDictSkipValue<'a> for crate::num::Tokens {
    #[inline]
    fn skip_value(slice: &mut CellSlice<'a>) -> bool {
        if let Ok(token_bytes) = slice.load_small_uint(4) {
            slice.try_advance(8 * token_bytes as u16, 0)
        } else {
            false
        }
    }
}

/// Typed augmented dictionary with fixed length keys.
///
/// # TLB scheme
///
/// ```text
/// ahm_edge#_ {n:#} {V:Type} {A:Type} {l:#} {m:#}
///   label:(HmLabel ~l n) {n = (~m) + l}
///   node:(HashmapAugNode m V A) = HashmapAug n V A;
///
/// ahmn_leaf#_ {V:Type} {A:Type} extra:A value:V = HashmapAugNode 0 V A;
/// ahmn_fork#_ {n:#} {V:Type} {A:Type} left:^(HashmapAug n V A)
///   right:^(HashmapAug n V A) extra:A = HashmapAugNode (n + 1) V A;
///
/// ahme_empty$0 {n:#} {V:Type} {A:Type} extra:A = HashmapAugE n V A;
/// ahme_root$1 {n:#} {V:Type} {A:Type} root:^(HashmapAug n V A) extra:A = HashmapAugE n V A;
/// ```
pub struct AugDict<K, A, V> {
    dict: Dict<K, (A, V)>,
    extra: A,
    _key: PhantomData<K>,
    _value: PhantomData<(A, V)>,
}

impl<K, A: ExactSize, V> ExactSize for AugDict<K, A, V> {
    #[inline]
    fn exact_size(&self) -> CellSliceSize {
        self.dict.exact_size() + self.extra.exact_size()
    }
}

impl<'a, K, A: Load<'a>, V> Load<'a> for AugDict<K, A, V> {
    #[inline]
    fn load_from(slice: &mut CellSlice<'a>) -> Result<Self, Error> {
        Ok(Self {
            dict: ok!(Dict::load_from(slice)),
            extra: ok!(A::load_from(slice)),
            _key: PhantomData,
            _value: PhantomData,
        })
    }
}

impl<K, A: Store, V> Store for AugDict<K, A, V> {
    #[inline]
    fn store_into(
        &self,
        builder: &mut CellBuilder,
        context: &mut dyn CellContext,
    ) -> Result<(), Error> {
        ok!(self.dict.store_into(builder, context));
        self.extra.store_into(builder, context)
    }
}

impl<K, A: Default, V> Default for AugDict<K, A, V> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<K, A: Clone, V> Clone for AugDict<K, A, V> {
    fn clone(&self) -> Self {
        Self {
            dict: self.dict.clone(),
            extra: self.extra.clone(),
            _key: PhantomData,
            _value: PhantomData,
        }
    }
}

impl<K, A: Eq, V> Eq for AugDict<K, A, V> {}

impl<K, A: PartialEq, V> PartialEq for AugDict<K, A, V> {
    fn eq(&self, other: &Self) -> bool {
        self.dict.eq(&other.dict) && self.extra.eq(&other.extra)
    }
}

impl<K, A: std::fmt::Debug, V> std::fmt::Debug for AugDict<K, A, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        debug_struct_field2_finish(f, "AugDict", "dict", &self.dict, "extra", &self.extra)
    }
}

impl<K, A: Default, V> AugDict<K, A, V> {
    /// Creates an empty dictionary
    pub fn new() -> Self {
        Self {
            dict: Dict::new(),
            extra: A::default(),
            _key: PhantomData,
            _value: PhantomData,
        }
    }
}

impl<K: DictKey, A, V> AugDict<K, A, V> {
    #[allow(unused)]
    pub(crate) fn load_from_root<'a>(
        slice: &mut CellSlice<'a>,
        context: &mut dyn CellContext,
    ) -> Result<Self, Error>
    where
        A: Load<'a>,
        V: AugDictSkipValue<'a>,
    {
        let (extra, root) = ok!(load_from_root::<A, V>(slice, K::BITS, context));

        Ok(Self {
            dict: Dict::from(Some(root)),
            extra,
            _key: PhantomData,
            _value: PhantomData,
        })
    }
}

impl<K, A, V> AugDict<K, A, V>
where
    K: DictKey,
    for<'a> A: Default + Load<'a>,
{
    fn update_root_extra(&mut self) -> Result<(), Error> {
        self.extra = match &self.dict.root {
            Some(root) => {
                let slice = &mut ok!(root.as_slice());
                let prefix = ok!(read_label(slice, K::BITS));
                if prefix.remaining_bits() != K::BITS {
                    ok!(slice.advance(0, 2));
                }
                ok!(A::load_from(slice))
            }
            None => A::default(),
        };
        Ok(())
    }
}

fn load_from_root<'a, A, V>(
    slice: &mut CellSlice<'a>,
    key_bit_len: u16,
    context: &mut dyn CellContext,
) -> Result<(A, Cell), Error>
where
    A: Load<'a>,
    V: AugDictSkipValue<'a>,
{
    let root = *slice;

    let label = ok!(read_label(slice, key_bit_len));
    let extra = if label.remaining_bits() != key_bit_len {
        if !slice.try_advance(0, 2) {
            return Err(Error::CellUnderflow);
        }
        ok!(A::load_from(slice))
    } else {
        let extra = ok!(A::load_from(slice));
        if !V::skip_value(slice) {
            return Err(Error::CellUnderflow);
        }
        extra
    };

    let root_bits = root.remaining_bits() - slice.remaining_bits();
    let root_refs = root.remaining_refs() - slice.remaining_refs();

    let mut b = CellBuilder::new();
    ok!(b.store_slice(root.get_prefix(root_bits, root_refs)));
    match b.build_ext(context) {
        Ok(cell) => Ok((extra, cell)),
        Err(e) => Err(e),
    }
}

impl<K, A, V> AugDict<K, A, V> {
    /// Returns `true` if the dictionary contains no elements.
    pub const fn is_empty(&self) -> bool {
        self.dict.is_empty()
    }

    /// Returns the underlying dictionary.
    #[inline]
    pub const fn dict(&self) -> &Dict<K, (A, V)> {
        &self.dict
    }

    /// Returns the root augmented value.
    #[inline]
    pub const fn root_extra(&self) -> &A {
        &self.extra
    }
}

impl<K, A, V> AugDict<K, A, V>
where
    K: Store + DictKey,
{
    /// Returns `true` if the dictionary contains a value for the specified key.
    pub fn contains_key<Q>(&self, key: Q) -> Result<bool, Error>
    where
        Q: Borrow<K>,
    {
        self.dict.contains_key(key)
    }
}

impl<K, A, V> AugDict<K, A, V>
where
    K: Store + DictKey,
{
    /// Returns the value corresponding to the key.
    pub fn get<'a: 'b, 'b, Q>(&'a self, key: Q) -> Result<Option<(A, V)>, Error>
    where
        Q: Borrow<K> + 'b,
        (A, V): Load<'a>,
    {
        self.dict.get(key)
    }
}

impl<K, A, V> AugDict<K, A, V>
where
    K: Store + DictKey,
    for<'a> A: Default + Store + Load<'a>,
    V: Store,
{
    /// Sets the augmented value associated with the key in the aug dictionary.
    ///
    /// Use [`set_ext`] if you need to use a custom cell context.
    ///
    /// [`set_ext`]: AugDict::set_ext
    pub fn set<Q, E, T>(
        &mut self,
        key: Q,
        aug: E,
        value: T,
        comparator: AugDictFn,
    ) -> Result<bool, Error>
    where
        Q: Borrow<K>,
        E: Borrow<A>,
        T: Borrow<V>,
    {
        self.set_ext(key, aug, value, comparator, &mut Cell::empty_context())
    }

    /// Sets the value associated with the key in the dictionary.
    pub fn set_ext<Q, E, T>(
        &mut self,
        key: Q,
        aug: E,
        value: T,
        comparator: AugDictFn,
        context: &mut dyn CellContext,
    ) -> Result<bool, Error>
    where
        Q: Borrow<K>,
        E: Borrow<A>,
        T: Borrow<V>,
    {
        self.insert_impl(
            key.borrow(),
            aug.borrow(),
            value.borrow(),
            SetMode::Set,
            comparator,
            context,
        )
    }

    /// Sets the augmented value associated with the key in the aug dictionary
    /// only if the key was already present in it.
    ///
    /// Use [`replace_ext`] if you need to use a custom cell context.
    ///
    /// [`replace_ext`]: AugDict::replace_ext
    pub fn replace<Q, E, T>(
        &mut self,
        key: Q,
        aug: E,
        value: T,
        comparator: AugDictFn,
    ) -> Result<bool, Error>
    where
        Q: Borrow<K>,
        E: Borrow<A>,
        T: Borrow<V>,
    {
        self.replace_ext(key, aug, value, comparator, &mut Cell::empty_context())
    }

    /// Sets the value associated with the key in the dictionary
    /// only if the key was already present in it.
    pub fn replace_ext<Q, E, T>(
        &mut self,
        key: Q,
        aug: E,
        value: T,
        comparator: AugDictFn,
        context: &mut dyn CellContext,
    ) -> Result<bool, Error>
    where
        Q: Borrow<K>,
        E: Borrow<A>,
        T: Borrow<V>,
    {
        self.insert_impl(
            key.borrow(),
            aug.borrow(),
            value.borrow(),
            SetMode::Replace,
            comparator,
            context,
        )
    }

    /// Sets the value associated with key in aug dictionary,
    /// but only if it is not already present.
    ///
    /// Use [`add_ext`] if you need to use a custom cell context.
    ///
    /// [`add_ext`]: AugDict::add_ext
    pub fn add<Q, E, T>(
        &mut self,
        key: Q,
        aug: E,
        value: T,
        comparator: AugDictFn,
    ) -> Result<bool, Error>
    where
        Q: Borrow<K>,
        E: Borrow<A>,
        T: Borrow<V>,
    {
        self.add_ext(key, aug, value, comparator, &mut Cell::empty_context())
    }

    /// Sets the value associated with key in dictionary,
    /// but only if it is not already present.
    pub fn add_ext<Q, E, T>(
        &mut self,
        key: Q,
        aug: E,
        value: T,
        comparator: AugDictFn,
        context: &mut dyn CellContext,
    ) -> Result<bool, Error>
    where
        Q: Borrow<K>,
        E: Borrow<A>,
        T: Borrow<V>,
    {
        self.insert_impl(
            key.borrow(),
            aug.borrow(),
            value.borrow(),
            SetMode::Add,
            comparator,
            context,
        )
    }

    /// Removes the value associated with key in aug dictionary.
    /// Returns an optional removed value as cell slice parts.
    pub fn remove<Q>(&mut self, key: Q, comparator: AugDictFn) -> Result<Option<(A, V)>, Error>
    where
        Q: Borrow<K>,
        for<'a> A: Load<'a> + 'static,
        for<'a> V: Load<'a> + 'static,
    {
        match ok!(self.remove_raw_ext(key, comparator, &mut Cell::empty_context())) {
            Some((cell, range)) => {
                let mut slice = ok!(range.apply(&cell));
                let extra = ok!(A::load_from(&mut slice));
                let value = ok!(V::load_from(&mut slice));
                Ok(Some((extra, value)))
            }
            None => Ok(None),
        }
    }

    /// Removes the value associated with key in dictionary.
    /// Returns an optional removed value as cell slice parts.
    pub fn remove_raw_ext<Q>(
        &mut self,
        key: Q,
        comparator: AugDictFn,
        context: &mut dyn CellContext,
    ) -> Result<Option<CellSliceParts>, Error>
    where
        Q: Borrow<K>,
    {
        self.remove_impl(key.borrow(), comparator, context)
    }

    fn insert_impl(
        &mut self,
        key: &K,
        extra: &A,
        value: &V,
        mode: SetMode,
        comparator: AugDictFn,
        context: &mut dyn CellContext,
    ) -> Result<bool, Error> {
        let mut key_builder = CellBuilder::new();
        ok!(key.store_into(&mut key_builder, &mut Cell::empty_context()));
        let inserted = ok!(aug_dict_insert(
            &mut self.dict.root,
            &mut key_builder.as_data_slice(),
            K::BITS,
            extra,
            value,
            mode,
            comparator,
            context,
        ));

        if inserted {
            ok!(self.update_root_extra());
        }

        Ok(inserted)
    }

    fn remove_impl(
        &mut self,
        key: &K,
        comparator: AugDictFn,
        context: &mut dyn CellContext,
    ) -> Result<Option<(Cell, CellSliceRange)>, Error> {
        let mut key_builder = CellBuilder::new();
        ok!(key.store_into(&mut key_builder, &mut Cell::empty_context()));
        let res = ok!(aug_dict_remove_owned(
            &mut self.dict.root,
            &mut key_builder.as_data_slice(),
            K::BITS,
            false,
            comparator,
            context,
        ));

        if res.is_some() {
            ok!(self.update_root_extra());
        }

        Ok(res)
    }
}

impl<K, A, V> AugDict<K, A, V>
where
    K: DictKey,
{
    /// Gets an iterator over the entries of the dictionary, sorted by key.
    /// The iterator element type is `Result<(K, A, V)>`.
    ///
    /// If the dictionary is invalid, finishes after the first invalid element,
    /// returning an error.
    ///
    /// # Performance
    ///
    /// In the current implementation, iterating over dictionary builds a key
    /// for each element. Use [`values`] or [`raw_values`] if you don't need keys from an iterator.
    ///
    /// [`values`]: Dict::values
    /// [`raw_values`]: Dict::raw_values
    pub fn iter<'a>(&'a self) -> AugIter<'_, K, A, V>
    where
        V: Load<'a>,
    {
        AugIter::new(self.dict.root())
    }

    /// Gets an iterator over the keys of the dictionary, in sorted order.
    /// The iterator element type is `Result<K>`.
    ///
    /// If the dictionary is invalid, finishes after the first invalid element,
    /// returning an error.
    ///
    /// # Performance
    ///
    /// In the current implementation, iterating over dictionary builds a key
    /// for each element. Use [`values`] if you don't need keys from an iterator.
    ///
    /// [`values`]: Dict::values
    pub fn keys(&'_ self) -> Keys<'_, K> {
        Keys::new(self.dict.root())
    }
}

impl<K, A, V> AugDict<K, A, V>
where
    K: DictKey,
{
    /// Gets an iterator over the augmented values of the dictionary, in order by key.
    /// The iterator element type is `Result<V>`.
    ///
    /// If the dictionary is invalid, finishes after the first invalid element,
    /// returning an error.
    pub fn values<'a>(&'a self) -> Values<'a, (A, V)>
    where
        V: Load<'a>,
    {
        Values::new(self.dict.root(), K::BITS)
    }
}

impl<K, A, V> AugDict<K, A, V>
where
    K: Store + DictKey,
{
    /// Gets an iterator over the raw entries of the dictionary, sorted by key.
    /// The iterator element type is `Result<(CellBuilder, CellSlice)>`.
    ///
    /// If the dictionary is invalid, finishes after the first invalid element,
    /// returning an error.
    ///
    /// # Performance
    ///
    /// In the current implementation, iterating over dictionary builds a key
    /// for each element. Use [`values`] or [`raw_values`] if you don't need keys from an iterator.
    ///
    /// [`values`]: AugDict::values
    /// [`raw_values`]: AugDict::raw_values
    pub fn raw_iter(&'_ self) -> RawIter<'_> {
        RawIter::new(self.dict.root(), K::BITS)
    }

    /// Gets an iterator over the raw keys of the dictionary, in sorted order.
    /// The iterator element type is `Result<CellBuilder>`.
    ///
    /// If the dictionary is invalid, finishes after the first invalid element,
    /// returning an error.
    ///
    /// # Performance
    ///
    /// In the current implementation, iterating over dictionary builds a key
    /// for each element. Use [`values`] or [`raw_values`] if you don't need keys from an iterator.
    ///
    /// [`values`]: AugDict::values
    /// [`raw_values`]: AugDict::raw_values
    pub fn raw_keys(&'_ self) -> RawKeys<'_> {
        RawKeys::new(self.dict.root(), K::BITS)
    }
}

impl<K, A, V> AugDict<K, A, V>
where
    K: DictKey,
{
    /// Gets an iterator over the raw values of the dictionary, in order by key.
    /// The iterator element type is `Result<CellSlice>`.
    ///
    /// If the dictionary is invalid, finishes after the first invalid element,
    /// returning an error.
    pub fn raw_values(&'_ self) -> RawValues<'_> {
        RawValues::new(self.dict.root(), K::BITS)
    }
}

#[cfg(feature = "serde")]
impl<K, A, V> serde::Serialize for AugDict<K, A, V>
where
    K: serde::Serialize + Store + DictKey,
    for<'a> A: serde::Serialize + Store + Load<'a>,
    for<'a> V: serde::Serialize + Load<'a>,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::{Error, SerializeMap};

        #[derive(serde::Serialize)]
        struct AugDictHelper<'a, K, A, V>
        where
            K: serde::Serialize + Store + DictKey,
            A: serde::Serialize + Store + Load<'a>,
            V: serde::Serialize + Load<'a>,
        {
            #[serde(serialize_with = "serialize_dict_entries")]
            entires: &'a AugDict<K, A, V>,
            extra: &'a A,
        }

        fn serialize_dict_entries<'a, K, A, V, S>(
            dict: &'a AugDict<K, A, V>,
            serializer: S,
        ) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
            K: serde::Serialize + Store + DictKey,
            A: serde::Serialize + Store + Load<'a>,
            V: serde::Serialize + Load<'a>,
        {
            let mut ser = serializer.serialize_map(None)?;
            for ref entry in dict.iter() {
                let (key, extra, value) = match entry {
                    Ok(entry) => entry,
                    Err(e) => return Err(Error::custom(e)),
                };
                ok!(ser.serialize_entry(key, &(extra, value)));
            }
            ser.end()
        }

        if serializer.is_human_readable() {
            AugDictHelper {
                entires: self,
                extra: &self.extra,
            }
            .serialize(serializer)
        } else {
            crate::boc::BocRepr::serialize(self, serializer)
        }
    }
}

/// An iterator over the entries of an [`AugDict`].
///
/// This struct is created by the [`iter`] method on [`AugDict`]. See its documentation for more.
///
/// [`iter`]: AugDict::iter
pub struct AugIter<'a, K, A, V> {
    inner: Iter<'a, K, (A, V)>,
}

impl<K, A, V> Clone for AugIter<'_, K, A, V> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<'a, K, A, V> AugIter<'a, K, A, V>
where
    K: DictKey,
{
    /// Creates an iterator over the entries of a dictionary.
    pub fn new(root: &'a Option<Cell>) -> Self {
        Self {
            inner: Iter::new(root),
        }
    }

    /// Changes the direction of the iterator to descending.
    #[inline]
    pub fn reversed(mut self) -> Self {
        self.inner = self.inner.reversed();
        self
    }

    /// Changes the behavior of the iterator to reverse the high bit.
    #[inline]
    pub fn signed(mut self) -> Self {
        self.inner = self.inner.signed();
        self
    }
}

impl<'a, K, A, V> Iterator for AugIter<'a, K, A, V>
where
    K: DictKey,
    (A, V): Load<'a>,
{
    type Item = Result<(K, A, V), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.inner.next()? {
            Ok((key, (aug, value))) => Some(Ok((key, aug, value))),
            Err(e) => Some(Err(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Context;

    use super::*;
    use crate::models::{AccountBlock, CurrencyCollection};
    use crate::prelude::Boc;

    #[test]
    fn dict_set() {
        let mut dict = AugDict::<u32, bool, u16>::new();
        assert_eq!(*dict.root_extra(), false);

        dict.set(123, false, 0xffff, bool_or_comp).unwrap();
        assert_eq!(dict.get(123).unwrap(), Some((false, 0xffff)));
        assert_eq!(*dict.root_extra(), false);

        dict.set(123, true, 0xcafe, bool_or_comp).unwrap();
        assert_eq!(dict.get(123).unwrap(), Some((true, 0xcafe)));
        assert_eq!(*dict.root_extra(), true);
    }

    #[test]
    fn dict_set_complex() {
        let mut dict = AugDict::<u32, bool, u32>::new();
        assert_eq!(*dict.root_extra(), false);

        for i in 0..520 {
            dict.set(i, true, 123, bool_or_comp).unwrap();
        }
        assert_eq!(*dict.root_extra(), true);
    }

    #[test]
    fn dict_replace() {
        let mut dict = AugDict::<u32, bool, u16>::new();
        assert_eq!(*dict.root_extra(), false);
        dict.replace(123, false, 0xff, bool_or_comp).unwrap();
        assert!(!dict.contains_key(123).unwrap());
        assert_eq!(*dict.root_extra(), false);

        dict.set(123, false, 0xff, bool_or_comp).unwrap();
        assert_eq!(dict.get(123).unwrap(), Some((false, 0xff)));
        assert_eq!(*dict.root_extra(), false);

        dict.replace(123, true, 0xaa, bool_or_comp).unwrap();
        assert_eq!(dict.get(123).unwrap(), Some((true, 0xaa)));
        assert_eq!(*dict.root_extra(), true);
    }

    #[test]
    fn dict_add() {
        let mut dict = AugDict::<u32, bool, u16>::new();
        assert_eq!(*dict.root_extra(), false);

        dict.add(123, false, 0x12, bool_or_comp).unwrap();
        assert_eq!(dict.get(123).unwrap(), Some((false, 0x12)));
        assert_eq!(*dict.root_extra(), false);

        dict.add(123, true, 0x11, bool_or_comp).unwrap();
        assert_eq!(dict.get(123).unwrap(), Some((false, 0x12)));
        assert_eq!(*dict.root_extra(), false);
    }

    #[test]
    fn dict_remove() {
        let mut dict = AugDict::<u32, bool, u32>::new();
        assert_eq!(*dict.root_extra(), false);

        for i in 0..10 {
            assert!(dict.set(i, i % 2 == 0, i, bool_or_comp).unwrap());
        }
        assert_eq!(*dict.root_extra(), true);

        let mut check_remove = |n: u32, expected: Option<(bool, u32)>| -> anyhow::Result<()> {
            let removed = dict.remove(n, bool_or_comp).context("Failed to remove")?;
            anyhow::ensure!(removed == expected);
            Ok(())
        };

        check_remove(0, Some((true, 0))).unwrap();

        check_remove(4, Some((true, 4))).unwrap();

        check_remove(9, Some((false, 9))).unwrap();
        check_remove(9, None).unwrap();

        check_remove(5, Some((false, 5))).unwrap();
        check_remove(5, None).unwrap();

        check_remove(100, None).unwrap();

        check_remove(1, Some((false, 1))).unwrap();
        check_remove(2, Some((true, 2))).unwrap();
        check_remove(3, Some((false, 3))).unwrap();
        check_remove(6, Some((true, 6))).unwrap();
        check_remove(7, Some((false, 7))).unwrap();
        check_remove(8, Some((true, 8))).unwrap();

        assert!(dict.is_empty());
    }

    #[test]
    fn dict_iter() {
        let mut dict = AugDict::<u32, u32, u32>::new();
        assert_eq!(*dict.root_extra(), 0);

        let mut expected_extra = 0;
        for i in 0..10 {
            expected_extra += i;
            dict.set(i, i, 9 - i, u32_add_comp).unwrap();
        }
        assert_eq!(*dict.root_extra(), expected_extra);

        let size = dict.values().count();
        assert_eq!(size, 10);

        for (i, entry) in dict.iter().enumerate() {
            let (key, aug, value) = entry.unwrap();
            assert_eq!(key, aug);
            assert_eq!(key, i as u32);
            assert_eq!(value, 9 - i as u32);
        }
    }

    #[test]
    fn aug_test() {
        fn cc_add_comp(
            left: &mut CellSlice<'_>,
            right: &mut CellSlice<'_>,
            b: &mut CellBuilder,
            cx: &mut dyn CellContext,
        ) -> Result<(), Error> {
            let mut left = CurrencyCollection::load_from(left)?;
            let right = CurrencyCollection::load_from(right)?;
            left.tokens = left
                .tokens
                .checked_add(right.tokens)
                .ok_or(Error::IntOverflow)?;
            left.store_into(b, cx)
        }

        let boc = Boc::decode(include_bytes!("./tests/account_blocks_aug_dict.boc")).unwrap();

        let original_dict = boc
            .parse::<AugDict<HashBytes, CurrencyCollection, AccountBlock>>()
            .unwrap();

        let mut data = Vec::new();
        for i in original_dict.iter() {
            if let Ok(entry) = i {
                data.push(entry);
            }
        }
        data.reverse();

        let mut new_dict: AugDict<HashBytes, CurrencyCollection, AccountBlock> = AugDict::new();
        for (key, aug, value) in data.iter() {
            new_dict.add(key, aug, value, cc_add_comp).unwrap();
        }
        assert_eq!(new_dict.root_extra(), original_dict.root_extra());

        let serialized = CellBuilder::build_from(&new_dict).unwrap();
        assert_eq!(serialized.repr_hash(), boc.repr_hash());

        for (key, _, _) in data.iter() {
            new_dict.remove(key, cc_add_comp).unwrap();
        }
        assert!(new_dict.is_empty());
        assert_eq!(new_dict.root_extra(), &CurrencyCollection::ZERO);
    }

    fn bool_or_comp(
        left: &mut CellSlice<'_>,
        right: &mut CellSlice<'_>,
        b: &mut CellBuilder,
        _: &mut dyn CellContext,
    ) -> Result<(), Error> {
        let left = left.load_bit()?;
        let right = right.load_bit()?;
        b.store_bit(left | right)
    }

    fn u32_add_comp(
        left: &mut CellSlice<'_>,
        right: &mut CellSlice<'_>,
        b: &mut CellBuilder,
        _: &mut dyn CellContext,
    ) -> Result<(), Error> {
        let left = left.load_u32()?;
        let right = right.load_u32()?;
        b.store_u32(left.saturating_add(right))
    }
}
