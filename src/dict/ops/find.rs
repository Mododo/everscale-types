use crate::cell::*;
use crate::dict::{read_label, Branch, DictBound, DictOwnedEntry, Segment};
use crate::error::Error;

/// Returns cell slice parts of the value corresponding to the key.
pub fn dict_find_owned(
    dict: Option<&Cell>,
    key_bit_len: u16,
    mut key: CellSlice<'_>,
    towards: DictBound,
    inclusive: bool,
    signed: bool,
    context: &mut dyn CellContext,
) -> Result<Option<DictOwnedEntry>, Error> {
    if key.remaining_bits() != key_bit_len {
        return Err(Error::CellUnderflow);
    }

    enum Leaf {
        Value(CellSliceRange),
        Divergence(Branch),
    }

    let root = match dict {
        Some(data) => ok!(context.load_cell(data.clone(), LoadMode::Full)),
        None => return Ok(None),
    };

    let mut original_key_range = key.range();
    let mut result_key = CellBuilder::new();

    let mut data = root.as_ref();
    let mut stack = Vec::<Segment>::new();
    let mut prev = None;

    // Try to find the required leaf
    let value_range = loop {
        let mut remaining_data = ok!(data.as_slice());

        // Read the next part of the key from the current data
        let prefix = &mut ok!(read_label(&mut remaining_data, key.remaining_bits()));

        // Match the prefix with the key
        let lcp = key.longest_common_data_prefix(prefix);
        let lcp_len = lcp.remaining_bits();
        match lcp_len.cmp(&key.remaining_bits()) {
            // If all bits match, an existing value was found
            std::cmp::Ordering::Equal => break Leaf::Value(remaining_data.range()),
            // LCP is less than prefix, an edge to slice was found
            std::cmp::Ordering::Less => {
                // LCP is less than prefix, an edge to slice was found
                if lcp_len < prefix.remaining_bits() {
                    let mut next_branch = Branch::from(ok!(key.get_bit(lcp_len)));
                    if signed && stack.is_empty() && lcp_len == 0 {
                        next_branch = next_branch.reversed();
                    }

                    break Leaf::Divergence(next_branch);
                }

                // The key contains the entire prefix, but there are still some bits left.
                // Fail fast if there are not enough references in the fork
                if data.reference_count() != 2 {
                    return Err(Error::CellUnderflow);
                }

                // Remove the LCP from the key
                key.try_advance(lcp.remaining_bits(), 0);

                // Load the next branch
                let next_branch = Branch::from(ok!(key.load_bit()));

                let child = match data.reference(next_branch as u8) {
                    Some(cell) => ok!(context.load_dyn_cell(cell, LoadMode::Full)),
                    None => return Err(Error::CellUnderflow),
                };

                // Push an intermediate edge to the stack
                stack.push(Segment {
                    data,
                    next_branch,
                    key_bit_len: key.remaining_bits(),
                });
                prev = Some((data, next_branch));
                data = child;
            }
            std::cmp::Ordering::Greater => {
                debug_assert!(false, "LCP of prefix and key can't be greater than key");
                unsafe { std::hint::unreachable_unchecked() };
            }
        }
    };

    // Return a value with the exact key
    if inclusive {
        if let Leaf::Value(value_range) = value_range {
            let cell = match stack.last() {
                Some(Segment {
                    data, next_branch, ..
                }) => match data.reference_cloned(*next_branch as u8) {
                    Some(cell) => ok!(context.load_cell(cell, LoadMode::Resolve)),
                    None => return Err(Error::CellUnderflow),
                },
                None => root,
            };

            let original_key = ok!(original_key_range.apply(key.cell()));
            ok!(result_key.store_slice_data(original_key));

            return Ok(Some((result_key, (cell, value_range))));
        }
    }

    // Rewind back to the divergent branch
    let rev_direction = towards.into_branch().reversed();
    let (mut data, mut remaining_bits, first_branch) = 'fork: {
        if let Leaf::Divergence(next_branch) = value_range {
            if next_branch == rev_direction {
                // Skip rewinding if the key diverged towards the opposite direction.
                let remaining_bits = key.remaining_bits();
                let prefix_len = key_bit_len - remaining_bits;
                original_key_range = original_key_range.get_prefix(prefix_len, 0);
                let _compatibility_gas = ok!(context.load_dyn_cell(data, LoadMode::UseGas));
                break 'fork (data, remaining_bits, None);
            }
        }

        while let Some(Segment {
            data,
            next_branch,
            key_bit_len: remaining_bits,
        }) = stack.pop()
        {
            let prefix_len = key_bit_len - remaining_bits;
            let signed_root = signed && prefix_len == 1;

            // Pop until the first diverged branch
            let first_branch = if signed_root && next_branch != rev_direction {
                rev_direction
            } else if !signed_root && next_branch == rev_direction {
                rev_direction.reversed()
            } else {
                continue;
            };

            // Remove the last bit from the prefix (we are chaning it to the opposite)
            original_key_range = original_key_range.get_prefix(prefix_len - 1, 0);
            prev = Some((data, next_branch));
            break 'fork (data, remaining_bits, Some(first_branch));
        }
        // There is no next/prev element if rewind consumed all stack
        return Ok(None);
    };

    // Store the longest suitable prefix
    let original_key = ok!(original_key_range.apply(key.cell()));
    ok!(result_key.store_slice_data(original_key));

    // Prepare the node to start the final search
    if let Some(branch) = first_branch {
        ok!(result_key.store_bit(branch.into_bit()));
        let child = match data.reference(branch as u8) {
            // TODO: possibly incorrect for signed find
            Some(child) => ok!(context.load_dyn_cell(child, LoadMode::Full)),
            None => return Err(Error::CellUnderflow),
        };
        prev = Some((data, branch));
        data = child;
    }

    // Try to find the required leaf
    let value_range = loop {
        let mut remaining_data = ok!(data.as_slice());

        // Read the key part written in the current edge
        let prefix = &ok!(read_label(&mut remaining_data, remaining_bits));
        if !prefix.is_data_empty() {
            ok!(result_key.store_slice_data(prefix));
        }

        match remaining_bits.checked_sub(prefix.remaining_bits()) {
            Some(0) => break remaining_data.range(),
            Some(remaining) => {
                if remaining_data.remaining_refs() < 2 {
                    return Err(Error::CellUnderflow);
                }
                remaining_bits = remaining - 1;
            }
            None => return Err(Error::CellUnderflow),
        }

        ok!(result_key.store_bit(rev_direction.into_bit()));

        let child = match data.reference(rev_direction as u8) {
            Some(child) => ok!(context.load_dyn_cell(child, LoadMode::Full)),
            None => return Err(Error::CellUnderflow),
        };
        prev = Some((data, rev_direction));
        data = child;
    };

    let cell = match prev {
        Some((prev, next_branch)) => match prev.reference_cloned(next_branch as u8) {
            Some(cell) => ok!(context.load_cell(cell, LoadMode::Resolve)),
            None => return Err(Error::CellUnderflow),
        },
        None => root,
    };

    Ok(Some((result_key, (cell, value_range))))
}

