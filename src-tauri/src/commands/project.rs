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
use crate::project::registry::{self as reg, ProjectMeta};
use crate::project::templates::{self, TemplateInfo};
use crate::project::{self, scast, store};

/// The open project, or None when nothing is loaded.
pub(crate) struct OpenProject {
    pub(crate) pool: sqlx::SqlitePool,
    pub(crate) scast_dir: PathBuf,
    pub(crate) project_id: String,
}

/// Tauri-managed state: at most one open project at a time.
#[derive(Default)]
pub struct ProjectState {
    pub(crate) current: Mutex<Option<OpenProject>>,
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

// ── Phase 2.1: registry-level CRUD ───────────────────────────────────────────
//
// These five commands form the "New / Save / Load / List / Delete" surface that
// lets the UI manage projects without a file dialog (useful for programmatic
// project creation, testing, and the home screen).
//
// They delegate to `project::registry` (metadata index) and `project::*`
// (per-project SQLite + folder). The registry stores a lightweight `ProjectMeta`
// for each project so `project_list` never has to open every `.scast` file.

/// Create a brand-new project in `<data_dir>/projects/<name>.scast` and
/// register it. Returns the created project's metadata entry.
///
/// This is the no-dialog companion to `project_create_from_template`: useful
/// for programmatic creation (onboarding, tests, deep links).
#[tauri::command]
pub async fn project_new(
    app: AppHandle,
    state: State<'_, ProjectState>,
    name: String,
) -> AppResult<ProjectMeta> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::Validation("project name cannot be empty".into()));
    }
    // Place new projects in `<app_data_dir>/projects/`.
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Internal(format!("resolving data dir: {e}")))?
        .join("projects");

    // Sanitise the folder name (replace file-system-unsafe chars with '_').
    let safe: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == ' ' { c } else { '_' })
        .collect();
    let scast_dir = data_dir.join(format!("{safe}.scast"));

    let (pool, project) = project::create(&scast_dir, &name, 48_000, 2).await?;

    // Register in the app-level index.
    let cfg = config_dir(&app)?;
    let meta = reg::register(&cfg, &project.name, &scast_dir)?;

    // Also add to the recent list so `project_recent` stays consistent.
    record_recent(&app, &project.name, &scast_dir)?;

    // Make it the current open project.
    *state.current.lock().await = Some(OpenProject {
        pool,
        scast_dir,
        project_id: project.id,
    });

    Ok(meta)
}

/// Persist the current project's mutable metadata (name) to the registry.
/// The per-project SQLite is already auto-saved; this command updates the
/// registry entry so `project_list` reflects the latest name.
///
/// Mirrors the "save" intent: call this after a rename to keep the registry
/// in sync with the project's internal state.
#[tauri::command]
pub async fn project_save(
    app: AppHandle,
    state: State<'_, ProjectState>,
    registry_id: String,
    name: String,
) -> AppResult<ProjectMeta> {
    // Rename inside the project's own database first.
    {
        let guard = state.current.lock().await;
        let op = current(&guard)?;
        store::rename_project(&op.pool, &op.project_id, &name).await?;
    }
    // Then update the registry entry.
    let cfg = config_dir(&app)?;
    reg::update_meta(&cfg, &registry_id, &name)
}

/// Load a project from the registry by its registry id and make it current.
/// Returns a `ProjectMeta` so the caller knows the resolved path and name;
/// use `project_snapshot` afterwards to get the full project + tracks.
#[tauri::command]
pub async fn project_load(
    app: AppHandle,
    state: State<'_, ProjectState>,
    registry_id: String,
) -> AppResult<ProjectMeta> {
    let cfg = config_dir(&app)?;
    let meta = reg::find(&cfg, &registry_id)?;
    let scast_dir = PathBuf::from(&meta.path);
    let (pool, snapshot) = project::open(&scast_dir).await?;
    record_recent(&app, &snapshot.project.name, &scast_dir)?;
    *state.current.lock().await = Some(OpenProject {
        pool,
        scast_dir,
        project_id: snapshot.project.id.clone(),
    });
    Ok(meta)
}

/// List all registered projects (most-recently created first).
/// This is fast: it reads only the registry JSON, never opens a `.scast` file.
#[tauri::command]
pub async fn project_list(app: AppHandle) -> AppResult<Vec<ProjectMeta>> {
    let cfg = config_dir(&app)?;
    Ok(reg::load_all(&cfg))
}

/// Remove a project from the registry by its registry id.
/// The `.scast` folder on disk is NOT deleted — the user keeps their audio.
#[tauri::command]
pub async fn project_delete(
    app: AppHandle,
    state: State<'_, ProjectState>,
    registry_id: String,
) -> AppResult<()> {
    let cfg = config_dir(&app)?;
    let meta = reg::find(&cfg, &registry_id)?;

    // If this project is currently open, close it.
    {
        let mut guard = state.current.lock().await;
        if let Some(op) = guard.as_ref() {
            if op.scast_dir == Path::new(&meta.path) {
                guard.take();
            }
        }
    }

    reg::remove(&cfg, &registry_id)
}

/// Borrow the open project or return a clear "no project open" error.
pub(crate) fn current(guard: &Option<OpenProject>) -> AppResult<&OpenProject> {
    guard
        .as_ref()
        .ok_or_else(|| AppError::Validation("no project open".into()))
}
