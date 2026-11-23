
use super::refuse;
use viewer_foundation::tooltip_text::ensure_newlines;
use viewer_foundation::tooltip_text::{TextIcons, Tooltip, displayable, localise};
use content_reader_localisation::localisation::Localisation;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoredKey {
    hash: u64,
    bytes: Vec<u8>,
    bound: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authored {
    path: String,
    title: Option<AuthoredKey>,
    body: Option<AuthoredKey>,
    refusal: Option<String>,
}

impl Authored {
    pub fn read(element: &content_reader_gui::gui::native::Element, disabled: bool) -> Option<Self> {
        use content_reader_script::localization_binding::FormattedLocalization;
        use content_reader_gui::gui::native::tooltip_hash;
        let common = &element.common;
        let mut out = Self {
            path: element.path.clone(),
            title: None,
            body: None,
            refusal: None,
        };
        if !common.bound_tooltip.is_empty() {
            if !common.context_aware_tooltip
                && common.bound_tooltip.nodes.len() == 1
                && common.bound_tooltip.nodes[0].arguments.is_empty()
                && let FormattedLocalization::Key(key) = &common.bound_tooltip.nodes[0].key
            {
                out.title = Some(AuthoredKey {
                    hash: tooltip_hash(key),
                    bytes: key.clone(),
                    bound: true,
                });
            } else {
                out.refusal =
                    Some("occupied bound/context-aware GUI tooltip form is not supported".into());
            }
            return Some(out);
        }
        if common.tooltip == 0
            && common.delayed_tooltip == 0
            && !(disabled && (common.disabled_tooltip != 0 || common.disabled_tooltip_delayed != 0))
        {
            return None;
        }
        let key = |normal_hash, normal: &[u8], disabled_hash, disabled_key: &[u8]| {
            let (hash, bytes) = if disabled && disabled_hash != 0 {
                (disabled_hash, disabled_key)
            } else {
                (normal_hash, normal)
            };
            (hash != 0).then(|| AuthoredKey {
                hash,
                bytes: bytes.to_vec(),
                bound: false,
            })
        };
        out.title = key(
            common.tooltip,
            &common.tooltip_key,
            common.disabled_tooltip,
            &common.disabled_tooltip_key,
        );
        out.body = key(
            common.delayed_tooltip,
            &common.delayed_tooltip_key,
            common.disabled_tooltip_delayed,
            &common.disabled_tooltip_delayed_key,
        );
        Some(out)
    }

    pub fn fallback(&self, loc: &Localisation, icons: &dyn TextIcons, out: &mut Tooltip) {
        if let Some(reason) = &self.refusal {
            refuse(out, "authored tooltip", format!("{}: {reason}", self.path));
            return;
        }
        for (body, key) in [(false, &self.title), (true, &self.body)] {
            let Some(key) = key else { continue };
            let text = (|| {
                if content_reader_gui::gui::native::tooltip_hash(&key.bytes) != key.hash {
                    return Err(format!(
                        "{} stored tooltip hash does not match its key",
                        self.path
                    ));
                }
                let text_key = std::str::from_utf8(content_reader_parser::parser::c_string(&key.bytes))
                    .map_err(|_| format!("{} tooltip key is not UTF-8", self.path))?;
                if key.bound {
                    localise(loc, text_key, &[]).map(Some)
                } else {
                    Ok(loc.get(text_key).map(|text| {
                        std::str::from_utf8(content_reader_parser::parser::c_string(text.as_bytes()))
                            .expect("a UTF-8 string split at NUL remains UTF-8")
                            .to_owned()
                    }))
                }
            })();
            match text.and_then(|text| text.map(|text| displayable(icons, text)).transpose()) {
                Ok(Some(text)) => {
                    let field = if body { &mut out.body } else { &mut out.title };
                    if key.bound {
                        field.push_str(&text);
                    } else {
                        *field = text;
                    }
                    ensure_newlines(field, 1);
                }
                Ok(None) => (),
                Err(reason) => refuse(out, "authored tooltip", reason),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_gui::gui::native;

    fn element(body: &str) -> native::Element {
        native::read(format!("guiTypes = {{ guiButtonType = {{ name = w {body} }} }}").as_bytes())
            .template("w")
            .unwrap()
            .clone()
    }

    #[test]
    fn authored_disabled_hashes_fall_back_independently_to_each_normal_field() {
        content_reader_script::test_game::enter();
        let parsed = element(
            "pdx_tooltip = TITLE pdx_tooltip_delayed = BODY pdx_disabled_tooltip = DISABLED",
        );
        let normal = Authored::read(&parsed, false).unwrap();
        assert_eq!(normal.title.unwrap().bytes, b"TITLE");
        assert_eq!(normal.body.unwrap().bytes, b"BODY");
        let disabled = Authored::read(&parsed, true).unwrap();
        assert_eq!(disabled.title.unwrap().bytes, b"DISABLED");
        assert_eq!(disabled.body.unwrap().bytes, b"BODY");
        let only_disabled = element("pdx_disabled_tooltip_delayed = DISABLED_BODY");
        assert!(Authored::read(&only_disabled, false).is_none());
        assert_eq!(
            Authored::read(&only_disabled, true)
                .unwrap()
                .body
                .unwrap()
                .bytes,
            b"DISABLED_BODY"
        );
    }

    #[test]
    fn empty_context_tree_keeps_callback_and_occupied_bound_text_overrides_normal_keys() {
        content_reader_script::test_game::enter();
        let empty = element("context_aware_tooltip = { } ");
        assert!(empty.common.bound_tooltip.is_empty());
        assert!(Authored::read(&empty, false).is_none());
        let scalar = element("bound_tooltip = BOUND pdx_tooltip = NORMAL");
        assert_eq!(
            Authored::read(&scalar, false).unwrap().title.unwrap().bytes,
            b"BOUND"
        );
        let complex = element(
            "bound_tooltip = { localization_key = BOUND VALUE = { localization_key = ARG } } pdx_tooltip = NORMAL",
        );
        let refused = Authored::read(&complex, false).unwrap();
        assert!(refused.refusal.is_some());
        assert!(refused.title.is_none() && refused.body.is_none());
    }
}
