//! Domain 64's lifecycle reason: one code column, both directions.
//!
//! The producer cast its own enum's discriminant onto the wire and the port
//! decoded with a literal `match` over a SECOND enum of the same name. Adding a
//! reason to one compiled; the other refused every payload carrying it, and
//! exchanging two codes on one side decoded each event as the other.

/// The wire code IS the discriminant, and the decoder comes off the same
/// column, so a reason added here reaches both sides or fails to compile.
macro_rules! lifecycle_reasons {
    ( $( $variant:ident = $code:literal, )+ ) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[repr(u32)]
        pub enum LifecycleReason {
            $($variant = $code,)+
        }

        impl LifecycleReason {
            /// The raw code on the wire. `enum_code` is this minus one, the
            /// dense form the schema carries.
            pub const fn raw_code(self) -> u32 {
                match self {
                    $(Self::$variant => $code,)+
                }
            }

            pub fn from_raw(raw: u32) -> Option<Self> {
                $(if raw == $code {
                    return Some(Self::$variant);
                })+
                None
            }
        }

        const _: () = {
            $(assert!(
                LifecycleReason::$variant as u32 == $code,
                "a lifecycle reason's declared code is not its own discriminant",
            );)+
            let codes = [$($code,)+];
            let mut i = 0;
            while i < codes.len() {
                assert!(codes[i] == i as u32 + 1, "the codes are not 1.. in order");
                let mut j = i + 1;
                while j < codes.len() {
                    assert!(codes[i] != codes[j], "two lifecycle reasons share a code");
                    j += 1;
                }
                i += 1;
            }
        };
    };
}

lifecycle_reasons! {
    RetainedDecrement = 1,
    Executed = 2,
    FailedTriggerConsumed = 3,
    DefinitionCancelled = 4,
}

impl LifecycleReason {
    /// The dense code, which is the raw code minus one. Code 0 is unassigned on
    /// the wire, which the `1..` assert above is what keeps true.
    pub const fn enum_code(self) -> u32 {
        self.raw_code() - 1
    }

    pub fn from_enum_code(code: u32) -> Option<Self> {
        Self::from_raw(code.checked_add(1)?)
    }
}
