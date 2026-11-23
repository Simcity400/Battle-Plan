
use super::{Context, ProjectContent};
use dashboard_base::products::at_hour::{self, Read};
use dashboard_query::Query;
use dashboard_text::triggers::{Eval, Evaluator, Scope, Scopes};
use content_reader_key_names::characters::ScientistVisibilityTemplate;
use replay_data::rows::{Recorded, ScientistDailyRow};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, Mutex},
};

type Templates = Result<Arc<Vec<ScientistVisibilityTemplate>>, String>;
type Verdicts = HashMap<u32, Result<bool, String>>;

#[derive(Default)]
pub(super) struct Cache {
    templates: Mutex<BTreeMap<String, Templates>>,
    last: Mutex<Option<((u64, i32, u32), Verdicts)>>,
}

fn without_world(row: &ScientistDailyRow) -> Result<Option<bool>, String> {
    match row.template_visibility {
        Recorded::Value(Some(0 | 2)) => Ok(Some(true)),
        Recorded::Value(Some(1)) => Ok(None),
        _ => Err("Scientist retained visibility-template state is unavailable".into()),
    }
}

fn retained_triggers(templates: &[ScientistVisibilityTemplate], line: u32) -> Result<&content_reader_script::triggers::Triggers, String> {
    let mut found = templates.iter().filter(|template| template.line == line);
    let template = found.next().ok_or("Scientist visibility: retained role origin is absent from recorded content")?;
    if found.next().is_some() {
        return Err("Scientist visibility: retained role origin identifies multiple bodies".into());
    }
    template.triggers.as_ref().map_err(|why| format!("Scientist visibility: {why}"))
}

fn templates(cache: &Cache, content: &ProjectContent, path: &str) -> Templates {
    let mut held = cache.templates.lock().unwrap_or_else(|error| error.into_inner());
    held.entry(path.to_owned())
        .or_insert_with(|| {
            content
                .politics
                .catalogue
                .content
                .layers
                .read(path)
                .map_err(|why| format!("Scientist visibility: {path}: {why}"))
                .and_then(|bytes| content_reader_key_names::characters::parse_scientist_visibility_templates(&bytes).map(Arc::new).map_err(|why| format!("Scientist visibility: {path}: {why}")))
        })
        .clone()
}

fn triggers_of(cache: &Cache, content: Option<&ProjectContent>, row: &ScientistDailyRow) -> Result<Option<(Arc<Vec<ScientistVisibilityTemplate>>, u32)>, String> {
    let Recorded::Value(Some(origin)) = &row.template_origin else {
        return Err("Scientist visibility: nonempty retained role has no recorded authored origin".into());
    };
    let path = std::str::from_utf8(&origin.file).map_err(|_| "Scientist visibility origin is not UTF-8")?;
    let content = content.ok_or("Scientist visibility content is unavailable")?;
    let templates = templates(cache, content, path)?;
    if retained_triggers(&templates, origin.line)?.is_empty() {
        return Ok(None);
    }
    Ok(Some((templates, origin.line)))
}

pub(super) fn verdicts(context: &Context, query: &Query<'_>, read: i32, slot: u32, observed: &[(u32, &ScientistDailyRow)]) -> io::Result<Verdicts> {
    let key = (query.timeline.id, read, slot);
    let cached: Verdicts = match &*context.visibility.last.lock().unwrap_or_else(|error| error.into_inner()) {
        Some((held, verdicts)) if *held == key => verdicts.clone(),
        _ => HashMap::new(),
    };
    let content = context.content.as_deref().ok();
    let mut out = Verdicts::new();
    let mut needs = Vec::new();
    for &(character, row) in observed {
        if let Some(found) = cached.get(&character) {
            out.insert(character, found.clone());
            continue;
        }
        match without_world(row).and_then(|verdict| match verdict {
            Some(visible) => Ok(Some(visible)),
            None => triggers_of(&context.visibility, content, row).map(|triggers| match triggers {
                None => Some(true),
                Some(triggers) => {
                    needs.push((character, triggers));
                    None
                }
            }),
        }) {
            Ok(Some(visible)) => {
                out.insert(character, Ok(visible));
            }
            Ok(None) => {}
            Err(why) => {
                out.insert(character, Err(why));
            }
        }
    }
    if let (false, Some(content)) = (needs.is_empty(), content) {
        let reads: Vec<Read> = dashboard_text::triggers::RECORDED_TABLES.iter().map(|&table| Read::all(table)).collect();
        let world = at_hour::frame(query, read, &reads, "Scientist visibility").map_err(|error| format!("Scientist visibility frame is unavailable: {error}"));
        let catalogue = &content.politics.catalogue;
        for (character, (templates, line)) in needs {
            let verdict = world.as_ref().map_err(Clone::clone).and_then(|world| {
                let scripted = catalogue.scripted_content()?;
                let text = dashboard_text::scripted_text::ScriptedText::new(world, &catalogue.localisation, scripted, slot);
                let triggers = retained_triggers(&templates, line)?;
                match Evaluator::new(text.trigger_world()?, None).evaluate(triggers, Scopes::of(Scope::Character(character))) {
                    Eval::True => Ok(true),
                    Eval::False => Ok(false),
                    Eval::Unknown(reasons) => Err(format!("Scientist visibility: {}", reasons.into_iter().collect::<Vec<_>>().join("; "))),
                }
            });
            out.insert(character, verdict);
        }
    }
    *context.visibility.last.lock().unwrap_or_else(|error| error.into_inner()) = Some((key, out.clone()));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_null_or_empty_retained_template_is_visible_without_content_or_a_world() {
        let mut scientist = ScientistDailyRow { template_visibility: Recorded::Value(Some(0)), ..Default::default() };
        assert_eq!(without_world(&scientist), Ok(Some(true)));
        scientist.template_visibility = Recorded::Value(Some(2));
        assert_eq!(without_world(&scientist), Ok(Some(true)));
        scientist.template_visibility = Recorded::Value(Some(1));
        assert_eq!(without_world(&scientist), Ok(None));
        let cache = Cache::default();
        assert!(triggers_of(&cache, None, &scientist).unwrap_err().contains("nonempty retained role"));
        scientist.template_visibility = Recorded::NotRecorded;
        assert!(without_world(&scientist).is_err());
    }

    #[test]
    fn replacement_role_uses_its_own_visible_origin_instead_of_the_character_template() {
        let bodies = content_reader_key_names::characters::parse_scientist_visibility_templates(
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
