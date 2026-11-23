use crate::dashboard_service::{DashboardService, host_config};
use dashboard_core::QueryError;
use std::sync::Arc;
use tauri::{State, WebviewWindow};
use viewer::game_windows::{GameWindowRender, GameWindowRequest, Service};

#[derive(Default)]
pub struct GameWindows(Arc<Service>);

impl GameWindows {
    pub fn evict_source(&self, source_id: &str) {
        self.0.evict_source(source_id);
    }

    pub fn clear_cache(&self) {
        self.0.clear_cache();
    }
}

#[tauri::command]
pub async fn render_game_window(
    window: WebviewWindow,
    service: State<'_, DashboardService>,
    renderer: State<'_, GameWindows>,
    request: GameWindowRequest,
) -> Result<GameWindowRender, QueryError> {
    let runtime = service
        .runtime(window.app_handle())
        .map_err(QueryError::from)?;
    let source = runtime
        .source(&request.source_id)
        .map_err(QueryError::from)?;
    if source.key().revision != request.revision {
        return Err(QueryError::from(
            "The recording changed while loading this game window.".to_owned(),
        ));
    }
    let visual_mods = runtime
        .map_settings(source.replay_ref())
        .visual_mods
        .unwrap_or_default();
    let renderer = Arc::clone(&renderer.0);
    tauri::async_runtime::spawn_blocking(move || {
        let (config, _) = host_config().map_err(QueryError::from)?;
        source
            .in_scope(|| renderer.render_from_source(&source, &config, &visual_mods, request))
            .map_err(QueryError::from)
    })
    .await
    .map_err(|error| QueryError::from(error.to_string()))?
}
