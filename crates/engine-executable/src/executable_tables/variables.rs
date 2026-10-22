//! Built-in variable names from the selected installation's documentation.

use std::{collections::BTreeMap, fs, io, path::Path};

use super::{leak_string, verify_digest};

const DOCUMENTATION_FILE: &str = "documentation/dynamic_variables_documentation.md";
const SCOPE_HEADING: &str = "## Dynamic variables for scope ";
const GLOBAL_DIGEST: &str = "d53ce197135d0b5761e928a3f9d98aaa090945a0cc0040f697fb56b1f5a43415";
const COUNTRY_DIGEST: &str = "02a404e2978857358947650ebe3707e85d53d0cb7912e64e53d16c4388c212f3";
const STATE_DIGEST: &str = "0f68ad3d80b7fb287f8645191600899e3ddd2e5ec10a6d13f5b22280fb358c0e";
const UNIT_LEADER_DIGEST: &str = "234247476dc1902370ec71cb263af291c56ddb3d1c16097291a3138d40b8754a";
const ORGANISATION_DIGEST: &str =
    "3793c302479ab950c505ed3fe798c1141f45201ad724b82c8f51616fc584a0a4";
const PROJECT_DIGEST: &str = "6d3bcef388838cebd0d362eff5b25279c8c430f0c5f1dab0213449ee38cf9eb7";

/// Exact names in increasing byte order for the supported variable stores.
#[derive(Debug)]
pub struct VariableTables {
    pub global: Vec<&'static str>,
    pub country: Vec<&'static str>,
    pub state: Vec<&'static str>,
    pub unit_leader: Vec<&'static str>,
    /// An industrial organisation's store, documented as
    /// `military_industrial_organization`.
    pub industrial_organisation: Vec<&'static str>,
    /// A special project's store, documented as `special_project`.
    pub project: Vec<&'static str>,
}

impl VariableTables {
    /// Read and validate documentation before publishing any names.
    pub fn decode(game_dir: &Path) -> io::Result<Self> {
        let mut scopes = read_documentation(game_dir)?;
        validate_documentation(&scopes)?;
        Ok(Self {
            global: publish_scope(&mut scopes, "global"),
            country: publish_scope(&mut scopes, "country"),
            state: publish_scope(&mut scopes, "state"),
            unit_leader: publish_scope(&mut scopes, "unit_leader"),
            industrial_organisation: publish_scope(&mut scopes, ORGANISATION_SCOPE),
            project: publish_scope(&mut scopes, "special_project"),
        })
    }

    /// Validate a new installation binding without retaining its names.
    pub fn verify(game_dir: &Path) -> io::Result<()> {
        validate_documentation(&read_documentation(game_dir)?)
    }
}

/// The documentation's heading for an industrial organisation's store.
const ORGANISATION_SCOPE: &str = "military_industrial_organization";

fn read_documentation(game_dir: &Path) -> io::Result<BTreeMap<String, Vec<String>>> {
    let text = fs::read_to_string(game_dir.join(DOCUMENTATION_FILE)).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("dynamic variable documentation could not be read: {error}"),
        )
    })?;
    parse_documentation(&text)
}

fn publish_scope(scopes: &mut BTreeMap<String, Vec<String>>, scope: &str) -> Vec<&'static str> {
    scopes
        .remove(scope)
        .expect("validated variable scope")
        .into_iter()
        .map(leak_string)
        .collect()
}

fn validate_documentation(scopes: &BTreeMap<String, Vec<String>>) -> io::Result<()> {
    for (scope, expected) in [
        ("global", GLOBAL_DIGEST),
        ("country", COUNTRY_DIGEST),
        ("state", STATE_DIGEST),
        ("unit_leader", UNIT_LEADER_DIGEST),
        (ORGANISATION_SCOPE, ORGANISATION_DIGEST),
        ("special_project", PROJECT_DIGEST),
    ] {
        let names = scopes
            .get(scope)
            .ok_or_else(|| invalid_documentation(format!("missing variable scope {scope}")))?;
        let table = format!(
            "dynamic variables for scope {scope} ({} names)",
            names.len()
        );
        verify_digest(&table, &canonical_names(names), expected)?;
    }
    Ok(())
}

