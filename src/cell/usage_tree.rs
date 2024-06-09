use super::cell_impl::VirtualCellWrapper;
use super::{Cell, CellDescriptor, CellImpl, DynCell, HashBytes};
use crate::util::TryAsMut;

#[cfg(feature = "stats")]
use super::CellTreeStats;

/// Rule for including cells in the usage tree.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum UsageTreeMode {
    /// Include cell on load.
    OnLoad,
    /// Include cell only when accessing references or data.
    OnDataAccess,
}

/// Usage tree for a family of cells.
pub struct UsageTree {
    state: SharedState,
}

impl UsageTree {
    /// Creates a usage tree with the specified tracking mode.
    pub fn new(mode: UsageTreeMode) -> Self {
        Self {
            state: UsageTreeState::new(mode),
        }
    }

    /// Wraps the specified cell in a usage cell to keep track
    /// of the data or links being accessed.
    pub fn track(&self, cell: &Cell) -> Cell {
        self.state.insert(cell, UsageTreeMode::OnLoad);
        self.state.wrap(cell.clone())
    }

    /// Returns `true` if the cell with the specified representation hash
    /// is present in this usage tree.
    pub fn contains(&self, repr_hash: &HashBytes) -> bool {
        self.state.contains(repr_hash)
    }

    /// Extends the usage tree with subtree tracker.
    pub fn with_subtrees(self) -> UsageTreeWithSubtrees {
        UsageTreeWithSubtrees {
            state: self.state,
            subtrees: Default::default(),
        }
    }
}

/// Usage tree for a family of cells with subtrees.
pub struct UsageTreeWithSubtrees {
    state: SharedState,
    subtrees: ahash::HashSet<HashBytes>,
}

impl UsageTreeWithSubtrees {
    /// Wraps the specified cell in a usage cell to keep track
    /// of the data or links being accessed.
    pub fn track(&self, cell: &Cell) -> Cell {
        self.state.as_ref().insert(cell, UsageTreeMode::OnLoad);
        self.state.wrap(cell.clone())
    }

    /// Returns `true` if the cell with the specified representation hash
    /// is present in this usage tree.
    pub fn contains_direct(&self, repr_hash: &HashBytes) -> bool {
        self.state.as_ref().contains(repr_hash)
    }

    /// Returns `true` if the subtree root with the specified representation hash
    /// is present in this usage tree.
    pub fn contains_subtree(&self, repr_hash: &HashBytes) -> bool {
        self.subtrees.contains(repr_hash)
    }

    /// Adds a subtree to the usage tree.
    /// Returns whether the value was newly inserted.
    pub fn add_subtree(&mut self, root: &DynCell) -> bool {
        self.subtrees.insert(*root.repr_hash())
    }
}

#[cfg(not(feature = "sync"))]
use self::rc::{SharedState, UsageCell, UsageTreeState};

#[cfg(feature = "sync")]
use self::sync::{SharedState, UsageCell, UsageTreeState};

impl CellImpl for UsageCell {
    fn descriptor(&self) -> CellDescriptor {
        self.cell.descriptor()
    }

    fn data(&self) -> &[u8] {
        if let Some(usage_tree) = self.usage_tree.upgrade() {
            usage_tree.insert(&self.cell, UsageTreeMode::OnDataAccess);
        }
        self.cell.data()
    }

    fn bit_len(&self) -> u16 {
        self.cell.bit_len()
    }

    fn reference(&self, index: u8) -> Option<&DynCell> {
        Some(self.load_reference(index)?.as_ref())
    }

    fn reference_cloned(&self, index: u8) -> Option<Cell> {
        let cell = self.load_reference(index)?.clone();

        #[cfg(not(feature = "sync"))]
        {
            Some(Cell::from(cell as std::rc::Rc<DynCell>))
        }

        #[cfg(feature = "sync")]
        {
            Some(Cell::from(cell as std::sync::Arc<DynCell>))
        }
    }

    fn virtualize(&self) -> &DynCell {
        VirtualCellWrapper::wrap(self)
    }

    fn hash(&self, level: u8) -> &HashBytes {
        self.cell.hash(level)
    }

    fn depth(&self, level: u8) -> u16 {
        self.cell.depth(level)
    }

    fn take_first_child(&mut self) -> Option<Cell> {
        self.cell.try_as_mut()?.take_first_child()
    }

    fn replace_first_child(&mut self, parent: Cell) -> Result<Cell, Cell> {
        match self.cell.try_as_mut() {
            Some(cell) => cell.replace_first_child(parent),
            None => Err(parent),
        }
    }

    fn take_next_child(&mut self) -> Option<Cell> {
        self.cell.try_as_mut()?.take_next_child()
    }

    #[cfg(feature = "stats")]
    fn stats(&self) -> CellTreeStats {
        self.cell.stats()
    }
}

#[cfg(not(feature = "sync"))]
mod rc {
    use std::rc::Rc;

    use super::UsageTreeMode;
    use crate::cell::{Cell, DynCell, HashBytes};

    pub type SharedState = Rc<UsageTreeState>;

    type VisitedCells = std::cell::RefCell<ahash::HashSet<HashBytes>>;

