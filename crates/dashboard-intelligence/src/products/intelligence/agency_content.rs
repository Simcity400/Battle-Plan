
use dashboard_base::Source;
use dashboard_base::{
    catalogue::{self, Catalogue, View},
    products::politics_content,
};
use dashboard_core::config::LoadContext;
use content_reader_military::intelligence::AgencyUpgrades;
use content_reader_localisation::localisation::Argument;
use std::sync::Arc;

pub type Shown = Result<String, String>;

const DEFENSE: &str = "AGENCY_DEFENSE_LEVEL";
const MARKER: &str = "\u{e000}";

struct DefenseTemplate {
    prefix: String,
    suffix: String,
    label: String,
}

pub struct AgencyContent {
    catalogue: Arc<Catalogue>,
    upgrades: Result<AgencyUpgrades, String>,
    defense: Result<DefenseTemplate, String>,
    pub default_name: Shown,
    pub warnings: Vec<String>,
}

type Kept = Option<(usize, Arc<AgencyContent>)>;

pub fn content(source: &Source, load: &LoadContext) -> Result<Arc<AgencyContent>, String> {
    let options = load.content().map_err(|error| error.to_string())?;
    let politics = politics_content::politics_content(source, options).map_err(|error| error.to_string())?;
    let binding = Arc::as_ptr(&politics) as usize;
    let state = source.state::<Kept>();
    if let Some((held, content)) = &*state.lock().unwrap_or_else(|error| error.into_inner())
        && *held == binding
    {
        return Ok(Arc::clone(content));
    }
    let content = Arc::new(AgencyContent::load(Arc::clone(&politics.catalogue)));
    *state.lock().unwrap_or_else(|error| error.into_inner()) = Some((binding, Arc::clone(&content)));
    Ok(content)
}

impl AgencyContent {
    pub fn load(catalogue: Arc<Catalogue>) -> Self {
        let mut warnings = Vec::new();
        let upgrades = content_reader_military::intelligence::read_agency_upgrades(&catalogue.content.layers).map_err(|error| format!("Acquired upgrade content unavailable: {error}"));
        match &upgrades {
            Ok(upgrades) => warnings.extend(upgrades.warnings.iter().cloned()),
            Err(why) => warnings.push(why.clone()),
        }
        let default_name = catalogue::localised_with(&catalogue.localisation, "AGENCY_NAME", &[], View::Plain);
        let defense = DefenseTemplate::load(&catalogue);
        for why in [default_name.as_ref().err(), defense.as_ref().err()].into_iter().flatten() {
            warnings.push(why.clone());
        }
        Self { catalogue, upgrades, defense, default_name, warnings }
    }

    pub fn defense_label(&self) -> Result<&str, String> {
        self.defense.as_ref().map(|template| template.label.as_str()).map_err(Clone::clone)
    }

    pub fn defense(&self, raw: i64) -> (Shown, Shown) {
        let text = catalogue::localised_with(&self.catalogue.localisation, DEFENSE, &[("LEVEL", Argument::Fixed5(raw))], View::Plain);
        let value = match (&text, &self.defense) {
            (Ok(text), Ok(template)) => text
                .strip_prefix(&template.prefix)
                .and_then(|text| text.strip_suffix(&template.suffix))
                .map(str::to_owned)
                .ok_or_else(|| format!("{DEFENSE}: the rendered number's boundaries differ from its fixed-point output")),
            (Err(why), _) | (_, Err(why)) => Err(why.clone()),
        };
        (text, value)
    }

    pub fn upgrade_name(&self, branch: &str, token: &str) -> Shown {
        let upgrades = self.upgrades.as_ref().map_err(Clone::clone)?;
        let definition = upgrades
            .branch_upgrade(branch, token)?
            .ok_or_else(|| format!("Agency upgrade {branch}/{token} is absent from the recorded content stack"))?;
        catalogue::localised_with(&self.catalogue.localisation, &definition.name, &[], View::Plain)
    }

    pub fn check(&self) -> Vec<(String, String)> {
        let mut refused: Vec<(String, String)> = Vec::new();
        refused.extend(self.default_name.as_ref().err().map(|why| ("AGENCY_NAME".to_owned(), why.clone())));
        refused.extend(self.defense.as_ref().err().map(|why| (DEFENSE.to_owned(), why.clone())));
        match &self.upgrades {
            Ok(upgrades) => {
                for ((branch, token), _) in &upgrades.branch_definitions {
                    refused.extend(self.upgrade_name(branch, token).err().map(|why| (format!("upgrade {branch}/{token}"), why)));
                }
                refused.extend(upgrades.refusals.iter().map(|why| ("agency upgrades".to_owned(), why.clone())));
            }
            Err(why) => refused.push(("agency upgrades".to_owned(), why.clone())),
        }
        refused
    }
}

impl DefenseTemplate {
    fn load(catalogue: &Catalogue) -> Result<Self, String> {
        if catalogue.localisation.get(DEFENSE).is_none() {
            return Err(format!("{DEFENSE}: required agency defence localisation key missing from the recorded content stack"));
        }
        let text = catalogue::localised_with(&catalogue.localisation, DEFENSE, &[("LEVEL", Argument::Text(MARKER))], View::Plain)?;
        let count = text.matches(MARKER).count();
        if count != 1 {
            return Err(format!("{DEFENSE}: the template must render LEVEL once to separate its label and number; it rendered {count}"));
        }
        let (prefix, suffix) = text.split_once(MARKER).expect("one marker");
        let label = format!("{prefix}{suffix}").trim_matches(|character: char| character.is_whitespace() || matches!(character, ':' | '：')).to_owned();
        if label.is_empty() {
            return Err(format!("{DEFENSE}: the template has no label apart from LEVEL"));
        }
        Ok(Self { prefix: prefix.to_owned(), suffix: suffix.to_owned(), label })
    }
}
