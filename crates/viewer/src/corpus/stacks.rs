
use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
};

use content_reader_parser::parser;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub id: String,
    pub name: String,
    pub file: String,
    pub descriptor: String,
    pub dependencies: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub id: String,
    pub name: String,
    pub mods: Vec<usize>,
    pub skipped: Vec<String>,
}

pub struct Unusable {
    pub path: String,
    pub why: String,
}

impl Unusable {
    fn new(path: &Path, why: io::Error) -> Self {
        Self {
            path: path.display().to_string(),
            why: why.to_string(),
        }
    }
}

pub fn candidates(
    user: &Path,
    workshop: Option<&Path>,
) -> io::Result<(Vec<Candidate>, Vec<Unusable>)> {
    let mut found = Vec::new();
    let mut refused = Vec::new();
    let mut items = BTreeSet::new();
    if let Some(workshop) = workshop {
        for entry in sorted_entries(workshop)? {
            let folder = entry.path();
            let descriptor = folder.join("descriptor.mod");
            if !entry.file_type()?.is_dir() || !descriptor.is_file() {
                continue;
            }
            let id = entry.file_name().to_string_lossy().into_owned();
            match launcher_descriptor(&descriptor, &folder, Some(&id)) {
                Ok((text, name, dependencies)) => {
                    items.insert(normal(&folder));
                    found.push(Candidate {
                        file: format!("mod/ugc_{id}.mod"),
                        id,
                        name,
                        descriptor: text,
                        dependencies,
                    });
                }
                Err(error) => refused.push(Unusable::new(&descriptor, error)),
            }
        }
    }
    let directory = user.join("mod");
    if directory.is_dir() {
        for entry in sorted_entries(&directory)? {
            let path = entry.path();
            if !entry.file_type()?.is_file()
                || path.extension().is_none_or(|extension| extension != "mod")
            {
                continue;
            }
            let result = (|| {
                let document = parser::parse(&fs::read(&path)?)?;
                let folder = document
                    .get(b"path")
                    .and_then(|value| value.scalar_str())
                    .ok_or_else(|| invalid("declares no folder path (an archive or none)"))?;
                let folder = if Path::new(folder).is_absolute() {
                    PathBuf::from(folder)
                } else {
                    user.join(folder)
                };
                if items.contains(&normal(&folder)) {
                    return Ok(None);
                }
                if !folder.is_dir() {
                    return Err(invalid(format!(
                        "its folder {} is not installed",
                        folder.display()
                    )));
                }
                launcher_descriptor(&path, &folder, None).map(Some)
            })();
            let stem = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
            match result {
                Ok(None) => {}
                Ok(Some((text, name, dependencies))) => found.push(Candidate {
                    id: format!("local-{stem}"),
                    name,
                    file: format!("mod/{}", entry.file_name().to_string_lossy()),
                    descriptor: text,
                    dependencies,
                }),
                Err(error) => refused.push(Unusable::new(&path, error)),
            }
        }
    }
    Ok((found, refused))
}

pub fn plans(candidates: &[Candidate]) -> Vec<Plan> {
    candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let mut mods = vec![index];
            let mut skipped = Vec::new();
            let mut pending = vec![index];
            while let Some(current) = pending.pop() {
                for dependency in &candidates[current].dependencies {
                    let named: Vec<usize> = (0..candidates.len())
                        .filter(|&other| candidates[other].name == *dependency)
                        .collect();
                    match named.as_slice() {
                        [] => skipped.push(format!("{dependency:?}: not installed")),
                        [one] if !mods.contains(one) => {
                            mods.push(*one);
                            pending.push(*one);
                        }
                        [_] => {}
                        several => skipped.push(format!(
                            "{dependency:?}: {} installed mods carry this name",
                            several.len()
                        )),
                    }
                }
            }
            Plan {
                id: candidate.id.clone(),
                name: candidate.name.clone(),
                mods,
                skipped,
            }
        })
        .collect()
}

pub fn user_folder(
    root: &Path,
    language: &str,
    candidates: &[Candidate],
    mods: &[usize],
) -> io::Result<PathBuf> {
    if root.exists() {
        fs::remove_dir_all(root)?;
    }
    fs::create_dir_all(root.join("mod"))?;
    fs::write(
        root.join("settings.txt"),
        format!("language=\"{language}\"\n"),
    )?;
    let mut enabled = Vec::new();
    for &index in mods {
        let candidate = &candidates[index];
        fs::write(root.join(&candidate.file), &candidate.descriptor)?;
        enabled.push(candidate.file.clone());
    }
    fs::write(
        root.join("dlc_load.json"),
        serde_json::to_vec(&serde_json::json!({
            "enabled_mods": enabled,
            "disabled_dlcs": [],
        }))?,
    )?;
    Ok(root.to_path_buf())
}

pub fn language(user: &Path) -> io::Result<String> {
    content_reader_settings::settings::read_keys(user)?
        .language()
        .ok_or_else(|| io::Error::other("settings.txt names no game language"))
}

