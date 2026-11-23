use super::{Layout, native_gui};

pub fn place(
    holder: &native_gui::Holder,
    at: [i32; 2],
    layout: &Layout,
    composed: &[Vec<native_gui::Part>],
) -> Vec<native_gui::Part> {
    holder.place_parts(at, layout.composed_parts(composed))
}

#[cfg(test)]
mod tests;
