//! Faction template referenced by a nullable faction template pointer.
//! Its bound localisation is distinct from a raw key or string.
//!
//! Content-stack loading merges filenames, selects their winning layers and
//! orders direct files bytewise, as `installed_content::script_files` does. Repeated
//! case-folded template tokens update the same object without reset: omitted
//! names retain earlier bindings, and each explicit name replaces its binding.
//! Scalar names use localisation lookup with literal fallback; block forms
//! carry keys and bindings, and formatted callbacks are distinct forms. An
//! unnamed new template displays empty text without identifier fallback.
//! Faction display first uses its nonempty literal name, otherwise evaluates
//! the template binding; a missing template yields empty text.

/// CFactionTemplate primary vtable.
pub const VTABLE_RVA: usize = 0x0270_4100;
/// CFactionTemplate native u32 LexerToken.
pub const IDENTIFIER_TOKEN: usize = 0x08;
/// Embedded bound-localisation object used for template display.
pub const BOUND_LOCALISATION: usize = 0xd0;
