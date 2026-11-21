
#[macro_export]
macro_rules! debug_fields {
    ($ty:ident($this:ident) { $($field:ident),* $(,)? } { $($name:ident => $value:expr),* $(,)? }) => {
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let $this = self;
                f.debug_struct(stringify!($ty))
                    $(.field(stringify!($field), &$this.$field))*
                    $(.field(stringify!($name), &$value))*
                    .finish()
            }
        }
    };
}

pub fn sorted_set<T: Ord>(set: &std::collections::HashSet<T>) -> std::collections::BTreeSet<&T> {
    set.iter().collect()
}
