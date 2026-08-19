pub const TAG_STRING_DATA_OFFSET: usize = 0x00;

pub const TAG_STRING_CAPACITY_OFFSET: usize = 0x08;

pub const TAG_STRING_SIZE_OFFSET: usize = 0x0C;

pub const TAG_STRING_STRIDE: usize = 0x20;

pub const MAX_STRUCTURAL_TAG_DB_SIZE: u32 = i32::MAX as u32;

#[inline]
#[must_use]
pub fn tag_db_size_structurally_valid(size: u32, capacity: u32) -> bool {
    size > 0 && size <= capacity && capacity <= MAX_STRUCTURAL_TAG_DB_SIZE
}

#[inline]
#[must_use]
pub fn tag_db_id_in_table(id: u32, table_size: u32) -> bool {
    id < table_size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_id_predicate_matches_the_accessor_range() {
        assert!(tag_db_id_in_table(0, 440), "the engine clamps to slot 0");
        assert!(tag_db_id_in_table(1, 440));
        assert!(tag_db_id_in_table(439, 440));
        assert!(!tag_db_id_in_table(440, 440), "size is EXCLUSIVE");
        assert!(!tag_db_id_in_table(441, 440));
        assert!(tag_db_id_in_table(4_096, 12_000));
        assert!(!tag_db_id_in_table(12_000, 12_000));
        assert!(!tag_db_id_in_table(1, 0));
    }

    #[test]
    fn structural_validity_follows_the_vector_layout() {
        assert!(tag_db_size_structurally_valid(440, 1_024));
        assert!(tag_db_size_structurally_valid(1, 1));
        assert!(
            !tag_db_size_structurally_valid(0, 1_024),
            "a loaded database always holds slot 0"
        );
        assert!(
            !tag_db_size_structurally_valid(1_025, 1_024),
            "size may never exceed capacity"
        );
        assert!(
            !tag_db_size_structurally_valid(1, MAX_STRUCTURAL_TAG_DB_SIZE + 1),
            "the engine indexes with a signed int"
        );
        assert!(tag_db_size_structurally_valid(
            MAX_STRUCTURAL_TAG_DB_SIZE,
            MAX_STRUCTURAL_TAG_DB_SIZE
        ));
    }

    #[test]
    fn an_impossible_header_is_not_an_empty_database() {
        assert!(
            !tag_db_size_structurally_valid(0, 1),
            "size 0 is not loaded"
        );
        assert!(
            !tag_db_size_structurally_valid(1, 0),
            "a vector cannot hold more elements than it has room for"
        );
        assert!(!tag_db_size_structurally_valid(2, 1));
        assert!(tag_db_size_structurally_valid(2, 2));
        assert!(tag_db_size_structurally_valid(2, 4));
    }
}
