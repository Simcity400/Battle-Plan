
use base64::Engine;
use dashboard_core::{config::Config, protocol::{MAX_MESSAGE_BYTES, replay_revision}, transport::{DataCommand, DataEvent, ReplyChunks}};
use dashboard::Service;
use serde::Deserialize;
use std::{
    io::{self, BufRead, Read, Write},
    path::PathBuf,
    sync::Arc,
};

#[global_allocator]
static ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum LocalCommand {
    SetContentFolders {
        id: String,
        game: PathBuf,
        user: PathBuf,
    },
    RecordingSummaries {
        id: String,
        paths: Vec<PathBuf>,
    },
    Artwork {
        id: String,
        name: String,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Command {
    Local(LocalCommand),
    Data(DataCommand),
}

fn read_command(input: &mut impl BufRead) -> io::Result<Option<Command>> {
    let mut line = Vec::new();
    let read = input
        .take((MAX_MESSAGE_BYTES + 1) as u64)
        .read_until(b'\n', &mut line)?;
    if read == 0 {
        return Ok(None);
    }
    if read > MAX_MESSAGE_BYTES {
        return Err(invalid("Bridge command exceeds its byte budget"));
    }
    serde_json::from_slice(&line).map(Some).map_err(Into::into)
}

fn emit(event: &DataEvent) -> bool {
    let mut output = io::stdout().lock();
    serde_json::to_writer(&mut output, event).is_ok()
        && output.write_all(b"\n").is_ok()
        && output.flush().is_ok()
}

fn answer(id: String, payload: String) -> Result<(), Box<dyn std::error::Error>> {
    for event in ReplyChunks::new(id, payload).map_err(invalid)? {
        if !emit(&event) {
            return Err("Browser connection closed".into());
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    engine_heap::return_freed_memory_promptly();
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments
        .first()
        .is_some_and(|arg| arg == "--choose-folder")
    {
        let title = match arguments.get(1).map(String::as_str) {
            Some("game") => "Choose your game installation",
            Some("user") => "Choose your Paradox user folder",
            Some("replay") => "Choose your recordings folder",
            _ => return Err("Choose a game, user or recordings folder".into()),
        };
        if arguments.len() > 3 {
            return Err("Unexpected folder picker arguments".into());
        }
        let mut dialog = rfd::FileDialog::new().set_title(title);
        if let Some(directory) = arguments
            .get(2)
            .and_then(|value| std::fs::canonicalize(value).ok())
            .filter(|path| path.is_dir())
        {
            dialog = dialog.set_directory(directory);
        }
        let selected = dialog.pick_folder();
        println!("{}", serde_json::to_string(&selected)?);
        return Ok(());
    }
    let mut config = Config::load()?;
    let mut artwork_url = None;
    let mut args = arguments.into_iter();
    while let Some(option) = args.next() {
        match option.as_str() {
            "--game-dir" => {
                config.game_dir = PathBuf::from(args.next().ok_or("Missing game folder")?)
            }
            "--user-dir" => {
                config.user_dir = PathBuf::from(args.next().ok_or("Missing user folder")?)
            }
            "--artwork-url" => artwork_url = Some(args.next().ok_or("Missing artwork URL")?),
            _ => return Err(format!("Unknown bridge option: {option}").into()),
        }
    }
    if let Err(error) = dashboard_text::artwork::configure(
        &artwork_url.ok_or("The bridge needs --artwork-url")?,
        dashboard_text::artwork::local_folder(),
    ) {
        eprintln!("Artwork: {error}");
    }
    let service = Service::new(Arc::new(|event| emit(&event)))?;
    if let Err(error) = service.configure_content(&config, false) {
        eprintln!("{error}");
    }
    let mut input = io::stdin().lock();
    while let Some(command) = read_command(&mut input)? {
        let mut command = match command {
            Command::Data(command) => command,
            Command::Local(LocalCommand::Artwork { id, name }) => {
                let png = dashboard_text::artwork::bytes(&name)
                    .map(|png| base64::engine::general_purpose::STANDARD.encode(png));
                answer(id, serde_json::to_string(&png)?)?;
                continue;
            }
            Command::Local(LocalCommand::RecordingSummaries { id, paths }) => {
                let partials: Vec<_> = paths
                    .iter()
                    .map(|path| {
                        dashboard_text::metadata::read_partial_recording(path)
                            .ok()
                            .flatten()
                    })
                    .collect();
                answer(id, serde_json::to_string(&partials)?)?;
                continue;
            }
            Command::Local(LocalCommand::SetContentFolders { id, game, user }) => {
                let next = Config {
                    game_dir: game,
                    user_dir: user,
                    ..config.clone()
                };
                match next
                    .save()
                    .and_then(|applied| service.configure_content(&next, false).map(|()| applied.warning))
                {
                    Ok(warning) => {
                        config = next;
                        let payload = serde_json::json!({ "game": config.game_dir, "user": config.user_dir, "contentReady": true, "warning": warning }).to_string();
                        answer(id, payload)?;
                    }
                    Err(error) => {
                        if !emit(&DataEvent::Error {
                            id,
                            error: error.into(),
                        }) {
                            return Err("Browser connection closed".into());
                        }
                    }
                }
                continue;
            }
        };
        if let DataCommand::Open { id, path, revision } = &mut command {
            match replay_revision(path) {
                Ok(current) => *revision = current,
                Err(error) => {
                    if !emit(&DataEvent::Error {
                        id: id.clone(),
                        error: error.into(),
                    }) {
                        return Err("Browser connection closed".into());
                    }
                    continue;
                }
            }
        }
        service.dispatch(command);
    }
    service.shutdown();
    Ok(())
}

#[cfg(test)]
mod folder_command_tests {
    use super::*;

    #[test]
    fn command_reader_accepts_all_5000_countries_and_refuses_past_the_byte_budget() {
        let tags: Vec<_> = (0..5000)
            .map(|index| format!("synthetic-country-{index:04}"))
            .collect();
        let command = serde_json::json!({
            "type": "product", "id": "request-1", "request": {
                "sourceId": "source-1", "revision": "1", "product": "agency-comparison",
                "params": { "branchId": "1", "tags": tags, "top": null }
            }
        });
        let mut line = serde_json::to_vec(&command).unwrap();
        line.push(b'\n');
        assert!(line.len() > 64 * 1024);
        let Some(Command::Data(DataCommand::Product { request, .. })) =
            read_command(&mut line.as_slice()).unwrap()
        else {
            panic!("wrong command");
        };
        assert_eq!(request.params["tags"], command["request"]["params"]["tags"]);
        line.pop();
        line.resize(MAX_MESSAGE_BYTES - 1, b' ');
        line.push(b'\n');
        assert!(read_command(&mut line.as_slice()).unwrap().is_some());
        line.insert(line.len() - 1, b' ');
        assert_eq!(
            read_command(&mut line.as_slice())
                .err()
                .unwrap()
                .to_string(),
            "Bridge command exceeds its byte budget"
        );
    }

    #[test]
    fn local_folder_commands_do_not_shadow_shared_data_commands() {
        let command = serde_json::from_value::<Command>(serde_json::json!({
            "type": "setContentFolders", "id": "folders-1", "game": "C:/Game", "user": "C:/User"
        }))
        .unwrap();
        assert!(
            matches!(command, Command::Local(LocalCommand::SetContentFolders { id, game, user })
            if id == "folders-1" && game == std::path::Path::new("C:/Game") && user == std::path::Path::new("C:/User"))
        );
        let command = serde_json::from_value::<Command>(serde_json::json!({
            "type": "mapSettings", "id": "settings-1", "target": {"scope": "global"},
            "sourceId": "", "revision": "", "settings": null, "flip": null
        }))
        .unwrap();
        assert!(matches!(
            command,
            Command::Data(DataCommand::MapSettings { .. })
        ));
        let command = serde_json::from_value::<Command>(serde_json::json!({
            "type": "artwork", "id": "art-1", "name": "synthetic.png"
        }))
        .unwrap();
        assert!(
            matches!(command, Command::Local(LocalCommand::Artwork { id, name })
            if id == "art-1" && name == "synthetic.png")
        );
        for value in [
            serde_json::json!({"type": "setContentFolders", "id": "1", "game": "C:/Game"}),
            serde_json::json!({"type": "setContentFolders", "id": "1", "game": "C:/Game", "user": null}),
            serde_json::json!({"type": "setContentFolders", "id": "1", "game": "C:/Game", "user": "C:/User", "extra": true}),
        ] {
            assert!(serde_json::from_value::<Command>(value).is_err());
        }
    }
}
