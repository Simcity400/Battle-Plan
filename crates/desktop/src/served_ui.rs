
use std::{
    borrow::Cow,
    path::{Component, Path, PathBuf},
};
use tauri::{
    Assets, Runtime,
    utils::assets::{AssetKey, AssetsIter, CspHash},
};

pub const VARIABLE: &str = "BATTLEPLAN_UI_DIST";

struct Bundle(PathBuf);

impl Bundle {
    fn file(&self, key: &str) -> Option<PathBuf> {
        let relative = Path::new(key.trim_start_matches('/'));
        relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
            .then(|| self.0.join(relative))
    }
}

fn files(folder: &Path, root: &Path, into: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.is_dir() {
            files(&path, root, into);
        } else if let Ok(relative) = path.strip_prefix(root) {
            let key = relative.to_string_lossy().replace('\\', "/");
            into.push((format!("/{key}"), path));
        }
    }
}

impl<R: Runtime> Assets<R> for Bundle {
    fn get(&self, key: &AssetKey) -> Option<Cow<'_, [u8]>> {
        std::fs::read(self.file(key.as_ref())?).ok().map(Cow::Owned)
    }

    fn iter(&self) -> Box<AssetsIter<'_>> {
        let mut found = Vec::new();
        files(&self.0, &self.0, &mut found);
        Box::new(found.into_iter().filter_map(|(key, path)| {
            Some((Cow::Owned(key), Cow::Owned(std::fs::read(path).ok()?)))
        }))
    }

    fn csp_hashes(&self, _html_path: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
        Box::new(std::iter::empty())
    }
}

fn not_production(html: &str) -> Option<&'static str> {
    if html.contains("/@vite/client") || html.contains("/src/main.tsx") {
        return Some("it is the UI's source page, not `vite build` output");
    }
    let inline = html.split("<script").skip(1).any(|tag| {
        let opening = tag.split('>').next().unwrap_or_default();
        !opening.contains("src=")
    });
    inline.then_some("it holds an inline script, which the release CSP would block without its hash")
}

pub fn serve<R: Runtime>(context: &mut tauri::Context<R>) -> Result<PathBuf, String> {
    let given = std::env::var_os(VARIABLE)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!("This shell serves its UI from a production bundle: set {VARIABLE} to the folder `vite build` wrote")
        })?;
    let folder = std::fs::canonicalize(&given)
        .map_err(|error| format!("{VARIABLE} {}: {error}", Path::new(&given).display()))?;
    for page in ["index.html", "recorder.html"] {
        let html = std::fs::read_to_string(folder.join(page))
            .map_err(|error| format!("{VARIABLE} {}: no {page} ({error})", folder.display()))?;
        if let Some(reason) = not_production(&html) {
            return Err(format!("{VARIABLE} {}: {page} is refused: {reason}", folder.display()));
        }
    }
    context.set_assets(Box::new(Bundle(folder.clone())));
    Ok(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundle_serves_only_files_under_its_folder() {
        let bundle = Bundle(PathBuf::from("bundle"));
        assert_eq!(bundle.file("/assets/main.js"), Some(Path::new("bundle").join("assets/main.js")));
        assert_eq!(bundle.file("/../secret"), None);
        assert_eq!(bundle.file("/assets/../../secret"), None);
        assert_eq!(bundle.file("/C:/Windows/win.ini"), None);
    }

    #[test]
    fn only_a_production_entry_page_is_served() {
        let built = r#"<script type="module" crossorigin src="/assets/main-1a2b.js"></script>"#;
        assert_eq!(not_production(built), None);
        assert!(not_production(r#"<script type="module" src="/src/main.tsx"></script>"#).is_some());
        assert!(not_production("<script>window.x = 1</script>").is_some());
    }
}
