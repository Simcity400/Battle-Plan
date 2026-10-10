//! PhysicsFS path-function and archive-interface layouts.

/// Platform-independent relative-path sanitizer.
pub const SANITIZE_PATH_RVA: usize = 0x024e_3ff0;
/// Bad-filename error for a colon or backslash in a relative path.
pub const BAD_FILENAME: i32 = 0x17;

/// Filesystem existence lookup, following path sanitization.
pub const EXISTS_RVA: usize = 0x024d_e580;
/// Real-directory lookup, sanitizing before archiver selection.
pub const GET_REAL_DIRECTORY_RVA: usize = 0x024e_3810;
/// File-open entry sanitizing the path before reading.
pub const OPEN_READ_RVA: usize = 0x024d_fac0;
/// Directory-stat wrapper joining mount root and converting slashes.
pub const DIRECTORY_STAT_RVA: usize = 0x0250_0150;
/// Windows path-stat entry using file attributes.
pub const PLATFORM_STAT_RVA: usize = 0x024f_f8e0;
/// ZIP member lookup with exact bytes.
pub const ZIP_LOOKUP_RVA: usize = 0x0250_1220;
/// 7Z member lookup with exact bytes.
pub const SEVEN_ZIP_LOOKUP_RVA: usize = 0x024f_6330;
/// Directory-tree lookup using djb2 hashing and exact comparison.
pub const DIRECTORY_TREE_FIND_RVA: usize = 0x024e_1820;
/// Filesystem initialization registering ZIP and 7Z archive interfaces.
pub const INITIALIZE_RVA: usize = 0x024d_f090;
/// ZIP archive dispatch interface.
pub const ZIP_ARCHIVER_RVA: usize = 0x02b8_1910;
/// 7Z archive dispatch interface.
pub const SEVEN_ZIP_ARCHIVER_RVA: usize = 0x02b8_15e0;
/// Directory archive dispatch interface.
pub const DIRECTORY_ARCHIVER_RVA: usize = 0x02b8_1780;
/// Directory opener selecting the directory archiver.
pub const OPEN_DIRECTORY_RVA: usize = 0x024e_3a90;
