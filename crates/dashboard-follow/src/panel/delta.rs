#[cfg(test)]
mod tests {
    use super::{apply, encode};

    #[test]
    fn rebuilds_a_shifted_page_with_a_changed_row_and_unchanged_art() {
        let first = b"first archived row \x91".repeat(22);
        let middle = b"rows retained across the step \x81".repeat(23);
        let art = b"unchanged artwork names and URLs \x92".repeat(31);
        let before = [&first[..], &middle[..], b"cost 155", &art[..]].concat();
        let after = [b"new".as_slice(), &middle[..], b"cost 5999", &art[..]].concat();
        let patch = encode(&before, &after).expect("the repeated regions fit in copy spans");
        assert!(patch.len() < after.len() / 4);
        assert_eq!(apply(&before, &patch).as_deref(), Some(after.as_slice()));
        assert!(apply(&after, &patch).is_none(), "a different base must be refused");
        for end in [0, 11, patch.len() - 1] {
            assert!(apply(&before, &patch[..end]).is_none(), "truncated at {end}");
        }
        let mut damaged = patch;
        damaged[8] ^= 1;
        assert!(apply(&before, &damaged).is_none(), "the reconstructed body must be verified");
    }

    #[test]
    fn sends_a_full_value_if_it_would_be_smaller() {
        assert!(encode(&[7; 50], &[8; 50]).is_none());
    }
}