fn launcher_descriptor(
    descriptor: &Path,
    folder: &Path,
    item: Option<&str>,
) -> io::Result<(String, String, Vec<String>)> {
    let document = parser::parse(&fs::read(descriptor)?)?;
    let name = document
        .get(b"name")
        .and_then(|value| value.scalar_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| invalid("names no mod"))?
        .to_owned();
    let mut dependencies = Vec::new();
    for value in document.get_all(b"dependencies") {
        for entry in value.items().unwrap_or_default() {
            if let (None, Some(name)) = (&entry.key, entry.value.scalar_str()) {
                dependencies.push(name.to_owned());
            }
        }
    }
    let folder = folder
        .to_str()
        .ok_or_else(|| invalid("folder path is not Unicode"))?
        .replace('\\', "/");
    let mut kept: Vec<parser::Item> = document
        .items
        .into_iter()
        .filter(|item| !matches!(item.key.as_deref(), Some(b"path" | b"archive")))
        .collect();
    let field = |key: &str, value: &str| parser::Item {
        key: Some(key.as_bytes().to_vec()),
        key_quoted: false,
        operator: parser::Operator::Equals,
        kind: parser::Kind::Quoted,
        value: parser::Value::Scalar(value.as_bytes().to_vec()),
    };
    kept.push(field("path", &folder));
    if let Some(id) = item.filter(|_| {
        !kept
            .iter()
            .any(|i| i.key.as_deref() == Some(b"remote_file_id"))
    }) {
        kept.push(field("remote_file_id", id));
    }
    let mut text = Vec::new();
    for item in &kept {
        render(item, &mut text);
        text.push(b'\n');
    }
    let fields = |items: &[parser::Item]| -> Vec<(Option<Vec<u8>>, parser::Value)> {
        items
            .iter()
            .map(|item| (item.key.clone(), item.value.clone()))
            .collect()
    };
    if fields(&parser::parse(&text)?.items) != fields(&kept) {
        return Err(invalid("cannot be rewritten to the same fields"));
    }
    let text = String::from_utf8(text).map_err(|_| invalid("is not UTF-8"))?;
    Ok((text, name, dependencies))
}

fn render(item: &parser::Item, out: &mut Vec<u8>) {
    let quoted = |bytes: &[u8], out: &mut Vec<u8>| {
        out.push(b'"');
        for &byte in bytes {
            if matches!(byte, b'"' | b'\\') {
                out.push(b'\\');
            }
            out.push(byte);
        }
        out.push(b'"');
    };
    if let Some(key) = &item.key {
        if item.key_quoted {
            quoted(key, out);
        } else {
            out.extend_from_slice(key);
        }
        out.extend_from_slice(item.operator.text());
    }
    let block = |items: &[parser::Item], out: &mut Vec<u8>| {
        out.extend_from_slice(b"{ ");
        for item in items {
            render(item, out);
            out.push(b' ');
        }
        out.push(b'}');
    };
    match &item.value {
        parser::Value::Scalar(bytes) if item.kind == parser::Kind::Quoted => quoted(bytes, out),
        parser::Value::Scalar(bytes) => out.extend_from_slice(bytes),
        parser::Value::Block(items) => block(items, out),
        parser::Value::Tagged { tag, items } => {
            out.extend_from_slice(tag);
            out.push(b' ');
            block(items, out);
        }
    }
}

fn normal(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

fn sorted_entries(path: &Path) -> io::Result<Vec<fs::DirEntry>> {
    let mut entries = fs::read_dir(path)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workshop_items_become_launcher_descriptors_with_their_dependencies() {
        let root = std::env::temp_dir().join(format!(
            "hoi4_viewer_corpus_stacks_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "workshop/10/descriptor.mod",
            "name=\"Base\" path=\"C:/old\" replace_path=\"common/ideas\"\ntags={\n\t\"A \\\"B\\\"\"\n}\npath = \"C:/second\"\n",
        );
        write(
            "workshop/20/descriptor.mod",
            "name=\"Sub\"\ndependencies={Base \"Absent\"}\ntags={ Balance \"Map\" }\nremote_file_id=\"20\"\n",
        );
        let item = root
            .join("workshop/10")
            .to_string_lossy()
            .replace('\\', "/");
        write(
            "user/mod/ugc_10.mod",
            &format!("name=\"Base\"\npath=\"{item}\"\n"),
        );
        fs::create_dir_all(root.join("user/mod/local")).unwrap();
        write("user/mod/local.mod", "name=\"Local\"\npath=\"mod/local\"\n");
        let (candidates, refused) =
            candidates(&root.join("user"), Some(&root.join("workshop"))).unwrap();
        assert!(refused.is_empty());
        let ids: Vec<_> = candidates.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["10", "20", "local-local"]);
        assert_eq!(
            candidates[0].descriptor,
            format!(
                "name=\"Base\"\nreplace_path=\"common/ideas\"\ntags={{ \"A \\\"B\\\"\" }}\npath=\"{item}\"\nremote_file_id=\"10\"\n"
            )
        );
        let rewritten = parser::parse(candidates[0].descriptor.as_bytes()).unwrap();
        assert_eq!(
            rewritten.get(b"path").unwrap().scalar_str(),
            Some(item.as_str())
        );
        assert_eq!(
            rewritten.get(b"tags").unwrap().items().unwrap()[0]
                .value
                .scalar(),
            Some(&b"A \"B\""[..])
        );
        assert!(
            !candidates[1]
                .descriptor
                .contains("remote_file_id=\"20\"\nremote_file_id")
        );
        assert!(candidates[1].descriptor.starts_with(
            "name=\"Sub\"\ndependencies={ Base \"Absent\" }\ntags={ Balance \"Map\" }\n"
        ));
        assert!(candidates[2].descriptor.contains("/user/mod/local\""));
        let plans = plans(&candidates);
        assert_eq!(plans[1].mods, [1, 0]);
        assert_eq!(plans[1].skipped, ["\"Absent\": not installed"]);
        let _ = fs::remove_dir_all(&root);
    }
}
