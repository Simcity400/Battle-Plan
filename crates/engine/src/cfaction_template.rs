//! NFactions::CFactionTemplate, referenced by CFaction's nullable template pointer.
//! Its bound localisation is not a raw key/string; no new interner walk is used.
//!
//! ## Template loading and naming evidence (a729, static analysis)
//!
//! - Shared database-load routine `14018B610`: `140191A27` loads string
//!   `142707C90` (`common/factions/templates`); `140191A4A` calls `140187FC0`
//!   with filter `142700814` (`.txt`). Its directory reader `140140720` calls
//!   `1424C73B0` at `1401407B3` with depth zero, constructs a mode-zero lexer
//!   at `1401407EB`, then calls template registration/parser `14012E4B0` at
//!   `140140807`. The VFS merges filenames, reads the winning layer for each,
//!   and orders the direct files bytewise (`installed_content::script_files`).
//! - On a new token, `14012E612` calls constructor `1401475D0`: `1401475EB`
//!   loads vtable `142704100`, `1401475F2` installs it, and `140147608` /
//!   `14014760F` construct `CBoundLocalization@140146AF0` at template `+0xD0`.
//!   Vtable slot 3 is `CDatabaseObject::Read@1424A96C0`, whose persistent
//!   loop dispatches slot 4, `CFactionTemplate::ReadToken@140A220D0`.
//! - The sole name dispatch is token `0x1B`: `140A2210A` / `140A2210E`
//!   select it; `140A2219F` passes `this+0xD0` to `1424ABB40` at `140A221A9`.
//!   Registrar `140082E0E` reads `142AC5BA4` (u32 `0x1B`), `140082E21`
//!   loads `1427101C8` (`name`), and `140082E31` registers that pair.
//! - `CBoundLocalization::Read@142390E40` clears the previous binding.
//!   A scalar constructs formatted localisation at `14239119E` (`142392F40`);
//!   evaluation `14238F3B0` -> `142393810` calls localiser `142230F20` at
//!   `14239386F`. It therefore treats ordinary scalar text as a localisation
//!   key, retaining the localiser's literal fallback for a missing key. A
//!   block uses `1423912A0` (`localization_key`, token `0x31F`, plus bindings);
//!   formatted callbacks and token values are distinct forms, not plain keys.
//!   Without `name`, construction leaves zero entries; `14238F0BA` tests
//!   that count and `14238F0C0`..`14238F0D6` return empty text. There is no
//!   fallback to the template identifier. A later omitted name keeps an
//!   earlier binding because duplicate bodies reuse the object.
//! - `14012E55B` interns the script-name key; `14012E5A2` compares its u32
//!   token with the database slot, and `14012E66D` stores a new token at `+8`.
//!   A match loads the same object at `14012E5DC` and jumps at `14012E5E3`
//!   directly to its virtual Read (`14012E70C`), without reset: definitions
//!   merge, and each explicit `name` replaces the earlier binding. The lexer
//!   folds ASCII case (`1424A6500` -> `1424A65A0` -> `1424A5EE0`). Thus one
//!   recorded token identifies one effective object, possibly from many bodies.
//!   `CFaction::Serialize@140D7B4D0` loads its template at `140D7B7BD` and
//!   spells `template+8` through `1424A96A0` -> `1424A7300` at `140D7B7C9`.
//! - `CFaction::display_name@140D77360` copies nonempty literal `+0x18`
//!   first, otherwise evaluates template `+0xD0` through `1404A7A00` ->
//!   `142390C20`; no template returns empty text. Literal bytes are not keys.

pub const VTABLE_RVA: usize = 0x0270_4100;
/// CFactionTemplate native u32 LexerToken; D7B4D0 calls 24A96A0/24A7300 to serialize its spelling.
pub const IDENTIFIER_TOKEN: usize = 0x08;
/// Embedded NPdxLoc::CBoundLocalization; 1475D0 calls its constructor 146AF0 here, and D77360 binds it for display.
pub const BOUND_LOCALISATION: usize = 0xd0;
