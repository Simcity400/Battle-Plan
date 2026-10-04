use std::io;

use super as native;
use crate::fields::invalid;

/// The sprite of a window's own button, through the native GUI registry
/// (FindGuiType 142247660, container_factory 142246270, FindButton 1422A4A20).
/// Every file is read in native order, including EOF closure and later
/// replacements; no fixed GUI filename or independent registry is used.
pub fn window_button_sprite(
    guis: &native::Guis,
    window: &str,
    button: &str,
    warnings: &mut Vec<String>,
) -> io::Result<String> {
    let (_, element) = kept_container_window(guis, window, warnings)?;
    let child = element
        .find(native::Kind::Button, button)
        .ok_or_else(|| invalid(format!("{window} has no {button} button")))?;
    let refusals: Vec<_> = element
        .refusals()
        .iter()
        .chain(child.refusals())
        .map(ToString::to_string)
        .collect();
    if !refusals.is_empty() {
        return Err(invalid(format!(
            "{window}.{button}: {} GUI refusals: {}",
            refusals.len(),
            refusals.join("; ")
        )));
    }
    let bytes = &child
        .button()
        .ok_or_else(|| invalid(format!("{window}.{button}: button fields unavailable")))?
        .sprite;
    let name = crate::parser::text(bytes)
        .map_err(|why| invalid(format!("{window}.{button}: sprite is {why}")))?;
    if name.is_empty() {
        return Err(invalid(format!("{button} in {window} names no sprite")));
    }
    Ok(name.to_owned())
}

