use super::GameWindowError;
use content_reader_gui::gui::native::{self, Element, Fields};
use content_reader_localisation::localisation::Localisation;
use content_reader_parser::parser::c_string;
use replay_data::rows::LexicalTokenRow;
use viewer_native_gui::native_gui::{self, Assets, lifecycle::Instance};

fn find_child<'a>(parent: &'a Element, segment: &str) -> Option<&'a Element> {
    if let Some(found) = parent
        .children
        .iter()
        .find(|child| child.is_named(segment.as_bytes()))
    {
        return Some(found);
    }
    if let Fields::DropDown(dd) = &parent.fields {
        if let Some(expand) = dd.expand_button.as_deref()
            && expand.is_named(segment.as_bytes())
        {
            return Some(expand);
        }
        if let Some(found) = dd
            .inner
            .children
            .iter()
            .find(|child| child.is_named(segment.as_bytes()))
        {
            return Some(found);
        }
    }
    None
}

pub(super) fn find_direct<'a>(root: &'a Element, path: &str) -> Option<&'a Element> {
    let mut current = root;
    for segment in path.split('/') {
        current = find_child(current, segment)?;
    }
    Some(current)
}

pub(super) fn hide_by_name(instance: &mut Instance, root: &Element, name: &str) {
    if let Some(element) = find_direct(root, name) {
        let _ = instance.hide(root, element);
    }
}

pub(super) fn disable_by_name(instance: &mut Instance, root: &Element, name: &str) {
    if let Some(element) = find_direct(root, name)
        && let Ok(bound) = instance.control(root, element)
    {
        bound.enabled = Some(false);
    }
}

pub(super) fn set_text_by_name(instance: &mut Instance, root: &Element, name: &str, text: String) {
    if let Some(element) = find_direct(root, name)
        && let Ok(bound) = instance.control(root, element)
    {
        bound.text = Some(text);
    }
}

pub(super) fn set_sprite_by_name(instance: &mut Instance, root: &Element, name: &str, sprite: &str) {
    if let Some(element) = find_direct(root, name)
        && let Ok(bound) = instance.control(root, element)
    {
        bound.sprite = Some(sprite.as_bytes().to_vec());
    }
}

pub(super) fn set_frame_by_name(instance: &mut Instance, root: &Element, name: &str, frame: i32) {
    if let Some(element) = find_direct(root, name)
        && let Ok(bound) = instance.control(root, element)
    {
        bound.frame = Some(frame);
    }
}

pub(super) fn row_size(template: &Element, window: &str) -> Result<[i32; 2], GameWindowError> {
    template
        .container()
        .map(|container| [container.size.size[0].value, container.size.size[1].value])
        .filter(|&[_, height]| height > 0)
        .ok_or_else(|| {
            GameWindowError::refused(format!(
                "Template '{window}' declares no height, so its rows cannot be stacked."
            ))
        })
}

pub(super) fn collect_template_assets(
    guis: &native::Guis,
    localisation: &Localisation,
    names: &[&str],
    assets: &mut Assets,
) {
    for name in names {
        if let Some(template) = guis.template(name.as_bytes()) {
            let found = native_gui::assets_in(template, &|n| guis.template(n), localisation);
            assets.sprites.extend(found.sprites);
            assets.fonts.extend(found.fonts);
            assets.textures.extend(found.textures);
        }
    }
}

pub(super) fn spell(lexical: &[LexicalTokenRow], token: u32) -> Option<String> {
    if token == 0 {
        return None;
    }
    let index = lexical.binary_search_by_key(&token, |row| row.token).ok()?;
    let bytes = c_string(&lexical[index].literal);
    std::str::from_utf8(bytes)
        .ok()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}
