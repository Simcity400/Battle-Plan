//! Result accounting follows the last owner, including replies in transit.

use std::{
    fmt,
    ops::Deref,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use serde::{Serialize, Serializer};

use crate::QueryResult;

pub(crate) struct ByteBudget {
    limit: usize,
    used: AtomicUsize,
}

impl ByteBudget {
    pub(crate) fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit,
            used: AtomicUsize::new(0),
        })
    }

    pub(crate) fn used(&self) -> usize {
        self.used.load(Ordering::Relaxed)
    }

    pub(crate) fn reserve(self: &Arc<Self>, bytes: usize) -> Option<Reservation> {
        let mut previous = self.used();
        loop {
            let next = previous
                .checked_add(bytes)
                .filter(|next| *next <= self.limit)?;
            match self.used.compare_exchange_weak(
                previous,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return Some(Reservation {
                        budget: Arc::clone(self),
                        bytes,
                    });
                }
                Err(actual) => previous = actual,
            }
        }
    }
}

pub(crate) struct Reservation {
    budget: Arc<ByteBudget>,
    bytes: usize,
}

impl Reservation {
    pub(crate) fn shrink(&mut self, bytes: usize) {
        assert!(
            bytes <= self.bytes,
            "reservation cannot grow after admission"
        );
        self.budget
            .used
            .fetch_sub(self.bytes - bytes, Ordering::Relaxed);
        self.bytes = bytes;
    }

    pub(crate) fn into_result(mut self, result: QueryResult) -> QueryResultHandle {
        self.shrink(result.retained_bytes());
        QueryResultHandle(Arc::new(ChargedResult {
            result,
            _reservation: self,
        }))
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.used.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

struct ChargedResult {
    result: QueryResult,
    _reservation: Reservation,
}

/// Sharing a result does not duplicate or uncharge its payload. Cache eviction
/// alone cannot reclaim the budget while a subscriber or serializer holds it.
#[derive(Clone)]
pub struct QueryResultHandle(Arc<ChargedResult>);

impl QueryResultHandle {
    pub fn shares_result(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Deref for QueryResultHandle {
    type Target = QueryResult;

    fn deref(&self) -> &Self::Target {
        &self.0.result
    }
}

impl fmt::Debug for QueryResultHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.result.fmt(f)
    }
}

impl Serialize for QueryResultHandle {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.result.serialize(serializer)
    }
}
