//! CBuildingDatabase loaded definitions; these layout constants do not prove lifetime or reload stability.

/// Building-database singleton pointer.
pub const SINGLETON_RVA: usize = 0x0331_6a58;
/// Graphics candidate-bucket array filled in building-ordinal order.
pub const CANDIDATE_BUCKETS: usize = 0x28;
/// Template-pointer array appending each parsed definition.
pub const TEMPLATES: usize = 0x40;
/// Template capacity, u32.
pub const TEMPLATE_CAPACITY: usize = 0x48;
/// Template count, i32, including the null object in slot zero.
pub const TEMPLATE_COUNT: usize = 0x4c;
/// Template-pointer stride.
pub const TEMPLATE_STRIDE: usize = 8;
/// Parallel key-record vector; key index i corresponds to template index i plus one.
pub const KEYS: usize = 0x58;
/// Key capacity, u32.
pub const KEY_CAPACITY: usize = 0x60;
/// Key count, i32.
pub const KEY_COUNT: usize = 0x64;
/// Separate spawn-point vector.
pub const SPAWN_POINTS: usize = 0x70;
/// Spawn-point count.
pub const SPAWN_POINT_COUNT: usize = 0x7c;
/// Port-template list filtered by each port flag.
pub const PORT_TEMPLATES: usize = 0x148;
/// Infrastructure template token; its spelling comes from the recorded lexical
/// dictionary. A missing matching template remains absent.
pub const INFRASTRUCTURE_TOKEN: u32 = 0x2fab;
/// Singleton getter.
pub const GET_SINGLETON_RVA: usize = 0x0016_1820;
/// Merged-content loader.
pub const LOAD_RVA: usize = 0x0018_b610;
/// Parsed-template loader appending matching key and template entries.
pub const APPEND_PARSED_TEMPLATE_RVA: usize = 0x0067_7080;
/// Lookup requiring one more template than key entries for the null object.
pub const LOOKUP_RVA: usize = 0x0067_27d0;
/// Railway template token; its spelling comes from recorded lexical tokens.
/// Province buildings of this type retain railway levels; the matching
/// template may be absent.
pub const RAILWAY_TEMPLATE_TOKEN: u32 = 0x4cc1;
/// Initializer filtering ports and generating spawn points and candidates.
pub const INITIALIZE_RVA: usize = 0x0067_7d30;
/// Building-file enumerator loading each returned file.
pub const ENUMERATE_BUILDING_FILES_RVA: usize = 0x0067_ae90;
/// Spawn-slot resolver selecting a candidate bucket.
pub const RESOLVE_SPAWN_SLOT_RVA: usize = 0x0067_ae30;
/// Candidate builder appending in ascending building-ordinal order.
pub const BUILD_CANDIDATE_BUCKETS_RVA: usize = 0x0067_a5d0;
/// Spawn generator matching slots to building indices.
pub const GENERATE_SPAWN_POINTS_RVA: usize = 0x0067_9740;