fn canonical_names<S: AsRef<str>>(names: &[S]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for name in names {
        bytes.extend_from_slice(name.as_ref().as_bytes());
        bytes.push(b'\n');
    }
    bytes
}

fn parse_documentation(text: &str) -> io::Result<BTreeMap<String, Vec<String>>> {
    let mut scopes = BTreeMap::<String, Vec<String>>::new();
    let mut scope = None;
    let mut fence: Option<(u8, usize)> = None;
    for (line_index, line) in text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .lines()
        .enumerate()
    {
        if let Some((delimiter, width)) = fence {
            if closes_fence(line, delimiter, width) {
                fence = None;
            }
            continue;
        }
        if let Some(opening) = opens_fence(line) {
            fence = Some(opening);
            continue;
        }
        if let Some(name) = line.strip_prefix(SCOPE_HEADING) {
            let name = name.trim();
            if !is_identifier(name) {
                return Err(invalid_documentation(format!(
                    "invalid variable scope heading on line {}",
                    line_index + 1
                )));
            }
            if scopes.insert(name.to_owned(), Vec::new()).is_some() {
                return Err(invalid_documentation(format!(
                    "duplicate variable scope {name} on line {} (1 duplicate)",
                    line_index + 1
                )));
            }
            scope = Some(name.to_owned());
            continue;
        }
        if line.starts_with("# ") || line.starts_with("## ") {
            scope = None;
            continue;
        }
        if let (Some(scope), Some(name)) = (&scope, line.strip_prefix("### "))
            && name != "Examples"
            && is_identifier(name)
        {
            scopes
                .get_mut(scope)
                .expect("current variable scope")
                .push(name.to_owned());
        }
    }
    if fence.is_some() {
        return Err(invalid_documentation("unterminated example fence"));
    }
    for (scope, names) in &mut scopes {
        names.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        let duplicates = names.windows(2).filter(|pair| pair[0] == pair[1]).count();
        if duplicates != 0 {
            let name = names.windows(2).find(|pair| pair[0] == pair[1]).unwrap();
            return Err(invalid_documentation(format!(
                "variable scope {scope} repeats identifier {} ({duplicates} duplicates)",
                name[0]
            )));
        }
    }
    Ok(scopes)
}

fn is_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn opens_fence(line: &str) -> Option<(u8, usize)> {
    let bytes = line.trim_start_matches(' ').as_bytes();
    let delimiter = *bytes.first()?;
    if delimiter != b'`' && delimiter != b'~' {
        return None;
    }
    let width = bytes.iter().take_while(|&&byte| byte == delimiter).count();
    (width >= 3).then_some((delimiter, width))
}

fn closes_fence(line: &str, delimiter: u8, width: usize) -> bool {
    let bytes = line.trim_start_matches(' ').as_bytes();
    let end = bytes.iter().take_while(|&&byte| byte == delimiter).count();
    end >= width && bytes[end..].iter().all(u8::is_ascii_whitespace)
}

