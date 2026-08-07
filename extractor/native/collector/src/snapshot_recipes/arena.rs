//! Raw custody: the byte arena an observation copies into.
//!
//! The arena is one growable buffer. Copied intervals are OFFSETS into it, not
//! pointers, so growth never invalidates anything the executor is holding and
//! the frozen snapshot can be moved without fixing anything up.
//!
//! # Custody is a budget, and exhaustion is a refusal
//!
//! The re-pricing raised raw custody from ~256 MiB to 512 MiB and stated the
//! rule that goes with it: "allocation failure or a full second slot must fail
//! closed as backpressure". Both failure modes are represented:
//!
//!  * [`CustodyRefusal::BudgetExhausted`] — the observation asked for more
//!    than its accounted ceiling.
//!  * [`CustodyRefusal::AllocationFailed`] — the allocator itself said no.
//!
//! Neither is recoverable by copying less: the arena does not shrink a request
//! to fit, and there is no partial-allocation return.

use super::{CustodyRefusal, DEFAULT_CUSTODY_BUDGET_BYTES};

/// How the arena obtains memory. Behind a trait so allocation FAILURE is a
/// reachable state in tests: `Vec::try_reserve` only fails at sizes no test
/// should try to allocate.
pub trait ArenaAllocator: core::fmt::Debug {
    /// Ensure `buf` can take `additional` more bytes without reallocating.
    fn reserve(&self, buf: &mut Vec<u8>, additional: usize) -> Result<(), ()>;
}

/// The production allocator: the global one, through the fallible API.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemArenaAllocator;

impl ArenaAllocator for SystemArenaAllocator {
    fn reserve(&self, buf: &mut Vec<u8>, additional: usize) -> Result<(), ()> {
        buf.try_reserve(additional).map_err(|_| ())
    }
}

/// The byte arena plus its custody accounting.
#[derive(Debug)]
pub struct Arena {
    bytes: Vec<u8>,
    budget: u64,
    allocator: Box<dyn ArenaAllocator + Send + Sync>,
}

impl Arena {
    /// An arena on the default 512 MiB ceiling.
    pub fn new() -> Self {
        Self::with_budget(DEFAULT_CUSTODY_BUDGET_BYTES)
    }

    pub fn with_budget(budget: u64) -> Self {
        Self {
            bytes: Vec::new(),
            budget,
            allocator: Box::new(SystemArenaAllocator),
        }
    }

    pub fn with_allocator(budget: u64, allocator: Box<dyn ArenaAllocator + Send + Sync>) -> Self {
        Self {
            bytes: Vec::new(),
            budget,
            allocator,
        }
    }

    pub fn budget(&self) -> u64 {
        self.budget
    }

    /// Bytes currently in custody.
    pub fn held(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// Reserve `len` zeroed bytes and return their offset.
    ///
    /// The budget check happens FIRST and in `u128`, so an observation that
    /// asks for a span near `u64::MAX` is refused on the budget rather than
    /// wrapping into a small request.
    pub fn alloc_zeroed(&mut self, len: u64) -> Result<usize, CustodyRefusal> {
        let held = self.held();
        let wanted = u128::from(held) + u128::from(len);
        if wanted > u128::from(self.budget) {
            return Err(CustodyRefusal::BudgetExhausted {
                budget: self.budget,
                held,
                requested: len,
            });
        }
        let additional = usize::try_from(len).map_err(|_| CustodyRefusal::AllocationFailed {
            held,
            requested: len,
        })?;
        self.allocator
            .reserve(&mut self.bytes, additional)
            .map_err(|()| CustodyRefusal::AllocationFailed {
                held,
                requested: len,
            })?;
        let offset = self.bytes.len();
        self.bytes.resize(offset + additional, 0);
        Ok(offset)
    }

    pub fn slice(&self, offset: usize, len: usize) -> &[u8] {
        &self.bytes[offset..offset + len]
    }

    pub fn slice_mut(&mut self, offset: usize, len: usize) -> &mut [u8] {
        &mut self.bytes[offset..offset + len]
    }

    /// Hand the bytes to the frozen snapshot. Consumes the arena: nothing can
    /// append to an image a reader is already holding.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}
