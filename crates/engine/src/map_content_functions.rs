//! Map-content function and static-data layouts.
//! These descriptors do not invoke game functions or enable a capture path.

/// Allocator mapping zero size to one byte and setting ENOMEM on failure.
pub const HEAP_ALLOCATE_RVA: usize = 0x021b_4b50;
/// Path-and-receiver file-checksum entry.
pub const FILE_CHECKSUM_MEMBER_WRAPPER_RVA: usize = 0x024c_2aa0;
/// Opens and checks a file before passing file and path bytes to checksum helpers.
pub const FILE_AND_PATH_CHECKSUM_UPDATE_RVA: usize = 0x024c_2ab0;
/// Returns the filesystem base directory as an MSVC string.
pub const VFS_BASE_DIRECTORY_STRING_RVA: usize = 0x024c_7a80;
/// MSVC-string path deletion wrapper used for stale sidecars.
pub const DELETE_PATH_STRING_RVA: usize = 0x024c_bb50;
/// MSVC-string path existence wrapper used for sidecars.
pub const PATH_EXISTS_STRING_RVA: usize = 0x024c_bbc0;
/// Splits a pointer-length view on one byte, preserving an empty final slice.
pub const SPLIT_BYTE_VIEWS_RVA: usize = 0x024d_23b0;
/// Signed/unsigned 32-bit radix conversion with whitespace, sign, base and overflow handling.
pub const PARSE_INTEGER32_RADIX_RVA: usize = 0x0253_c770;
/// Signed radix-conversion wrapper, propagating CRT context errors.
pub const PARSE_SIGNED_INTEGER32_RVA: usize = 0x0253_d550;
/// MSVC-string inequality by length and bytes, returned in the low byte.
pub const STRING_NOT_EQUAL_RVA: usize = 0x0015_af10;
/// Filename suffix helper receiving destination, source string and suffix pointer.
pub const STRING_SUFFIX_CALLSITE_HELPER_RVA: usize = 0x0011_fb30;

/// Naval-distance cache filename initializer.
pub const INIT_NAVAL_DISTANCE_CACHE_NAME_RVA: usize = 0x0006_b2c0;
/// Naval-distance digest filename initializer.
pub const INIT_NAVAL_DISTANCE_CHECKSUM_NAME_RVA: usize = 0x0006_b310;
/// Naval-distance cache filename MSVC-string global.
pub const NAVAL_DISTANCE_CACHE_NAME_RVA: usize = 0x0332_0e98;
/// Naval-distance digest filename MSVC-string global.
pub const NAVAL_DISTANCE_CHECKSUM_NAME_RVA: usize = 0x0332_0e78;
/// City-population generator using a map and province-ID entropy sample.
pub const BUILD_CITY_POPULATION_RVA: usize = 0x0163_f450;
/// City-eligibility predicate using two masks and terrain-graphics flags.
pub const CITY_ELIGIBILITY_RVA: usize = 0x0164_09b0;
/// Naval-distance cache filename bytes read from the installed executable.
pub const NAVAL_DISTANCE_CACHE_LITERAL_RVA: usize = 0x0292_71f8;
/// Naval-distance digest filename bytes read from the installed executable.
pub const NAVAL_DISTANCE_CHECKSUM_LITERAL_RVA: usize = 0x0292_71d8;
/// Authored province-adjacency loader.
pub const LOAD_AUTHORED_ADJACENCIES_RVA: usize = 0x00a5_3290;
/// Bitmap-header preflight checking the signature and headers before resetting the stream.
pub const BITMAP_HEADER_PREFLIGHT_RVA: usize = 0x022c_f9a0;
/// Raw path existence probe used after texture-extension substitution.
pub const PATH_EXISTS_RVA: usize = 0x024c_7950;
/// Blitz order-definition token bytes read from the installed executable.
pub const BLITZ_ORDER_LITERAL_RVA: usize = 0x0296_2aa4;
/// Length of the blitz token bytes, excluding a NUL terminator.
pub const BLITZ_ORDER_LITERAL: [u8; 5] = *b"blitz";
