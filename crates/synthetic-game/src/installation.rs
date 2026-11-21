use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

pub struct Installation {
    name: String,
    game: Vec<(String, Vec<u8>)>,
    executable: Vec<u8>,
    language: String,
    dlcs: Vec<Dlc>,
    mods: Vec<Mod>,
    user: Vec<(String, Vec<u8>)>,
}

pub struct Dlc {
    folder: String,
    name: String,
    files: Vec<(String, Vec<u8>)>,
    enabled: bool,
}

pub struct Mod {
    name: String,
    folder: String,
    remote_id: Option<u64>,
    dependencies: Vec<String>,
    replace_paths: Vec<String>,
    files: Vec<(String, Vec<u8>)>,
    enabled: bool,
}

pub struct Built {
    root: PathBuf,
    pub game_dir: PathBuf,
    pub user_dir: PathBuf,
    pub cache_dir: PathBuf,
}

fn localisation(language: &str, file: &str, entries: &[(&str, &str)]) -> (String, Vec<u8>) {
    let short = language.strip_prefix("l_").unwrap_or(language);
    let mut text = format!("\u{feff}{language}:\n");
    for (key, value) in entries {
        writeln!(text, " {key}:0 \"{value}\"").expect("writing to a String cannot fail");
    }
    (
        format!("localisation/{short}/{file}_{language}.yml"),
        text.into_bytes(),
    )
}

impl Installation {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            game: Vec::new(),
            executable: b"synthetic executable".to_vec(),
            language: "l_english".to_owned(),
            dlcs: Vec::new(),
            mods: Vec::new(),
            user: Vec::new(),
        }
    }

    pub fn file(mut self, path: &str, contents: impl AsRef<[u8]>) -> Self {
        self.game.push((path.to_owned(), contents.as_ref().to_vec()));
        self
    }

    pub fn localisation(mut self, language: &str, file: &str, entries: &[(&str, &str)]) -> Self {
        self.game.push(localisation(language, file, entries));
        self
    }

    pub fn executable(mut self, bytes: impl AsRef<[u8]>) -> Self {
        self.executable = bytes.as_ref().to_vec();
        self
    }

    pub fn language(mut self, language: &str) -> Self {
        language.clone_into(&mut self.language);
        self
    }

    pub fn dlc(mut self, dlc: Dlc) -> Self {
        self.dlcs.push(dlc);
        self
    }

    pub fn with_mod(mut self, layer: Mod) -> Self {
        self.mods.push(layer);
        self
    }

    pub fn user_file(mut self, path: &str, contents: impl AsRef<[u8]>) -> Self {
        self.user.push((path.to_owned(), contents.as_ref().to_vec()));
        self
    }

    pub fn build(self) -> Built {
        static BUILT: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "synthetic-game-{}-{}-{}",
            self.name,
            std::process::id(),
            BUILT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        let built = Built {
            game_dir: root.join("game"),
            user_dir: root.join("user"),
            cache_dir: root.join("cache"),
            root,
        };
        let game = &built.game_dir;
        let user = &built.user_dir;
        write(game, "hoi4.exe", &self.executable);
        for (path, contents) in &self.game {
            write(game, path, contents);
        }
        let mut disabled_dlcs = Vec::new();
        for dlc in &self.dlcs {
            let folder = format!("dlc/{}", dlc.folder);
            let descriptor = format!("{}/{}.dlc", folder, dlc.folder);
            write(
                game,
                &descriptor,
                format!("name=\"{}\" path=\"{folder}\"", dlc.name).as_bytes(),
            );
            for (path, contents) in &dlc.files {
                write(game, &format!("{folder}/{path}"), contents);
            }
            if !dlc.enabled {
                disabled_dlcs.push(quoted(&descriptor));
            }
        }
        let mut enabled_mods = Vec::new();
        for layer in &self.mods {
            let descriptor = format!("mod/{}.mod", layer.folder);
            let mut text = format!("name=\"{}\" path=\"layers/{}\"", layer.name, layer.folder);
            if let Some(id) = layer.remote_id {
                write!(text, " remote_file_id={id}").expect("writing to a String cannot fail");
            }
            if !layer.dependencies.is_empty() {
                let names: Vec<_> = layer.dependencies.iter().map(|name| quoted(name)).collect();
                write!(text, " dependencies={{{}}}", names.join(" "))
                    .expect("writing to a String cannot fail");
            }
            for path in &layer.replace_paths {
                write!(text, " replace_path=\"{path}\"").expect("writing to a String cannot fail");
            }
            write(user, &descriptor, text.as_bytes());
            for (path, contents) in &layer.files {
                write(user, &format!("layers/{}/{path}", layer.folder), contents);
            }
            if layer.enabled {
                enabled_mods.push(quoted(&descriptor));
            }
        }
        write(
            user,
            "settings.txt",
            format!("language={}", self.language).as_bytes(),
        );
        write(
            user,
            "dlc_load.json",
            format!(
                "{{\"enabled_mods\":[{}],\"disabled_dlcs\":[{}]}}",
                enabled_mods.join(","),
                disabled_dlcs.join(",")
            )
            .as_bytes(),
        );
        for (path, contents) in &self.user {
            write(user, path, contents);
        }
        std::fs::create_dir_all(&built.cache_dir).expect("synthetic cache folder");
        built
    }
}

fn quoted(text: &str) -> String {
    format!("\"{text}\"")
}

fn write(base: &Path, path: &str, contents: &[u8]) {
    let path = base.join(path);
    std::fs::create_dir_all(path.parent().expect("a file in the synthetic game has a folder"))
        .expect("synthetic game folder");
    std::fs::write(&path, contents).expect("synthetic game file");
}

impl Dlc {
    pub fn new(folder: &str, name: &str) -> Self {
        Self {
            folder: folder.to_owned(),
            name: name.to_owned(),
            files: Vec::new(),
            enabled: true,
        }
    }

    pub fn file(mut self, path: &str, contents: impl AsRef<[u8]>) -> Self {
        self.files.push((path.to_owned(), contents.as_ref().to_vec()));
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

impl Mod {
    pub fn new(folder: &str, name: &str) -> Self {
        Self {
            name: name.to_owned(),
            folder: folder.to_owned(),
            remote_id: None,
            dependencies: Vec::new(),
            replace_paths: Vec::new(),
            files: Vec::new(),
            enabled: true,
        }
    }

    pub fn remote_id(mut self, id: u64) -> Self {
        self.remote_id = Some(id);
        self
    }

    pub fn depends_on(mut self, name: &str) -> Self {
        self.dependencies.push(name.to_owned());
        self
    }

    pub fn replaces(mut self, path: &str) -> Self {
        self.replace_paths.push(path.to_owned());
        self
    }

    pub fn file(mut self, path: &str, contents: impl AsRef<[u8]>) -> Self {
        self.files.push((path.to_owned(), contents.as_ref().to_vec()));
        self
    }

    pub fn localisation(mut self, language: &str, file: &str, entries: &[(&str, &str)]) -> Self {
        self.files.push(localisation(language, file, entries));
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

impl Built {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn mod_folder(&self, folder: &str) -> PathBuf {
        self.user_dir.join("layers").join(folder)
    }
}

impl Drop for Built {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
