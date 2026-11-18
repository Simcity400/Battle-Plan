pub use map_content::witness::ContentReads;
use std::sync::Arc;

// Weak bank keys keep allocation identities distinct without retaining immutable payloads.
pub(crate) struct Operand<T>(std::sync::Weak<T>);
impl<T> Operand<T> {
    pub fn new(value: &Arc<T>) -> Self {
        Self(Arc::downgrade(value))
    }
}
impl<T> Clone for Operand<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> PartialEq for Operand<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}
impl<T> Eq for Operand<T> {}
impl<T> PartialOrd for Operand<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<T> Ord for Operand<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.as_ptr().cmp(&other.0.as_ptr())
    }
}

#[derive(Clone)]
pub(crate) struct KeyText(Arc<str>);
impl KeyText {
    pub fn new(value: &str) -> Self {
        Self(Arc::from(value))
    }
}
impl PartialEq for KeyText {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl Eq for KeyText {}
impl PartialOrd for KeyText {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for KeyText {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

#[cfg(test)]
mod operand_tests {
    use super::*;
    #[test]
    fn weak_input_keys_prevent_mutable_alias_and_address_reuse_without_pinning_payloads() {
        let mut bank = Arc::new(vec![1u8, 2]);
        let old = Operand::new(&bank);
        assert!(Arc::get_mut(&mut bank).is_none());
        assert!(old == old.clone());
        let payload = Arc::downgrade(&bank);
        drop(bank);
        assert!(payload.upgrade().is_none());
        let replacement = Arc::new(vec![1u8, 2]);
        assert!(old != Operand::new(&replacement));
    }
}
