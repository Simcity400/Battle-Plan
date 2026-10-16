//! Lookups into one hour's keyed replay tables without building an index.

/// A replay table's rows in their key order, searched by key. Frame decoding
/// admits only unique ascending keys; a slice that is not strictly ascending
/// is refused whole, so a duplicate key can never pick one of two rows.
/// Checking the order costs one pass, once per binding that shares the
/// lookup; each search is logarithmic and allocates nothing.
pub(super) struct Keyed<'a, T> {
    rows: &'a [T],
    key: fn(&T) -> u64,
}

impl<T> Clone for Keyed<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Keyed<'_, T> {}

impl<'a, T> Keyed<'a, T> {
    pub(super) fn new(rows: &'a [T], key: fn(&T) -> u64) -> Option<Self> {
        rows.is_sorted_by(|a, b| key(a) < key(b))
            .then_some(Self { rows, key })
    }

    pub(super) fn get(&self, key: u64) -> Option<&'a T> {
        let index = self.rows.binary_search_by_key(&key, self.key).ok()?;
        Some(&self.rows[index])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_strictly_ascending_keys_are_searched() {
        let key = |row: &(u64, char)| row.0;
        let rows = [(1, 'a'), (4, 'b'), (9, 'c')];
        let keyed = Keyed::new(&rows, key).unwrap();
        assert_eq!(keyed.get(4), Some(&(4, 'b')));
        assert_eq!(keyed.get(5), None);
        assert!(Keyed::new(&[(1, 'a'), (1, 'b')], key).is_none());
        assert!(Keyed::new(&[(4, 'a'), (1, 'b')], key).is_none());
        assert!(Keyed::new(&[], key).unwrap().get(1).is_none());
    }
}
