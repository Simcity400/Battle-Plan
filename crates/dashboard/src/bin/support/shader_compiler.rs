use std::path::{Path, PathBuf};

const MISSING: &str = "shader compiler not found: sprites, icons and portraits are refused";

fn select(
    environment: Option<PathBuf>,
    beside: Option<PathBuf>,
    user: Option<PathBuf>,
    mut is_file: impl FnMut(&Path) -> bool,
) -> Option<(&'static str, PathBuf)> {
    if let Some(path) = environment {
        return is_file(&path).then_some(("env", path));
    }
    [("beside", beside), ("user install", user)]
        .into_iter()
        .find_map(|(source, path)| {
            let path = path?;
            is_file(&path).then_some((source, path))
        })
}

pub(super) fn configure() -> &'static str {
    let compiler = select(
        std::env::var_os("BATTLE_PLAN_SHADER_COMPILER").or_else(|| std::env::var_os("HOI4_SHADER_COMPILER")).map(PathBuf::from),
        std::env::current_exe()
            .ok()
            .map(|exe| exe.with_file_name("shader-tools").join("dxc/dxc.exe")),
        std::env::var_os("LOCALAPPDATA")
            .map(|root| PathBuf::from(root).join("HOI4Replay/shader-tools/dxc/dxc.exe")),
        Path::is_file,
    );
    let source = match compiler {
        Some((source, path)) => {
            // Both callers run before any worker threads start or LOCALAPPDATA changes.
            unsafe { std::env::set_var("BATTLE_PLAN_SHADER_COMPILER", path) };
            source
        }
        None => MISSING,
    };
    eprintln!("shaderCompiler: {source}");
    source
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiler_precedence_and_missing_files() {
        let paths = || ["environment", "beside", "user"].map(|path| Some(PathBuf::from(path)));
        for (present, expected) in [
            (vec!["environment", "beside", "user"], Some("env")),
            (vec!["beside", "user"], None),
            (vec![], None),
        ] {
            let [environment, beside, user] = paths();
            let chosen = select(environment, beside, user, |path| {
                present.iter().any(|candidate| path == Path::new(candidate))
            });
            assert_eq!(chosen.map(|(source, _)| source), expected);
        }
        for (present, expected) in [
            (vec!["beside", "user"], Some("beside")),
            (vec!["user"], Some("user install")),
            (vec![], None),
        ] {
            let [_, beside, user] = paths();
            let chosen = select(None, beside, user, |path| {
                present.iter().any(|candidate| path == Path::new(candidate))
            });
            assert_eq!(chosen.as_ref().map(|(source, _)| *source), expected);
            assert_eq!(
                chosen.map_or(MISSING, |(source, _)| source),
                expected.unwrap_or(MISSING)
            );
        }
    }
}
