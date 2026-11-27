use crate::{Channel, RELEASES, UpdateState, Updates, install, updates};
use std::sync::Arc;
use tauri::AppHandle;

#[tauri::command]
pub async fn app_update_state(app: AppHandle) -> Result<UpdateState, String> {
    blocking(app, |updates| Ok(updates.state())).await
}

async fn blocking<T: Send + 'static>(
    app: AppHandle,
    work: impl FnOnce(Arc<Updates>) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || work(updates(&app)?))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn app_update_check(app: AppHandle) -> Result<UpdateState, String> {
    blocking(app, |updates| {
        tauri::async_runtime::block_on(updates.check())
    })
    .await
}

#[tauri::command]
pub async fn app_update_download(app: AppHandle) -> Result<UpdateState, String> {
    blocking(app, |updates| {
        tauri::async_runtime::block_on(updates.download())
    })
    .await
}

#[tauri::command]
pub async fn app_update_install(app: AppHandle) -> Result<(), String> {
    blocking(app, |updates| updates.install()).await
}

pub fn install_now(app: &AppHandle) -> Result<(), String> {
    updates(app)?.install()
}

#[tauri::command]
pub async fn app_update_set_channel(
    app: AppHandle,
    channel: Channel,
) -> Result<UpdateState, String> {
    blocking(app, move |updates| {
        updates.set_channel(channel)?;
        let _ = tauri::async_runtime::block_on(updates.check());
        Ok(updates.state())
    })
    .await
}

#[tauri::command]
pub fn app_update_open_release(version: String) -> Result<(), String> {
    let releases = RELEASES.ok_or("This build has no release page.")?;
    let version_like = |byte: u8| byte.is_ascii_alphanumeric() || b".-".contains(&byte);
    if version.is_empty() || !version.bytes().all(version_like) {
        return Err("That isn’t a release version.".into());
    }
    install::browse(&format!("{releases}/tag/v{version}"))
}
