//! One code column for a portable-workspace wire enum.
//!
//! THE DEFECT. The encoder was one `match` and the decoder another over the
//! same mapping, so moving one value's encode arm alone compiled and made
//! newly written data unreadable by its own reader (lane TV, row R4).
//!
//! Written here rather than taken from the extractor's `compact_contract`:
//! this workspace does not depend on the extractor's, which is the same reason
//! [`crate::hud_role`] carries its own copy of the law. This one generates
//! only the mapping, for taxonomies whose enum is written by hand.
//!
//! The decoder is a `match`, not the equality chain the extractor's copy uses:
//! `replay_app`'s `no_silent_skip` scan refuses a `macro_rules!` whose
//! expansion contains `return`.

/// Both directions of a wire code, from one column.
#[macro_export]
macro_rules! portable_wire_coded_codes {
    (
        $enum:ident : $repr:ty, code = $cvis:vis $code_fn:ident,
        $decode:ident = $dvis:vis $from_fn:ident($arg:ident) -> $ret:ty { $fallback:expr };
        $( $variant:ident = $code:literal, )+
    ) => {
        impl $enum {
            $cvis const fn $code_fn(self) -> $repr {
                match self {
                    $(Self::$variant => $code,)+
                }
            }

            $dvis fn $from_fn($arg: $repr) -> $ret {
                match $arg {
                    $($code => $decode(Self::$variant),)+
                    _ => $fallback,
                }
            }
        }

        const _: () = {
            let codes: [$repr; [$($code,)+].len()] = [$($code,)+];
            let mut i = 0;
            while i < codes.len() {
                let mut j = i + 1;
                while j < codes.len() {
                    assert!(codes[i] != codes[j], "two wire values share a code");
                    j += 1;
                }
                i += 1;
            }
        };
    };
}
