use super::{GameWindowError, strip_markup};
use content_reader_gui::gui::native::{self, Element, Fields};
use content_reader_localisation::localisation::Localisation;
use content_reader_parser::parser::c_string;
use replay_data::rows::LexicalTokenRow;
use viewer_native_gui::native_gui::{self, Assets, lifecycle::Instance};

pub(crate) fn find_child<'a>(parent: &'a Element, segment: &str) -> Option<&'a Element> {
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

pub(crate) fn find_direct<'a>(root: &'a Element, path: &str) -> Option<&'a Element> {
    let mut current = root;
    for segment in path.split('/') {
        current = find_child(current, segment)?;
    }
    Some(current)
}

pub(crate) fn hide_by_name(instance: &mut Instance, root: &Element, name: &str) {
    if let Some(element) = find_direct(root, name) {
        let _ = instance.hide(root, element);
    }
}

pub(crate) fn disable_by_name(instance: &mut Instance, root: &Element, name: &str) {
    if let Some(element) = find_direct(root, name)
        && let Ok(bound) = instance.control(root, element)
    {
        bound.enabled = Some(false);
    }
}

pub(crate) fn set_text_by_name(instance: &mut Instance, root: &Element, name: &str, text: String) {
    if let Some(element) = find_direct(root, name)
        && let Ok(bound) = instance.control(root, element)
    {
        bound.text = Some(text);
    }
}

pub(crate) fn set_sprite_by_name(instance: &mut Instance, root: &Element, name: &str, sprite: &str) {
    if let Some(element) = find_direct(root, name)
        && let Ok(bound) = instance.control(root, element)
    {
        bound.sprite = Some(sprite.as_bytes().to_vec());
    }
}

pub(crate) fn set_frame_by_name(instance: &mut Instance, root: &Element, name: &str, frame: i32) {
    if let Some(element) = find_direct(root, name)
        && let Ok(bound) = instance.control(root, element)
    {
        bound.frame = Some(frame);
    }
}

pub(crate) fn expand_loc_refs(localisation: &Localisation, text: &str, depth: usize) -> String {
    if depth == 0 || !text.contains('$') {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('$') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('$') else {
            out.push('$');
            out.push_str(after);
            return out;
        };
        let token = &after[..end];
        let key = token.split('|').next().unwrap_or(token);
        if !key.is_empty()
            && key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-')
            && let Some(resolved) = localisation.get(key)
        {
            let clean = strip_markup(resolved);
            out.push_str(&expand_loc_refs(localisation, &clean, depth - 1));
        } else {
            out.push('$');
            out.push_str(token);
            out.push('$');
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

pub(crate) fn loc_or(localisation: &Localisation, key: &str) -> String {
    localisation
        .get(key)
        .map(|raw| expand_loc_refs(localisation, &strip_markup(raw), 4))
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| key.to_owned())
}

pub(crate) fn loc_fill(localisation: &Localisation, key: &str, values: &[(&str, &str)]) -> String {
    let Some(raw) = localisation.get(key) else {
        let shown: Vec<&str> = values.iter().map(|(_, value)| *value).collect();
        return format!("{key} {}", shown.join(" "));
    };
    let text = strip_markup(raw);
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(start) = rest.find('$') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('$') else {
            out.push('$');
            out.push_str(after);
            return out;
        };
        let token = &after[..end];
        let name = token.split('|').next().unwrap_or(token);
        match values.iter().find(|(placeholder, _)| *placeholder == name) {
            Some((_, value)) => out.push_str(value),
            None => out.push_str(&expand_loc_refs(localisation, &format!("${token}$"), 4)),
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

pub(crate) fn row_size(template: &Element, window: &str) -> Result<[i32; 2], GameWindowError> {
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

pub(crate) fn collect_template_assets(
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

pub(crate) fn spell(lexical: &[LexicalTokenRow], token: u32) -> Option<String> {
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
