//! Development-only stdio adapter for the browser UI, plus an explicit native
//! folder-picker mode. No renderer or live game access.

use dashboard::{
    Config, Service,
    protocol::{MAX_MESSAGE_BYTES, replay_revision},
    transport::{DataCommand, DataEvent, ReplyChunks},
};
use serde::Deserialize;
use std::{
    io::{self, BufRead, Read, Write},
    path::PathBuf,
    sync::Arc,
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum FolderCommand {
    SetContentFolders {
        id: String,
        game: PathBuf,
        user: PathBuf,
    },
    RecordingSummaries {
        id: String,
        paths: Vec<PathBuf>,
    },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Command {
    Folders(FolderCommand),
    Data(DataCommand),
}

fn emit(event: &DataEvent) -> bool {
    let mut output = io::stdout().lock();
    serde_json::to_writer(&mut output, event).is_ok()
        && output.write_all(b"\n").is_ok()
        && output.flush().is_ok()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments
        .first()
        .is_some_and(|arg| arg == "--choose-folder")
    {
        let title = match arguments.get(1).map(String::as_str) {
            Some("game") => "Choose your game installation",
            Some("user") => "Choose your Paradox user folder",
            Some("replay") => "Choose your replay folder",
            _ => return Err("Choose a game, user or replay folder".into()),
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
    let mut args = arguments.into_iter();
    while let Some(option) = args.next() {
        match option.as_str() {
            "--game-dir" => {
                config.game_dir = PathBuf::from(args.next().ok_or("Missing game folder")?)
            }
            "--user-dir" => {
                config.user_dir = PathBuf::from(args.next().ok_or("Missing user folder")?)
            }
            _ => return Err(format!("Unknown bridge option: {option}").into()),
        }
    }
    let service = Service::new(Arc::new(|event| emit(&event)))?;
    service.configure_content(&config, false);
    let mut input = io::stdin().lock();
    loop {
        let mut line = Vec::new();
        let read = input
            .by_ref()
            .take((MAX_MESSAGE_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)?;
        if read == 0 {
            break;
        }
        if read > MAX_MESSAGE_BYTES {
            return Err("Bridge command exceeds its byte budget".into());
        }
        let mut command = match serde_json::from_slice(&line)? {
            Command::Data(command) => command,
            Command::Folders(FolderCommand::RecordingSummaries { id, paths }) => {
                let partials: Vec<_> = paths
                    .iter()
                    .map(|path| {
                        dashboard::metadata::read_partial_recording(path)
                            .ok()
                            .flatten()
                    })
                    .collect();
                let payload = serde_json::to_string(&partials)?;
                for event in ReplyChunks::new(id, payload).map_err(invalid)? {
                    if !emit(&event) {
                        return Err("Browser connection closed".into());
                    }
                }
                continue;
            }
            Command::Folders(FolderCommand::SetContentFolders { id, game, user }) => {
                let next = Config {
                    game_dir: game,
                    user_dir: user,
                    ..config.clone()
                };
                match next.save() {
                    Ok(()) => {
                        config = next;
                        service.configure_content(&config, false);
                        let payload = serde_json::json!({ "game": config.game_dir, "user": config.user_dir, "contentReady": true }).to_string();
                        for event in ReplyChunks::new(id, payload).map_err(invalid)? {
                            if !emit(&event) {
                                return Err("Browser connection closed".into());
                            }
                        }
                    }
                    Err(error) => {
                        if !emit(&DataEvent::Error {
                            id,
                            message: error.to_string(),
                        }) {
                            return Err("Browser connection closed".into());
                        }
                    }
                }
                continue;
            }
        };
        if let DataCommand::Open { path, revision, .. } = &mut command {
            *revision = replay_revision(path)?;
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
    fn local_folder_commands_do_not_shadow_shared_data_commands() {
        let command = serde_json::from_value::<Command>(serde_json::json!({
            "type": "setContentFolders", "id": "folders-1", "game": "C:/Game", "user": "C:/User"
        }))
        .unwrap();
        assert!(
            matches!(command, Command::Folders(FolderCommand::SetContentFolders { id, game, user })
            if id == "folders-1" && game == std::path::Path::new("C:/Game") && user == std::path::Path::new("C:/User"))
        );
        let command = serde_json::from_value::<Command>(serde_json::json!({
            "type": "mapSettings", "id": "settings-1", "target": {"scope": "global"},
            "sourceId": "", "revision": "", "settings": null
        }))
        .unwrap();
        assert!(matches!(
            command,
            Command::Data(DataCommand::MapSettings { .. })
        ));
        for value in [
            serde_json::json!({"type": "setContentFolders", "id": "1", "game": "C:/Game"}),
            serde_json::json!({"type": "setContentFolders", "id": "1", "game": "C:/Game", "user": null}),
            serde_json::json!({"type": "setContentFolders", "id": "1", "game": "C:/Game", "user": "C:/User", "extra": true}),
        ] {
            assert!(serde_json::from_value::<Command>(value).is_err());
        }
    }
}
