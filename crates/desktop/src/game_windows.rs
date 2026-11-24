use crate::dashboard_service::{DashboardService, host_config};
use dashboard_core::QueryError;
use std::sync::Arc;
use tauri::{Manager, State, WebviewWindow};
use viewer::game_windows::{GameWindowError, GameWindowRender, GameWindowRequest, Service};

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

fn map_window_error(error: GameWindowError) -> QueryError {
    match error {
        GameWindowError::Refused(text) => QueryError::Refused { text },
        GameWindowError::Failed(message) => QueryError::Failed { message },
        GameWindowError::Cancelled => QueryError::Failed {
            message: "Request cancelled".to_owned(),
        },
    }
}

#[tauri::command]
pub fn cancel_game_window(renderer: State<'_, GameWindows>, request_id: String) -> Result<(), String> {
    renderer.0.cancel_request(&request_id);
    Ok(())
}

#[tauri::command]
pub async fn render_game_window(
    window: WebviewWindow,
    service: State<'_, DashboardService>,
    renderer: State<'_, GameWindows>,
    request: GameWindowRequest,
    request_id: Option<String>,
) -> Result<GameWindowRender, QueryError> {
    let runtime = service
        .runtime(window.app_handle())
        .map_err(QueryError::from)?;
    let source = runtime
        .source(&request.source_id)
        .map_err(QueryError::from)?;
    if source.key().revision != request.revision {
        return Err(QueryError::Refused {
            text: "The recording changed while loading this game window.".to_owned(),
        });
    }
    let visual_mods = runtime
        .map_settings(source.replay_ref())
        .visual_mods
        .unwrap_or_default();
    let renderer = Arc::clone(&renderer.0);
    let cancel_id = request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let registration = renderer.register_cancel(cancel_id);
    tauri::async_runtime::spawn_blocking(move || {
        let (config, _) = host_config().map_err(QueryError::from)?;
        let result = source.in_scope(|| {
            renderer.render_from_source(
                &source,
                &config,
                &visual_mods,
                request,
                registration.token(),
            )
        });
        drop(registration);
        result.map_err(map_window_error)
    })
    .await
    .map_err(|error| QueryError::from(error.to_string()))?
}
