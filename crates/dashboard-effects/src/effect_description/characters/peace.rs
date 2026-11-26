use super::*;

impl Renderer<'_, '_, '_> {
    pub(in crate::effect_description) fn char_peace(
        &self,
        item: &Item,
        scopes: Scopes,
        conference: bool,
    ) -> Result<String, String> {
        if let Some(key) = field(children(item), "tooltip")
            .and_then(|item| item.value.read_string())
            .filter(|key| !key.is_empty())
        {
            return self.line(key, &[], scopes, true);
        }
        let operand = item
            .value
            .scalar()
            .or_else(|| field(children(item), "tag").and_then(|item| item.value.scalar()));
        let country = match operand {
            Some(operand) => self
                .evaluator()?
                .scoped_country(operand, scopes)
                .map_err(|why| why.to_string())?,
            None => match scopes.this {
                Scope::Country(country) | Scope::CountryLeader { country, .. } if country > 0 => {
                    Some(country)
                }
                _ => None,
            },
        };
        let Some(country) = country else {
            return Ok(String::new());
        };
        let name = self.country_name(country, false)?;
        self.char_native_line(
            if conference {
                b"EFFECT_START_PEACE_CONFERENCE_WITH"
            } else {
                b"EFFECT_WHITE_PEACE_WITH"
            },
            &[("COUNTRY", Argument::Text(&name))],
            scopes,
            true,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::game;
    use crate::{
        fixtures::folder_catalogue,
        effect_description::tests::Fixture,
        scripted_text::{EventObject, EventScope},
    };
    fn check(forms: &[(&str, &str)]) {
        let mut fixture = Fixture::new();
        let (catalogue, _folder) = folder_catalogue(&[
            (
                "common/defines/test.lua",
                "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23}}",
            ),
            (
                "localisation/test_l_english.yml",
                concat!(
                    "\u{feff}l_english:\n AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n",
                    " EFFECT_WHITE_PEACE_WITH:0 \"Peace $COUNTRY$ [Root.GetTag]\"\n",
                    " EFFECT_START_PEACE_CONFERENCE_WITH:0 \"Conference $COUNTRY$ [Root.GetTag]\"\n",
                    " SYN_TIP:0 \"Tip [Root.GetTag]\"\n"
                ),
            ),
        ]);
        catalogue
            .scripted_content()
            .unwrap()
            .set_synthetic_environment(Default::default());
        fixture.catalogue = catalogue;
        for &(body, expected) in forms {
            let mut graph = EventScopes::new(vec![EventScope::of(EventObject::Country(1))]);
            let out = fixture.render(body, &mut graph, Content::default());
            assert_eq!(out.text, expected, "{body}");
            assert!(out.refusals.is_empty(), "{body}: {:?}", out.refusals);
        }
    }

    #[test]
    fn peace_effect_targets_and_tooltip_overrides() {
        game::enter();
        check(&[
            ("fixture_agree_ceasefire=BBB", "Peace Beta AAA\n"),
            ("fixture_agree_ceasefire={tag=missing tooltip=SYN_TIP}", "Tip AAA\n"),
            (
                "fixture_begin_conference={score_factor=0.6 tag=BBB}",
                "Conference Beta AAA\n",
            ),
            (
                "fixture_begin_conference={tag=missing tooltip=SYN_TIP}",
                "Tip AAA\n",
            ),
        ]);
    }
}
