//! Visibility derives from the retained role's exact content template and its
//! supplying daily frame. No trigger is called in the live game.
use super::*;
type Templates = Result<Arc<Vec<installed_content_core::characters::ScientistVisibilityTemplate>>, String>;
type Cache = Option<(Arc<PoliticsContent>, BTreeMap<String, Templates>)>;

fn retained_triggers(
    templates: &[installed_content_core::characters::ScientistVisibilityTemplate],
    line: u32,
) -> Result<&installed_content_core::triggers::Triggers, String> {
    let mut found = templates.iter().filter(|template| template.line == line);
    let template = found
        .next()
        .ok_or("Scientist visibility: retained role origin is absent from recorded content")?;
    if found.next().is_some() {
        return Err("Scientist visibility: retained role origin identifies multiple bodies".into());
    }
    template
        .triggers
        .as_ref()
        .map_err(|why| format!("Scientist visibility: {why}"))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn at_observation(
    source: &Source,
    params: &SpecialProjectsParams,
    frame: &DecodedFrame,
    scientist: &replay_data::rows::ScientistDailyRow,
    content: Option<&Content>,
    slot: u32,
    cancelled: &AtomicBool,
) -> Result<bool, String> {
    match scientist.template_visibility {
        Recorded::Value(Some(0 | 2)) => return Ok(true),
        Recorded::Value(Some(1)) => {}
        _ => return Err("Scientist retained visibility-template state is unavailable".into()),
    }
    let Recorded::Value(Some(origin)) = &scientist.template_origin else {
        return Err(
            "Scientist visibility: nonempty retained role has no recorded authored origin".into(),
        );
    };
    let path = std::str::from_utf8(&origin.file)
        .map_err(|_| "Scientist visibility origin is not UTF-8")?;
    let content = content.ok_or("Scientist visibility content is unavailable")?;
    let cache = source.state::<Cache>();
    let templates = {
        let mut cache = cache.lock().unwrap_or_else(|error| error.into_inner());
        if cache
            .as_ref()
            .is_none_or(|(politics, _)| !Arc::ptr_eq(politics, &content.politics))
        {
            *cache = Some((Arc::clone(&content.politics), BTreeMap::new()));
        }
        cache
            .as_mut()
            .unwrap()
            .1
            .entry(path.to_owned())
            .or_insert_with(|| {
                content
                    .catalogue
                    .content
                    .layers
                    .read(path)
                    .map_err(|why| format!("Scientist visibility: {path}: {why}"))
                    .and_then(|bytes| {
                        installed_content_core::characters::parse_scientist_visibility_templates(&bytes)
                            .map(Arc::new)
                            .map_err(|why| format!("Scientist visibility: {path}: {why}"))
                    })
            })
            .clone()?
    };
    let triggers = retained_triggers(&templates, origin.line)?;
    if triggers.is_empty() {
        return Ok(true);
    }
    let full = progressive::frame_at(
        source,
        params.country().branch(),
        frame.hour,
        dashboard_text::triggers::RECORDED_TABLES,
        cancelled,
    )
    .map_err(|error| error.to_string())?
    .ok_or("Scientist visibility frame is unavailable")?;
    let scripted = content.catalogue.scripted_content()?;
    let text = dashboard_text::scripted_text::ScriptedText::new(
        &full,
        &content.catalogue.localisation,
        scripted,
        slot,
    );
    let world = text.trigger_world()?;
    use dashboard_text::triggers::{Eval, Evaluator, Scope, Scopes};
    match Evaluator::new(world, None)
        .evaluate(triggers, Scopes::of(Scope::Character(scientist.character)))
    {
        Eval::True => Ok(true),
        Eval::False => Ok(false),
        Eval::Unknown(reasons) => Err(format!(
            "Scientist visibility: {}",
            reasons.into_iter().collect::<Vec<_>>().join("; ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_null_retained_visibility_template_needs_no_content_or_trigger_frame() {
        let frame = dashboard_base::test_fixtures::source(1, 24, None);
        let replay = replay::campaign::Replay::from_records(
            dashboard_base::test_fixtures::CAMPAIGN.into(),
            &[dashboard_base::test_fixtures::records(
                1,
                vec![frame.clone()],
            )],
        )
        .unwrap();
        let (source, root) = dashboard_base::test_fixtures::file_source(replay);
        let params = SpecialProjectsParams {
            branch_id: "1".into(),
            tag: "GER".into(),
            selection_tags: Vec::new(),
            hour: Some(24),
        };
        let mut scientist = replay_data::rows::ScientistDailyRow {
            template_visibility: Recorded::Value(Some(0)),
            ..Default::default()
        };
        assert_eq!(
            at_observation(
                &source,
                &params,
                &frame,
                &scientist,
                None,
                0,
                &AtomicBool::new(false)
            ),
            Ok(true)
        );
        // Empty generated/replacement roles do not require the character's
        // authored template, content, or a trigger frame either.
        scientist.template_visibility = Recorded::Value(Some(2));
        assert_eq!(
            at_observation(
                &source,
                &params,
                &frame,
                &scientist,
                None,
                0,
                &AtomicBool::new(false)
            ),
            Ok(true)
        );
        scientist.template_visibility = Recorded::Value(Some(1));
        assert!(
            at_observation(
                &source,
                &params,
                &frame,
                &scientist,
                None,
                0,
                &AtomicBool::new(false)
            )
            .unwrap_err()
            .contains("nonempty retained role")
        );
        scientist.template_visibility = Recorded::NotRecorded;
        assert!(
            at_observation(
                &source,
                &params,
                &frame,
                &scientist,
                None,
                0,
                &AtomicBool::new(false)
            )
            .is_err()
        );
        drop(source);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn replacement_role_uses_its_own_visible_origin_instead_of_the_character_template() {
        let bodies = installed_content_core::characters::parse_scientist_visibility_templates(
            br#"
characters={ original={scientist={visible={always=yes}}} }
effect={add_scientist_role={scientist={visible={always=no}}}}
"#,
        )
        .unwrap();
        assert_eq!(bodies.len(), 2);
        let original = retained_triggers(&bodies, bodies[0].line).unwrap();
        let replacement = retained_triggers(&bodies, bodies[1].line).unwrap();
        assert_eq!(original[0].value.read_string(), Some(&b"yes"[..]));
        assert_eq!(replacement[0].value.read_string(), Some(&b"no"[..]));
    }
}
