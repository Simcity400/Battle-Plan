
use super::*;

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
        concise: None,
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