fn invalid_documentation(message: impl Into<String>) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("dynamic variable documentation: {}", message.into()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_keep_exact_identifiers_and_ignore_examples() {
        let text = "### outside\n\
                    ## Dynamic variables for scope country\n\
                    ### omega_\n\
                    ### alpha2\n\
                    ### Examples\n\
                    ```text\n\
                    ## Dynamic variables for scope fake\n\
                    ### inside_example\n\
                    ```\n\
                    ### Uppercase_Key\n\
                    ### not.an.identifier\n\
                    ## Appendix\n\
                    ### outside_again\n\
                    ## Dynamic variables for scope additional_scope\n\
                    ### metadata\n";
        let scopes = parse_documentation(text).unwrap();
        assert_eq!(scopes["country"], ["Uppercase_Key", "alpha2", "omega_"]);
        assert_eq!(scopes["additional_scope"], ["metadata"]);
        assert_eq!(scopes.len(), 2);
    }

    #[test]
    fn line_endings_do_not_change_names() {
        let text = "## Dynamic variables for scope state\n### second_\n### first\n";
        assert_eq!(
            parse_documentation(text).unwrap(),
            parse_documentation(&text.replace('\n', "\r\n")).unwrap()
        );
    }

    #[test]
    fn duplicate_names_are_refused_with_scope_and_count() {
        let text = "## Dynamic variables for scope country\n\
                    ### repeated\n### repeated\n### repeated\n";
        let error = parse_documentation(text).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let message = error.to_string();
        assert!(message.contains("country"));
        assert!(message.contains("repeated"));
        assert!(message.contains("2 duplicates"));
    }

    #[test]
    fn duplicate_scope_is_refused() {
        let error = parse_documentation(
            "## Dynamic variables for scope state\n\
             ## Dynamic variables for scope state\n",
        )
        .unwrap_err();
        assert!(error.to_string().contains("duplicate variable scope state"));
    }

    #[test]
    fn required_scope_is_not_an_empty_fallback() {
        let error = validate_documentation(&BTreeMap::new()).unwrap_err();
        assert!(error.to_string().contains("missing variable scope global"));
    }

    #[test]
    fn changed_names_are_refused() {
        let scopes =
            parse_documentation("## Dynamic variables for scope global\n### replacement\n")
                .unwrap();
        assert!(validate_documentation(&scopes).is_err());
    }

    #[test]
    fn unterminated_example_is_refused() {
        assert!(parse_documentation("```\n### example\n").is_err());
    }

    fn assert_scope_digest(names: &[&str], count: usize, expected: &str, scope: &str) {
        assert_eq!(names.len(), count);
        assert!(
            names
                .windows(2)
                .all(|pair| pair[0].as_bytes() < pair[1].as_bytes())
        );
        verify_digest(scope, &canonical_names(names), expected).unwrap();
    }

    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR"]
    fn installed_global_variables_match_digest() {
        let tables = crate::executable_tables::initialize_for_tests().unwrap();
        assert_scope_digest(
            &tables.variables.global,
            19,
            GLOBAL_DIGEST,
            "global variables",
        );
    }

    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR"]
    fn installed_country_variables_match_digest() {
        let tables = crate::executable_tables::initialize_for_tests().unwrap();
        assert_scope_digest(
            &tables.variables.country,
            265,
            COUNTRY_DIGEST,
            "country variables",
        );
    }

    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR"]
    fn installed_state_variables_match_digest() {
        let tables = crate::executable_tables::initialize_for_tests().unwrap();
        assert_scope_digest(&tables.variables.state, 22, STATE_DIGEST, "state variables");
    }

    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR"]
    fn installed_unit_leader_variables_match_digest() {
        let tables = crate::executable_tables::initialize_for_tests().unwrap();
        assert_scope_digest(
            &tables.variables.unit_leader,
            66,
            UNIT_LEADER_DIGEST,
            "unit leader variables",
        );
    }

    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR"]
    fn installed_organisation_variables_match_digest() {
        let tables = crate::executable_tables::initialize_for_tests().unwrap();
        assert_scope_digest(
            &tables.variables.industrial_organisation,
            8,
            ORGANISATION_DIGEST,
            "industrial organisation variables",
        );
    }

    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR"]
    fn installed_project_variables_match_digest() {
        let tables = crate::executable_tables::initialize_for_tests().unwrap();
        assert_scope_digest(
            &tables.variables.project,
            3,
            PROJECT_DIGEST,
            "special project variables",
        );
    }
}
