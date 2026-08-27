//! One declaration per wire enum: the discriminant column is also the decoder.

/// A wire enum whose discriminant is its code, with the decoder generated
/// from the same column.
///
/// The discriminant was the encoder and a literal `match` was the decoder, so
/// exchanging two discriminants compiled and decoded each value as the other
/// (lane TV, Codex round 10). The encoder is generated too, as an exhaustive
/// match rather than an `as` cast, so a variant left out of the column below
/// fails to compile instead of emitting a code its own decoder refuses (Codex
/// round 11).
///
/// This is a SECOND copy of the law in `compact_contract::wire_coded_codes!`,
/// and the barrier is POLICY, not crate direction: the dependency on that
/// crate exists, but `compact_contract_dependency_policy.rs` admits only d81's
/// payload authority and a taxonomy helper is not on that list (Codex round
/// 13). A fix to one copy of the law must be applied to the other.
///
/// `$decode` is `Ok` or `Some`; `$fallback` is evaluated with `$arg` bound to
/// the undecodable code.
macro_rules! wire_coded_kinds {
    (
        $enum:ident : $repr:ty,
        code = $cvis:vis $code_fn:ident,
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
            $(assert!(
                $enum::$variant as $repr == $code,
                "a wire value's declared code is not its own discriminant",
            );)+
            let codes = [$($code,)+];
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

pub(crate) use wire_coded_kinds;
