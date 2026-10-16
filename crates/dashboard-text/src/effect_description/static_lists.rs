//! A content static modifier's raw modifier list at a scale, for hovers that show a
//! static modifier's effects in proportion to a value (the Operatives map mode's
//! network effects).

use super::*;

/// `token`'s modifier list without scope expansion, each line after `prefix`, with
/// every value multiplied by the fixed5 `scale` before formatting: precision two, no
/// outer colour, every category, no plus or polarity override.
pub fn scaled_static_modifiers(
    context: &Context<'_, '_>,
    token: &str,
    prefix: &str,
    scale: i64,
) -> Result<String, String> {
    let renderer = Renderer {
        context,
        graph: Rc::new(RefCell::new(EventScopes::new(Vec::new()))),
        evaluator: OnceCell::new(),
        out: Description::default(),
        scripted_stack: Vec::new(),
    };
    let (_, modifiers) = renderer.static_modifier(token)?;
    renderer.modifier_description_with(
        &modifiers,
        prefix,
        ModifierListOptions {
            category_mask: u32::MAX,
            scale,
            ..Default::default()
        },
    )
}