/// Finds the specified dict bound and returns a key and a value corresponding to the key.
pub fn dict_find_bound<'a: 'b, 'b>(
    dict: Option<&'a Cell>,
    mut key_bit_len: u16,
    bound: DictBound,
    signed: bool,
    context: &mut dyn CellContext,
) -> Result<Option<(CellBuilder, CellSlice<'b>)>, Error> {
    let mut data = match dict {
        Some(data) => ok!(context
            .load_dyn_cell(data.as_ref(), LoadMode::Full)
            .and_then(CellSlice::new)),
        None => return Ok(None),
    };

    let mut direction = None;
    let mut key = CellBuilder::new();

    // Try to find the required leaf
    loop {
        // Read the key part written in the current edge
        let prefix = ok!(read_label(&mut data, key_bit_len));
        #[allow(clippy::needless_borrow)]
        if !prefix.is_data_empty() {
            ok!(key.store_slice_data(prefix));
        }

        match key_bit_len.checked_sub(prefix.remaining_bits()) {
            Some(0) => break,
            Some(remaining) => {
                if data.remaining_refs() < 2 {
                    return Err(Error::CellUnderflow);
                }
                key_bit_len = remaining - 1;
            }
            None => return Err(Error::CellUnderflow),
        }

        let next_branch = bound.update_direction(&prefix, signed, &mut direction);
        ok!(key.store_bit(next_branch.into_bit()));

        // Load next child based on the next bit
        data = match data.cell().reference(next_branch as u8) {
            Some(data) => ok!(context
                .load_dyn_cell(data, LoadMode::Full)
                .and_then(CellSlice::new)),
            None => return Err(Error::CellUnderflow),
        };
    }

    // Return the last slice as data
    Ok(Some((key, data)))
}

/// Finds the specified dict bound and returns a key and cell slice parts corresponding to the key.
pub fn dict_find_bound_owned(
    dict: Option<&Cell>,
    mut key_bit_len: u16,
    bound: DictBound,
    signed: bool,
    context: &mut dyn CellContext,
) -> Result<Option<(CellBuilder, CellSliceParts)>, Error> {
    let root = match dict {
        Some(data) => ok!(context.load_cell(data.clone(), LoadMode::Full)),
        None => return Ok(None),
    };
    let mut data = ok!(root.as_slice());
    let mut prev = None;

    let mut direction = None;
    let mut key = CellBuilder::new();

    // Try to find the required leaf
    loop {
        // Read the key part written in the current edge
        let prefix = ok!(read_label(&mut data, key_bit_len));
        #[allow(clippy::needless_borrow)]
        if !prefix.is_data_empty() {
            ok!(key.store_slice_data(prefix));
        }

        match key_bit_len.checked_sub(prefix.remaining_bits()) {
            Some(0) => break,
            Some(remaining) => {
                if data.remaining_refs() < 2 {
                    return Err(Error::CellUnderflow);
                }
                key_bit_len = remaining - 1;
            }
            None => return Err(Error::CellUnderflow),
        }

        let next_branch = bound.update_direction(&prefix, signed, &mut direction);
        ok!(key.store_bit(next_branch.into_bit()));

        // Load next child based on the next bit
        prev = Some((data.cell(), next_branch));
        data = match data.cell().reference(next_branch as u8) {
            Some(data) => ok!(context
                .load_dyn_cell(data, LoadMode::Full)
                .and_then(CellSlice::new)),
            None => return Err(Error::CellUnderflow),
        };
    }

    // Build cell slice parts
    let range = data.range();
    let slice = match prev {
        Some((prev, next_branch)) => {
            let cell = match prev.reference_cloned(next_branch as u8) {
                Some(cell) => ok!(context.load_cell(cell, LoadMode::Resolve)),
                None => return Err(Error::CellUnderflow),
            };
            (cell, range)
        }
        None => (root, range),
    };

    // Return the last slice as data
    Ok(Some((key, slice)))
}
