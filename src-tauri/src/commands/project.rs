//! Project commands — create/open a `.scast` project and mutate its contents.
//!
//! The currently-open project (its sqlx pool + folder + id) lives in
//! `ProjectState`, managed by Tauri. Every mutation persists immediately
//! through `store`, so there is no explicit "save". All commands are async.
//!
//! The real work and all the edge cases are unit-tested in `project::*`; these
//! handlers are the thin IPC layer that hold the open-project state and resolve
//! the recent-projects path.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex;

use crate::error::{AppError, AppResult};
use crate::project::model::{Marker, Project, ProjectSnapshot, RecentProject, Track};
use crate::project::templates::{self, TemplateInfo};
use crate::project::{self, scast, store};

/// The open project, or None when nothing is loaded.
struct OpenProject {
    pool: sqlx::SqlitePool,
    scast_dir: PathBuf,
    project_id: String,
}

/// Tauri-managed state: at most one open project at a time.
#[derive(Default)]
pub struct ProjectState {
    current: Mutex<Option<OpenProject>>,
}

fn config_dir(app: &AppHandle) -> AppResult<PathBuf> {
    app.path()
        .app_config_dir()
        .map_err(|e| AppError::Internal(format!("resolving config dir: {e}")))
}

/// Record a freshly opened/created project in the recent list.
fn record_recent(app: &AppHandle, name: &str, scast_dir: &Path) -> AppResult<()> {
    let entry = RecentProject {
        name: name.to_string(),
        path: scast_dir.to_string_lossy().into_owned(),
        last_opened: store::now_ms(),
    };
    scast::push_recent(&config_dir(app)?, entry)
}

/// Create a new project at `path` (a `.scast` folder) and make it current.
#[tauri::command]
pub async fn project_create(
    app: AppHandle,
    state: State<'_, ProjectState>,
    path: String,
    name: String,
    sample_rate: i32,
    channel_count: i32,
) -> AppResult<ProjectSnapshot> {
    let scast_dir = PathBuf::from(&path);
    let (pool, project) = project::create(&scast_dir, &name, sample_rate, channel_count).await?;
    let snapshot = store::load_snapshot(&pool).await?;
    record_recent(&app, &project.name, &scast_dir)?;
    *state.current.lock().await = Some(OpenProject {
        pool,
        scast_dir,
        project_id: project.id,
    });
    Ok(snapshot)
}

/// The quick-start templates for the gallery.
#[tauri::command]
pub async fn project_templates() -> AppResult<Vec<TemplateInfo>> {
    Ok(templates::all())
}

/// Create a project pre-configured from a quick-start template.
#[tauri::command]
pub async fn project_create_from_template(
    app: AppHandle,
    state: State<'_, ProjectState>,
    path: String,
    name: String,
    template_id: String,
) -> AppResult<ProjectSnapshot> {
    let channel_count = templates::channel_count(&template_id)
        .ok_or_else(|| AppError::Validation(format!("unknown template: {template_id}")))?;
    let scast_dir = PathBuf::from(&path);
    let (pool, project) = project::create(&scast_dir, &name, 48_000, channel_count).await?;
    templates::apply(&pool, &project.id, &template_id).await?;
    let snapshot = store::load_snapshot(&pool).await?;
    record_recent(&app, &project.name, &scast_dir)?;
    *state.current.lock().await = Some(OpenProject {
        pool,
        scast_dir,
        project_id: project.id,
    });
    Ok(snapshot)
}

/// Open an existing project and make it current.
#[tauri::command]
pub async fn project_open(
    app: AppHandle,
    state: State<'_, ProjectState>,
    path: String,
) -> AppResult<ProjectSnapshot> {
    let scast_dir = PathBuf::from(&path);
    let (pool, snapshot) = project::open(&scast_dir).await?;
    record_recent(&app, &snapshot.project.name, &scast_dir)?;
    *state.current.lock().await = Some(OpenProject {
        pool,
        scast_dir,
        project_id: snapshot.project.id.clone(),
    });
    Ok(snapshot)
}

/// The recent-projects list (most-recent first).
#[tauri::command]
pub async fn project_recent(app: AppHandle) -> AppResult<Vec<RecentProject>> {
    Ok(scast::load_recent(&config_dir(&app)?))
}

/// Reload the current project's snapshot (project + tracks + markers).
#[tauri::command]
pub async fn project_snapshot(state: State<'_, ProjectState>) -> AppResult<ProjectSnapshot> {
    let guard = state.current.lock().await;
    let op = current(&guard)?;
    store::load_snapshot(&op.pool).await
}

#[tauri::command]
pub async fn project_rename(state: State<'_, ProjectState>, name: String) -> AppResult<Project> {
    let guard = state.current.lock().await;
    let op = current(&guard)?;
    store::rename_project(&op.pool, &op.project_id, &name).await?;
    store::load_project(&op.pool).await
}

/// Back up the current project's database (hourly from the UI). Returns the
/// backup file path.
#[tauri::command]
pub async fn project_backup(state: State<'_, ProjectState>) -> AppResult<String> {
    let guard = state.current.lock().await;
    let op = current(&guard)?;
    let path = scast::backup_db(&op.scast_dir)?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn track_add(
    state: State<'_, ProjectState>,
    name: String,
    color: String,
) -> AppResult<Track> {
    let guard = state.current.lock().await;
    let op = current(&guard)?;
    store::add_track(&op.pool, &op.project_id, &name, &color).await
}

#[tauri::command]
pub async fn track_update(state: State<'_, ProjectState>, track: Track) -> AppResult<()> {
    let guard = state.current.lock().await;
    let op = current(&guard)?;
    store::update_track(&op.pool, &track).await
}

#[tauri::command]
pub async fn track_delete(state: State<'_, ProjectState>, id: String) -> AppResult<()> {
    let guard = state.current.lock().await;
    let op = current(&guard)?;
    store::delete_track(&op.pool, &id).await
}

#[tauri::command]
pub async fn marker_add(
    state: State<'_, ProjectState>,
    position_ms: f64,
    label: String,
    color: String,
) -> AppResult<Marker> {
    let guard = state.current.lock().await;
    let op = current(&guard)?;
    store::add_marker(&op.pool, &op.project_id, position_ms, &label, &color).await
}

#[tauri::command]
pub async fn marker_delete(state: State<'_, ProjectState>, id: String) -> AppResult<()> {
    let guard = state.current.lock().await;
    let op = current(&guard)?;
    store::delete_marker(&op.pool, &id).await
}

/// Borrow the open project or return a clear "no project open" error.
fn current(guard: &Option<OpenProject>) -> AppResult<&OpenProject> {
    guard
        .as_ref()
        .ok_or_else(|| AppError::Validation("no project open".into()))
}
