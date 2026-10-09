mod metrics;
mod movement_oracle;
mod oracle;
mod recording;
mod replay_io;
#[cfg(test)]
mod test_support;
mod windows;

fn main() -> std::process::ExitCode {
    match recording::command(std::env::args_os().skip(1).collect()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