/// The native registry's kept container declaration, shared by window-local
/// artwork and static text lookup. A later unread suffix can replace it.
fn kept_container_window<'a>(
    guis: &'a native::Guis,
    window: &str,
    warnings: &mut Vec<String>,
) -> io::Result<(&'a str, &'a native::Element)> {
    let declarations: Vec<_> = guis
        .files
        .iter()
        .flat_map(|(path, gui)| {
            gui.templates()
                .iter()
                .filter(move |template| template.is_named(window))
                .map(move |template| (path, template))
        })
        .collect();
    if declarations.len() > 1 {
        warnings.push(format!(
            "{window} is declared {} times ({}); the native registry selects the kept declaration.",
            declarations.len(),
            declarations
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let kept = guis
        .files
        .iter()
        .rposition(|(_, gui)| gui.template(window).is_some());
    // An unread suffix can register this name again. A later readable declaration
    // settles earlier files; unrelated refusals cannot change this lookup.
    for (path, gui) in &guis.files[kept.unwrap_or(0)..] {
        if let Some(why) = gui
            .refusals()
            .iter()
            .find(|why| why.cause == native::Cause::Unaligned)
        {
            return Err(invalid(format!(
                "{window}: GUI file {path} may register the window in its unread suffix: {why}"
            )));
        }
    }
    let (path, gui) = kept
        .map(|index| &guis.files[index])
        .ok_or_else(|| invalid(format!("no interface file declares a {window} window")))?;
    let element = gui
        .template(window)
        .ok_or_else(|| invalid(format!("no interface file declares a {window} window")))?;
    if element.kind != native::Kind::Container {
        return Err(invalid(format!(
            "{window} is {}, not the containerWindowType the game opens",
            element.kind.keyword()
        )));
    }
    Ok((path, element))
}

/// The localisation keys of a kept container's direct text boxes, through
/// the native GUI registry. Each (child, field) requests `text` or
/// `pdx_tooltip`; missing, empty or refused fields keep their own reason.
/// The first child of a name must be an `instantTextBoxType`, as the
/// container's one name map keeps it (CContainerWindowType::parse_field@1422FAF80).
pub fn window_texts(
    guis: &native::Guis,
    window: &str,
    texts: &[(&str, &str)],
    warnings: &mut Vec<String>,
) -> io::Result<Vec<Result<String, String>>> {
    let (path, element) = kept_container_window(guis, window, warnings)?;
    Ok(texts
        .iter()
        .map(|&(name, field)| {
            let child = element.children.iter().find(|child| child.is_named(name));
            let Some(child) = child else {
                return Err(format!("{window} in {path} has no {name}"));
            };
            let Some(text_box) = child.text_box() else {
                return Err(format!(
                    "{name} in {window} ({path}) is {}, not an instantTextBoxType",
                    child.kind.keyword()
                ));
            };
            let refusals: Vec<_> = element
                .refusals()
                .iter()
                .chain(child.refusals())
                .map(ToString::to_string)
                .collect();
            if !refusals.is_empty() {
                return Err(format!(
                    "{window}.{name}: {} GUI refusals: {}",
                    refusals.len(),
                    refusals.join("; ")
                ));
            }
            let bytes = match field {
                "text" => &text_box.text,
                "pdx_tooltip" => &child.common.tooltip_key,
                _ => {
                    return Err(format!(
                        "{name} in {window} ({path}): unsupported text field {field}"
                    ));
                }
            };
            let key = crate::parser::text(bytes)
                .map_err(|why| format!("{name} in {window} ({path}): {field} is {why}"))?;
            if key.is_empty() {
                return Err(format!("{name} in {window} ({path}) has no {field}"));
            }
            Ok(key.to_owned())
        })
        .collect())
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;
    use crate::ResolvedLayers;

    mod layered {
        use super::*;

        /// A content stack of `layers`, each a list of (path, text) files.
        fn stack(name: &str, layers: &[&[(&str, &str)]]) -> ResolvedLayers {
            let root = std::env::temp_dir().join(format!(
                "hoi4_gui_{name}_{}_{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&root);
            let folders: Vec<std::path::PathBuf> = layers
                .iter()
                .enumerate()
                .map(|(index, files)| {
                    let folder = root.join(index.to_string());
                    for (path, text) in *files {
                        let path = folder.join(path);
                        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                        std::fs::write(path, text).unwrap();
                    }
                    std::fs::create_dir_all(&folder).unwrap();
                    folder
                })
                .collect();
            let folders: Vec<(&std::path::Path, &[&str])> = folders
                .iter()
                .map(|folder| (folder.as_path(), &[][..]))
                .collect();
            ResolvedLayers::from_folders(&folders).unwrap()
        }

        fn combat_log_gui(combat: &str, recovered_kind: &str) -> String {
            format!(
                r#"guiTypes = {{
    containerWindowType = {{
        name = "combat_log_equipment_tab"
        instantTextBoxType = {{ name = "title_lost_in_combat" text = "{combat}" }}
        {recovered_kind} = {{ name = "title_recovered" text = "COMBAT_LOG_RECOVERED" }}
        instantTextBoxType = {{ name = "title_lost_by_attrition" text = "" }}
    }}
}}"#
            )
        }

        const TEXTS: [(&str, &str); 4] = [
            ("title_lost_in_combat", "text"),
            ("title_recovered", "text"),
            ("title_lost_by_attrition", "text"),
            ("title_missing", "text"),
        ];

        fn texts(layers: &ResolvedLayers) -> (Vec<Result<String, String>>, Vec<String>) {
            let mut warnings = Vec::new();
            let texts = window_texts(
                &native::read_layers(layers).unwrap(),
                "combat_log_equipment_tab",
                &TEXTS,
                &mut warnings,
            )
            .unwrap();
            (texts, warnings)
        }

        #[test]
        fn window_texts_are_the_kept_windows_own_text_keys() {
            let vanilla = combat_log_gui("COMBAT_LOG_LOSS_REASON_COMBAT", "instantTextBoxType");
            let layers = stack("vanilla", &[&[("interface/combatlogview.gui", &vanilla)]]);
            let (texts, warnings) = texts(&layers);
            assert_eq!(texts[0].as_deref(), Ok("COMBAT_LOG_LOSS_REASON_COMBAT"));
            assert_eq!(texts[1].as_deref(), Ok("COMBAT_LOG_RECOVERED"));
            assert!(texts[2].as_ref().unwrap_err().contains("has no text"));
            assert!(
                texts[3]
                    .as_ref()
                    .unwrap_err()
                    .contains("has no title_missing")
            );
            assert!(warnings.is_empty());
        }

        #[test]
        fn a_mod_replacing_the_file_or_redeclaring_the_window_names_its_own_keys() {
            let vanilla = combat_log_gui("COMBAT_LOG_LOSS_REASON_COMBAT", "instantTextBoxType");
            let replaced = combat_log_gui("MOD_LOST_IN_BATTLE", "instantTextBoxType");
            let layers = stack(
                "replaced",
                &[
                    &[("interface/combatlogview.gui", &vanilla)],
                    &[("interface/combatlogview.gui", &replaced)],
                ],
            );
            assert_eq!(texts(&layers).0[0].as_deref(), Ok("MOD_LOST_IN_BATTLE"));
            // A later file's declaration replaces the earlier one, with a warning.
            let redeclared = combat_log_gui("MOD_LOST_IN_BATTLE", "iconType");
            let layers = stack(
                "redeclared",
                &[
                    &[("interface/combatlogview.gui", &vanilla)],
                    &[("interface/zz_losses.gui", &redeclared)],
                ],
            );
            let (texts, warnings) = texts(&layers);
            assert_eq!(texts[0].as_deref(), Ok("MOD_LOST_IN_BATTLE"));
            assert!(
                texts[1]
                    .as_ref()
                    .unwrap_err()
                    .contains("not an instantTextBoxType")
            );
            assert_eq!(warnings.len(), 1, "{warnings:?}");
            assert!(warnings[0].contains("interface/zz_losses.gui"));
            // No declaration at all: refused.
            let layers = stack("absent", &[&[("interface/other.gui", "guiTypes = { }")]]);
            assert!(
                window_texts(
                    &native::read_layers(&layers).unwrap(),
                    "combat_log_equipment_tab",
                    &TEXTS,
                    &mut Vec::new(),
                )
                .is_err()
            );
        }
    }
}