    pub struct UsageTreeState {
        mode: UsageTreeMode,
        visited: VisitedCells,
    }

    impl UsageTreeState {
        pub fn new(mode: UsageTreeMode) -> SharedState {
            Rc::new(Self {
                mode,
                visited: Default::default(),
            })
        }

        pub fn wrap(self: &SharedState, cell: Cell) -> Cell {
            Cell::from(Rc::new(UsageCell {
                cell,
                usage_tree: Rc::downgrade(self),
                children: Default::default(),
            }) as Rc<DynCell>)
        }

        #[inline]
        pub fn insert(&self, cell: &Cell, ctx: UsageTreeMode) {
            if self.mode == ctx {
                self.visited.borrow_mut().insert(*cell.repr_hash());
            }
        }

        #[inline]
        pub fn contains(&self, repr_hash: &HashBytes) -> bool {
            self.visited.borrow().contains(repr_hash)
        }
    }

    pub struct UsageCell {
        pub cell: Cell,
        pub usage_tree: std::rc::Weak<UsageTreeState>,
        pub children: std::cell::UnsafeCell<[Option<Rc<Self>>; 4]>,
    }

    impl UsageCell {
        pub fn load_reference(&self, index: u8) -> Option<&Rc<Self>> {
            if index < 4 {
                let children = unsafe { &mut *self.children.get() };
                Some(match &mut children[index as usize] {
                    Some(value) => value,
                    slot @ None => {
                        let child = self.cell.as_ref().reference_cloned(index)?;
                        if let Some(usage_tree) = self.usage_tree.upgrade() {
                            usage_tree.insert(&child, UsageTreeMode::OnLoad);
                        }

                        slot.insert(Rc::new(UsageCell {
                            cell: child,
                            usage_tree: self.usage_tree.clone(),
                            children: Default::default(),
                        }))
                    }
                })
            } else {
                None
            }
        }
    }
}

#[cfg(feature = "sync")]
mod sync {
    use std::cell::UnsafeCell;
    use std::sync::{Arc, Once};

    use super::UsageTreeMode;
    use crate::cell::{Cell, DynCell, HashBytes};

    pub type SharedState = Arc<UsageTreeState>;

    type VisitedCells = dashmap::DashSet<HashBytes, ahash::RandomState>;

    pub struct UsageTreeState {
        mode: UsageTreeMode,
        visited: VisitedCells,
    }

    impl UsageTreeState {
        pub fn new(mode: UsageTreeMode) -> SharedState {
            Arc::new(Self {
                mode,
                visited: Default::default(),
            })
        }

        pub fn wrap(self: &SharedState, cell: Cell) -> Cell {
            Cell::from(Arc::new(UsageCell {
                cell,
                usage_tree: Arc::downgrade(self),
                reference_states: [(); 4].map(|_| Once::new()),
                reference_data: [(); 4].map(|_| UnsafeCell::new(None)),
            }) as Arc<DynCell>)
        }

        #[inline]
        pub fn insert(&self, cell: &Cell, ctx: UsageTreeMode) {
            if self.mode == ctx {
                self.visited.insert(*cell.repr_hash());
            }
        }

        #[inline]
        pub fn contains(&self, repr_hash: &HashBytes) -> bool {
            self.visited.contains(repr_hash)
        }
    }

    pub struct UsageCell {
        pub cell: Cell,
        pub usage_tree: std::sync::Weak<UsageTreeState>,
        // TODO: Compress into one futex with bitset.
        pub reference_states: [Once; 4],
        pub reference_data: [UnsafeCell<Option<Arc<Self>>>; 4],
    }

    impl UsageCell {
        pub fn load_reference(&self, index: u8) -> Option<&Arc<Self>> {
            if index < 4 {
                let mut updated = false;
                self.reference_states[index as usize].call_once_force(|_| {
                    let Some(child) = self.cell.as_ref().reference_cloned(index) else {
                        // NOTE: Don't forget to set `None` here if `MaybeUninit` is used.
                        return;
                    };

                    updated = true;

                    // SAFETY: `UnsafeCell` data is controlled by the `Once` state.
                    unsafe {
                        *self.reference_data[index as usize].get() = Some(Arc::new(Self {
                            cell: child,
                            usage_tree: self.usage_tree.clone(),
                            reference_states: [(); 4].map(|_| Once::new()),
                            reference_data: [(); 4].map(|_| UnsafeCell::new(None)),
                        }))
                    };
                });

                // SAFETY: `UnsafeCell` data is controlled by the `Once` state.
                let child = unsafe { &*self.reference_data[index as usize].get().cast_const() };
                if crate::util::unlikely(updated) {
                    if let Some(child) = child {
                        if let Some(usage_tree) = self.usage_tree.upgrade() {
                            usage_tree.insert(&child.cell, UsageTreeMode::OnLoad);
                        }
                    }
                }

                child.as_ref()
            } else {
                None
            }
        }
    }

    // SAFETY: `UnsafeCell` data is controlled by the `Once` state.
    unsafe impl Send for UsageCell {}
    unsafe impl Sync for UsageCell {}
}
