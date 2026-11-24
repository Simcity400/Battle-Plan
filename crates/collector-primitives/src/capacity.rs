pub trait CapacityMax: Copy {
    fn merge_max(&mut self, other: Self);
}

impl CapacityMax for usize {
    fn merge_max(&mut self, other: Self) {
        *self = (*self).max(other);
    }
}

impl CapacityMax for bool {
    fn merge_max(&mut self, other: Self) {
        *self |= other;
    }
}

impl CapacityMax for () {
    fn merge_max(&mut self, _: Self) {}
}

impl<T: CapacityMax, const N: usize> CapacityMax for [T; N] {
    fn merge_max(&mut self, other: Self) {
        for (current, raised) in self.iter_mut().zip(other) {
            current.merge_max(raised);
        }
    }
}

#[macro_export]
macro_rules! capacity_max {
    ($name:ident;) => {
        impl $crate::capacity::CapacityMax for $name {
            fn merge_max(&mut self, _: Self) {}
        }
    };
    ($name:ident { $($field:ident $({ $($nested:tt)* })?),* $(,)? }) => {
        impl $crate::capacity::CapacityMax for $name {
            fn merge_max(&mut self, other: Self) {
                let Self { $($field: _,)* } = other;
                $($crate::capacity_max!(@merge self.$field, other.$field $(, { $($nested)* })?);)*
            }
        }
    };
    (@merge $current:expr, $raised:expr) => {
        $crate::capacity::CapacityMax::merge_max(&mut $current, $raised)
    };
    (@merge $current:expr, $raised:expr, { $($field:ident $({ $($nested:tt)* })?),* $(,)? }) => {
        $($crate::capacity_max!(@merge ($current).$field, ($raised).$field $(, { $($nested)* })?);)*
    };
}

