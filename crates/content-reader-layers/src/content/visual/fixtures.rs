use super::*;

pub struct Machine {
    pub root: PathBuf,
    pub options: ResolveOptions,
}

impl Machine {
    pub fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "hoi4_visual_{name}_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        let game_dir = root.join("steamapps/common/Game");
        let user_dir = root.join("user");
        fs::create_dir_all(&game_dir).unwrap();
        fs::create_dir_all(user_dir.join("mod")).unwrap();
        fs::create_dir_all(
            root.join("steamapps/workshop/content")
                .join(WORKSHOP_APP_ID),
        )
        .unwrap();
        Self {
            options: ResolveOptions {
                game_dir,
                user_dir,
                cache_dir: root.join("cache"),
            },
            root,
        }
    }

    pub fn write(&self, relative: &str, bytes: &[u8]) -> PathBuf {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }

    pub(super) fn folder(&self, relative: &str) -> String {
        self.root
            .join(relative)
            .display()
            .to_string()
            .replace('\\', "/")
    }

    pub fn local(&self, file: &str, body: &str, folder: &str) {
        fs::create_dir_all(self.root.join(folder)).unwrap();
        self.write(
            &format!("user/mod/{file}"),
            format!("{body}\npath=\"{}\"\n", self.folder(folder)).as_bytes(),
        );
    }

    pub fn item(&self, item: &str, descriptor: &str) -> String {
        let folder = format!("steamapps/workshop/content/{WORKSHOP_APP_ID}/{item}");
        self.write(&format!("{folder}/descriptor.mod"), descriptor.as_bytes());
        folder
    }

    pub fn recorded(&self, folders: &[&str]) -> ResolvedLayers {
        let paths: Vec<PathBuf> = folders
            .iter()
            .map(|folder| self.root.join(folder))
            .collect();
        for path in &paths {
            fs::create_dir_all(path).unwrap();
        }
        let folders: Vec<(&Path, &[&str])> =
            paths.iter().map(|path| (path.as_path(), &[][..])).collect();
        ResolvedLayers::from_folders(&folders).unwrap()
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub fn pick(id: &str, name: &str) -> Choice {
    Choice {
        id: id.into(),
        name: name.into(),
    }
}

pub fn never() -> AtomicBool {
    AtomicBool::new(false)
}
