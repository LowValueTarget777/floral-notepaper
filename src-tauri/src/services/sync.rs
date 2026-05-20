use crate::services::notes::{default_store, AppConfig, AppError, Note, NoteStore, SaveNoteRequest};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    sync::Arc,
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

const NOTES_CHANGED_EVENT: &str = "notes-changed";
pub const SYNC_STATUS_CHANGED_EVENT: &str = "sync-status-changed";
const WAIT_TIMEOUT_SECONDS: u64 = 25;

enum AutoSyncCommand {
    SyncNow,
    ConfigChanged,
}

pub struct AutoSyncController {
    sender: mpsc::UnboundedSender<AutoSyncCommand>,
    sync_lock: Mutex<()>,
}

impl AutoSyncController {
    fn new(sender: mpsc::UnboundedSender<AutoSyncCommand>) -> Self {
        Self {
            sender,
            sync_lock: Mutex::new(()),
        }
    }

    pub fn request_sync(&self) {
        self.send(AutoSyncCommand::SyncNow);
    }

    pub fn notify_config_changed(&self) {
        self.send(AutoSyncCommand::ConfigChanged);
    }

    fn send(&self, command: AutoSyncCommand) {
        let _ = self.sender.send(command);
    }
}

pub fn setup_auto_sync(app: &AppHandle) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let controller = Arc::new(AutoSyncController::new(sender));
    app.manage(controller.clone());
    request_startup_auto_sync(controller.as_ref());
    spawn_auto_sync_worker(app.clone(), controller, receiver);
}

fn request_startup_auto_sync(controller: &AutoSyncController) {
    controller.request_sync();
}

pub fn request_auto_sync(app: &AppHandle) {
    if let Some(controller) = app.try_state::<Arc<AutoSyncController>>() {
        controller.request_sync();
    }
}

pub fn notify_auto_sync_config_changed(app: &AppHandle) {
    if let Some(controller) = app.try_state::<Arc<AutoSyncController>>() {
        controller.notify_config_changed();
    }
}

pub async fn sync_now_for_app(app: &AppHandle) -> Result<SyncStatus, AppError> {
    if let Some(controller) = app.try_state::<Arc<AutoSyncController>>() {
        run_sync_operation(app, Some(controller.inner())).await
    } else {
        run_sync_operation(app, None).await
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub enabled: bool,
    pub configured: bool,
    pub last_revision: u64,
    pub last_sync_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SyncChange {
    pub id: String,
    pub title: String,
    pub content: String,
    pub category: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub content_hash: String,
    pub device_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct RevisionedChange {
    revision: u64,
    note: SyncChange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ChangesResponse {
    revision: u64,
    changes: Vec<RevisionedChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PushRequest {
    device_id: String,
    changes: Vec<SyncChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PushResponse {
    revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct WaitResponse {
    revision: u64,
    changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SyncedNoteState {
    pub content_hash: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TombstoneState {
    pub deleted_at: DateTime<Utc>,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SyncState {
    pub device_id: String,
    pub last_revision: u64,
    pub notes: HashMap<String, SyncedNoteState>,
    pub tombstones: HashMap<String, TombstoneState>,
    pub last_sync_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

impl Default for SyncState {
    fn default() -> Self {
        Self {
            device_id: Uuid::new_v4().to_string(),
            last_revision: 0,
            notes: HashMap::new(),
            tombstones: HashMap::new(),
            last_sync_at: None,
            last_error: None,
        }
    }
}

pub fn status(store: &NoteStore, config: &AppConfig) -> Result<SyncStatus, AppError> {
    let state = load_state(store)?;
    Ok(status_from_state(config, &state))
}

pub async fn test_connection(
    store: &NoteStore,
    config: &AppConfig,
) -> Result<SyncStatus, AppError> {
    let mut state = load_state(store)?;
    let result = request_health(config).await;
    match result {
        Ok(()) => {
            state.last_error = None;
            save_state(store, &state)?;
            Ok(status_from_state(config, &state))
        }
        Err(error) => {
            state.last_error = Some(error.message.clone());
            save_state(store, &state)?;
            Err(error)
        }
    }
}

pub async fn sync_now(store: &NoteStore, config: &AppConfig) -> Result<SyncStatus, AppError> {
    let mut state = load_state(store)?;
    if !is_configured(config) {
        state.last_error = Some("Sync server URL and token are required".into());
        save_state(store, &state)?;
        return Ok(status_from_state(config, &state));
    }

    match sync_inner(store, config, &mut state).await {
        Ok(()) => {
            state.last_sync_at = Some(Utc::now());
            state.last_error = None;
            rebuild_note_state(store, &mut state)?;
            save_state(store, &state)?;
            Ok(status_from_state(config, &state))
        }
        Err(error) => {
            state.last_error = Some(error.message.clone());
            save_state(store, &state)?;
            Err(error)
        }
    }
}

fn spawn_auto_sync_worker(
    app: AppHandle,
    controller: Arc<AutoSyncController>,
    receiver: mpsc::UnboundedReceiver<AutoSyncCommand>,
) {
    tauri::async_runtime::spawn(async move {
        auto_sync_worker(app, controller, receiver).await;
    });
}

async fn auto_sync_worker(
    app: AppHandle,
    controller: Arc<AutoSyncController>,
    mut receiver: mpsc::UnboundedReceiver<AutoSyncCommand>,
) {
    loop {
        let store = match default_store() {
            Ok(store) => store,
            Err(error) => {
                eprintln!("auto sync failed to open store: {error}");
                if receiver.recv().await.is_none() {
                    break;
                }
                continue;
            }
        };

        let config = match store.load_config() {
            Ok(config) => config,
            Err(error) => {
                eprintln!("auto sync failed to load config: {error}");
                if receiver.recv().await.is_none() {
                    break;
                }
                continue;
            }
        };

        let state = match load_state(&store) {
            Ok(state) => state,
            Err(error) => {
                eprintln!("auto sync failed to load state: {error}");
                if receiver.recv().await.is_none() {
                    break;
                }
                continue;
            }
        };

        if !config.sync_enabled || !is_configured(&config) {
            if receiver.recv().await.is_none() {
                break;
            }
            continue;
        }

        tokio::select! {
            maybe_command = receiver.recv() => {
                match maybe_command {
                    Some(command) => {
                        let (should_sync, disconnected) = collect_pending_auto_sync(command, &mut receiver);
                        if disconnected {
                            break;
                        }
                        if should_sync {
                            let _ = run_sync_operation(&app, Some(&controller)).await;
                        }
                    }
                    None => break,
                }
            }
            wait_result = wait_for_remote_change(&config, state.last_revision) => {
                match wait_result {
                    Ok(response) if response.changed && response.revision > state.last_revision => {
                        let _ = run_sync_operation(&app, Some(&controller)).await;
                    }
                    Ok(_) => {}
                    Err(error) => {
                        eprintln!("auto sync remote wait failed: {error}");
                    }
                }
            }
        }
    }
}

fn collect_pending_auto_sync(
    initial: AutoSyncCommand,
    receiver: &mut mpsc::UnboundedReceiver<AutoSyncCommand>,
) -> (bool, bool) {
    let mut should_sync = matches!(initial, AutoSyncCommand::SyncNow);

    loop {
        match receiver.try_recv() {
            Ok(AutoSyncCommand::SyncNow) => should_sync = true,
            Ok(AutoSyncCommand::ConfigChanged) => {}
            Err(mpsc::error::TryRecvError::Empty) => return (should_sync, false),
            Err(mpsc::error::TryRecvError::Disconnected) => return (should_sync, true),
        }
    }
}

async fn run_sync_operation(
    app: &AppHandle,
    controller: Option<&AutoSyncController>,
) -> Result<SyncStatus, AppError> {
    let _sync_guard = match controller {
        Some(runtime) => Some(runtime.sync_lock.lock().await),
        None => None,
    };

    let store = default_store()?;
    let config = store.load_config()?;

    match sync_now(&store, &config).await {
        Ok(status) => {
            emit_sync_status(app, &status);
            let _ = app.emit(NOTES_CHANGED_EVENT, ());
            Ok(status)
        }
        Err(error) => {
            if let Ok(current_status) = status(&store, &config) {
                emit_sync_status(app, &current_status);
            }
            Err(error)
        }
    }
}

fn emit_sync_status(app: &AppHandle, status: &SyncStatus) {
    let _ = app.emit(SYNC_STATUS_CHANGED_EVENT, status);
}

async fn sync_inner(
    store: &NoteStore,
    config: &AppConfig,
    state: &mut SyncState,
) -> Result<(), AppError> {
    let remote = fetch_changes(config, state.last_revision).await?;
    for change in &remote.changes {
        apply_remote_change(store, state, &change.note)?;
        remember_remote_change(state, &change.note);
    }

    let local_changes = collect_local_changes(store, state, &state.device_id)?;
    if local_changes.is_empty() {
        state.last_revision = remote.revision;
        return Ok(());
    }

    let pushed = push_changes(config, &state.device_id, local_changes.clone()).await?;
    for change in local_changes {
        remember_local_change(state, change);
    }
    state.last_revision = pushed.revision.max(remote.revision);
    Ok(())
}

pub fn collect_local_changes(
    store: &NoteStore,
    state: &SyncState,
    device_id: &str,
) -> Result<Vec<SyncChange>, AppError> {
    let notes = store.list_note_contents()?;
    let mut current_ids = HashMap::new();
    let mut changes = Vec::new();

    for note in notes {
        current_ids.insert(note.id.clone(), ());
        let content_hash = note_hash(&note);
        let known = state.notes.get(&note.id);
        if known.map(|entry| entry.content_hash.as_str()) != Some(content_hash.as_str()) {
            changes.push(change_from_note(note, content_hash, device_id));
        }
    }

    for (id, known) in &state.notes {
        if current_ids.contains_key(id) || state.tombstones.contains_key(id) {
            continue;
        }
        // A missing note that was present in the previous snapshot is a local delete.
        // Tombstones carry that delete across devices so an older client cannot recreate it.
        let deleted_at = Utc::now();
        changes.push(SyncChange {
            id: id.clone(),
            title: String::new(),
            content: String::new(),
            category: String::new(),
            created_at: known.updated_at,
            updated_at: known.updated_at,
            deleted_at: Some(deleted_at),
            content_hash: deleted_hash(id, deleted_at),
            device_id: device_id.to_string(),
        });
    }

    Ok(changes)
}

pub fn apply_remote_change(
    store: &NoteStore,
    state: &SyncState,
    remote: &SyncChange,
) -> Result<(), AppError> {
    let local = store.read_note(&remote.id).ok();
    let local_changed = local
        .as_ref()
        .map(|note| local_note_changed(note, state))
        .unwrap_or(false);

    if let Some(deleted_at) = remote.deleted_at {
        if let Some(local_note) = local {
            if local_changed && local_note.updated_at > deleted_at {
                return Ok(());
            }
            if local_changed {
                create_conflict_backup(store, &local_note)?;
            }
            store.delete_synced_note(&remote.id)?;
        }
        return Ok(());
    }

    if let Some(local_note) = local {
        if local_changed && local_note.updated_at > remote.updated_at {
            return Ok(());
        }
        if local_changed && note_hash(&local_note) != remote.content_hash {
            create_conflict_backup(store, &local_note)?;
        }
    }

    store.upsert_synced_note(
        &remote.id,
        &remote.title,
        &remote.content,
        &remote.category,
        remote.created_at,
        remote.updated_at,
    )?;
    Ok(())
}

fn remember_remote_change(state: &mut SyncState, remote: &SyncChange) {
    if let Some(deleted_at) = remote.deleted_at {
        state.notes.remove(&remote.id);
        state.tombstones.insert(
            remote.id.clone(),
            TombstoneState {
                deleted_at,
                content_hash: remote.content_hash.clone(),
            },
        );
        return;
    }

    state.tombstones.remove(&remote.id);
    state.notes.insert(
        remote.id.clone(),
        SyncedNoteState {
            content_hash: remote.content_hash.clone(),
            updated_at: remote.updated_at,
        },
    );
}

fn remember_local_change(state: &mut SyncState, change: SyncChange) {
    if let Some(deleted_at) = change.deleted_at {
        state.notes.remove(&change.id);
        state.tombstones.insert(
            change.id,
            TombstoneState {
                deleted_at,
                content_hash: change.content_hash,
            },
        );
        return;
    }

    state.tombstones.remove(&change.id);
    state.notes.insert(
        change.id,
        SyncedNoteState {
            content_hash: change.content_hash,
            updated_at: change.updated_at,
        },
    );
}

fn rebuild_note_state(store: &NoteStore, state: &mut SyncState) -> Result<(), AppError> {
    state.notes.clear();
    for note in store.list_note_contents()? {
        state.notes.insert(
            note.id.clone(),
            SyncedNoteState {
                content_hash: note_hash(&note),
                updated_at: note.updated_at,
            },
        );
    }
    Ok(())
}

fn load_state(store: &NoteStore) -> Result<SyncState, AppError> {
    let path = store.sync_state_path();
    if !path.exists() {
        return Ok(SyncState::default());
    }

    match serde_json::from_str::<SyncState>(&fs::read_to_string(&path)?) {
        Ok(mut state) => {
            if state.device_id.trim().is_empty() {
                state.device_id = Uuid::new_v4().to_string();
            }
            Ok(state)
        }
        Err(_) => {
            let corrupt_name = format!(
                "sync_state.corrupt-{}.json",
                Utc::now().format("%Y%m%d%H%M%S")
            );
            fs::rename(&path, path.with_file_name(corrupt_name))?;
            Ok(SyncState::default())
        }
    }
}

fn save_state(store: &NoteStore, state: &SyncState) -> Result<(), AppError> {
    let path = store.sync_state_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp_path = path.with_extension("json.tmp");
    fs::write(&temp_path, serde_json::to_string_pretty(state)?)?;
    if path.exists() {
        fs::remove_file(&path)?;
    }
    fs::rename(temp_path, path)?;
    Ok(())
}

fn status_from_state(config: &AppConfig, state: &SyncState) -> SyncStatus {
    SyncStatus {
        enabled: config.sync_enabled,
        configured: is_configured(config),
        last_revision: state.last_revision,
        last_sync_at: state.last_sync_at,
        last_error: state.last_error.clone(),
    }
}

fn is_configured(config: &AppConfig) -> bool {
    !config.sync_server_url.trim().is_empty() && !config.sync_token.trim().is_empty()
}

fn change_from_note(note: Note, content_hash: String, device_id: &str) -> SyncChange {
    SyncChange {
        id: note.id,
        title: note.title,
        content: note.content,
        category: note.category,
        created_at: note.created_at,
        updated_at: note.updated_at,
        deleted_at: None,
        content_hash,
        device_id: device_id.to_string(),
    }
}

fn local_note_changed(note: &Note, state: &SyncState) -> bool {
    state
        .notes
        .get(&note.id)
        .map(|known| known.content_hash.as_str() != note_hash(note))
        .unwrap_or(true)
}

fn create_conflict_backup(store: &NoteStore, note: &Note) -> Result<(), AppError> {
    // Conflict copies keep the losing local text visible instead of silently dropping it.
    // They are normal notes, so users can delete or merge them with the existing UI.
    let suffix = Utc::now().format("%Y-%m-%d %H-%M");
    store.create_note(SaveNoteRequest {
        title: format!("{} Conflict backup {}", note.title, suffix)
            .trim()
            .to_string(),
        content: note.content.clone(),
        category: note.category.clone(),
    })?;
    Ok(())
}

fn note_hash(note: &Note) -> String {
    let created_at = note.created_at.to_rfc3339();
    let updated_at = note.updated_at.to_rfc3339();
    stable_hash([
        note.id.as_str(),
        note.title.as_str(),
        note.content.as_str(),
        note.category.as_str(),
        created_at.as_str(),
        updated_at.as_str(),
    ])
}

fn deleted_hash(id: &str, deleted_at: DateTime<Utc>) -> String {
    let deleted_at = deleted_at.to_rfc3339();
    stable_hash([id, "deleted", deleted_at.as_str()])
}

fn stable_hash<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    // FNV-1a is enough here because the value is a local change detector, not a security boundary.
    let mut hash = 0xcbf29ce484222325_u64;
    for part in parts {
        for byte in part.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

async fn request_health(config: &AppConfig) -> Result<(), AppError> {
    let client = reqwest::Client::new();
    client
        .get(sync_endpoint(config, "/health")?)
        .bearer_auth(config.sync_token.trim())
        .send()
        .await
        .map_err(sync_transport_error)?
        .error_for_status()
        .map_err(sync_transport_error)?;
    Ok(())
}

async fn fetch_changes(config: &AppConfig, since: u64) -> Result<ChangesResponse, AppError> {
    let client = reqwest::Client::new();
    client
        .get(sync_endpoint(config, "/v1/changes")?)
        .query(&[("since", since)])
        .bearer_auth(config.sync_token.trim())
        .send()
        .await
        .map_err(sync_transport_error)?
        .error_for_status()
        .map_err(sync_transport_error)?
        .json()
        .await
        .map_err(sync_transport_error)
}

async fn push_changes(
    config: &AppConfig,
    device_id: &str,
    changes: Vec<SyncChange>,
) -> Result<PushResponse, AppError> {
    let client = reqwest::Client::new();
    client
        .post(sync_endpoint(config, "/v1/push")?)
        .bearer_auth(config.sync_token.trim())
        .json(&PushRequest {
            device_id: device_id.to_string(),
            changes,
        })
        .send()
        .await
        .map_err(sync_transport_error)?
        .error_for_status()
        .map_err(sync_transport_error)?
        .json()
        .await
        .map_err(sync_transport_error)
}

async fn wait_for_remote_change(
    config: &AppConfig,
    since: u64,
) -> Result<WaitResponse, AppError> {
    let client = reqwest::Client::new();
    client
        .get(sync_endpoint(config, "/v1/wait")?)
        .query(&[("since", since), ("timeoutSeconds", WAIT_TIMEOUT_SECONDS)])
        .bearer_auth(config.sync_token.trim())
        .send()
        .await
        .map_err(sync_transport_error)?
        .error_for_status()
        .map_err(sync_transport_error)?
        .json()
        .await
        .map_err(sync_transport_error)
}

fn server_url(config: &AppConfig) -> String {
    config
        .sync_server_url
        .trim()
        .trim_end_matches('/')
        .to_string()
}

fn sync_endpoint(config: &AppConfig, path: &str) -> Result<reqwest::Url, AppError> {
    let raw = server_url(config);
    let normalized = if raw.ends_with('/') {
        raw
    } else {
        format!("{raw}/")
    };
    let base = reqwest::Url::parse(&normalized).map_err(|_| AppError {
        code: "syncConfig".into(),
        message:
            "同步服务器地址格式不正确，请填写完整地址，例如 http://127.0.0.1:8787 或 http://[::1]:8787。"
                .into(),
    })?;
    base.join(path.trim_start_matches('/')).map_err(|_| AppError {
        code: "syncConfig".into(),
        message:
            "同步服务器地址格式不正确，请填写完整地址，例如 http://127.0.0.1:8787 或 http://[::1]:8787。"
                .into(),
    })
}

fn sync_transport_error(error: reqwest::Error) -> AppError {
    let message = if error.is_timeout() {
        "连接同步服务器超时，请检查网络或服务器状态。".into()
    } else if error.is_connect() {
        "无法连接到同步服务器，请检查地址、端口和网络。".into()
    } else if error.is_decode() {
        "同步服务器返回了无法识别的响应，请确认服务端版本兼容。".into()
    } else if let Some(status) = error.status() {
        match status.as_u16() {
            401 | 403 => "同步认证失败，请检查同步 Token 是否正确。".into(),
            404 => "同步服务器地址不正确，未找到同步接口。".into(),
            500..=599 => "同步服务器暂时不可用，请稍后再试。".into(),
            code => format!("同步服务器返回了错误（HTTP {code}）。"),
        }
    } else if error.is_builder() {
        "同步服务器地址格式不正确，请填写完整地址，例如 http://127.0.0.1:8787 或 http://[::1]:8787。"
            .into()
    } else {
        "同步请求失败，请稍后重试。".into()
    };

    AppError {
        code: "syncTransport".into(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::notes::{NoteStore, SaveNoteRequest};
    use chrono::{DateTime, Utc};
    use std::{fs, path::PathBuf};

    fn test_root(name: &str) -> PathBuf {
        let base = std::env::var_os("FLORAL_NOTEPAPER_TEST_TEMP_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("floral-notepaper-sync-tests"));
        let root = base.join(name);
        if root.exists() {
            fs::remove_dir_all(&root).expect("remove stale test root");
        }
        fs::create_dir_all(&root).expect("create test root");
        root
    }

    #[test]
    fn queues_initial_sync_when_auto_sync_starts() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let controller = AutoSyncController::new(sender);

        request_startup_auto_sync(&controller);

        assert!(matches!(receiver.try_recv(), Ok(AutoSyncCommand::SyncNow)));
    }

    #[test]
    fn detects_new_and_deleted_local_notes_as_push_changes() {
        let store = NoteStore::new(test_root("detect-local-changes"));
        let note = store
            .create_note(SaveNoteRequest {
                title: "Local".into(),
                content: "body".into(),
                category: String::new(),
            })
            .expect("create local note");
        let mut state = SyncState::default();
        state.notes.insert(
            "deleted-note".into(),
            SyncedNoteState {
                content_hash: "old".into(),
                updated_at: parse_time("2026-05-18T08:00:00Z"),
            },
        );

        let changes = collect_local_changes(&store, &state, "device-a").expect("collect changes");

        assert!(changes
            .iter()
            .any(|change| change.id == note.id && change.deleted_at.is_none()));
        assert!(changes
            .iter()
            .any(|change| change.id == "deleted-note" && change.deleted_at.is_some()));
    }

    #[test]
    fn applies_remote_update_and_keeps_local_conflict_backup() {
        let root = test_root("remote-conflict");
        let store = NoteStore::new(root);
        let local = store
            .upsert_synced_note(
                "conflict-note",
                "Local title",
                "local body",
                "",
                parse_time("2026-05-18T08:00:00Z"),
                parse_time("2026-05-18T08:05:00Z"),
            )
            .expect("create local note");
        let mut state = SyncState::default();
        state.notes.insert(
            local.id.clone(),
            SyncedNoteState {
                content_hash: "previous-hash".into(),
                updated_at: parse_time("2026-05-18T08:00:00Z"),
            },
        );
        let remote = SyncChange {
            id: local.id.clone(),
            title: "Remote title".into(),
            content: "remote body".into(),
            category: "Remote".into(),
            created_at: parse_time("2026-05-18T08:00:00Z"),
            updated_at: parse_time("2026-05-18T08:20:00Z"),
            deleted_at: None,
            content_hash: "remote-hash".into(),
            device_id: "device-b".into(),
        };

        apply_remote_change(&store, &state, &remote).expect("apply remote change");

        let synced = store.read_note(&local.id).expect("read remote winner");
        assert_eq!(synced.title, "Remote title");
        assert_eq!(synced.content, "remote body");
        assert!(store
            .list_notes()
            .expect("list notes")
            .iter()
            .any(|note| note.title.contains("Conflict backup")));
    }

    #[test]
    fn keeps_bracketed_ipv6_server_urls_intact() {
        let config = AppConfig {
            notes_dir: String::new(),
            global_shortcut: "Ctrl+Space".into(),
            close_to_tray: false,
            autostart: false,
            default_view_mode: "split".into(),
            note_auto_save: true,
            note_surface_auto_save: true,
            tile_color: "#f6f3ec".into(),
            tile_color_mode: "system".into(),
            theme: "system".into(),
            font_size: 14,
            surface_font_size: 14,
            external_file_auto_save: true,
            remember_surface_size: true,
            tile_ctrl_close: true,
            surface_width: None,
            surface_height: None,
            sync_enabled: true,
            sync_server_url: " http://[240e:abcd::1]:18787/ ".into(),
            sync_token: "secret".into(),
        };

        assert_eq!(server_url(&config), "http://[240e:abcd::1]:18787");
    }

    #[test]
    fn translates_invalid_server_urls_into_friendly_messages() {
        let config = AppConfig {
            notes_dir: String::new(),
            global_shortcut: "Ctrl+Space".into(),
            close_to_tray: false,
            autostart: false,
            default_view_mode: "split".into(),
            note_auto_save: true,
            note_surface_auto_save: true,
            tile_color: "#f6f3ec".into(),
            tile_color_mode: "system".into(),
            theme: "system".into(),
            font_size: 14,
            surface_font_size: 14,
            external_file_auto_save: true,
            remember_surface_size: true,
            tile_ctrl_close: true,
            surface_width: None,
            surface_height: None,
            sync_enabled: true,
            sync_server_url: "127.0.0.1:18787".into(),
            sync_token: "secret".into(),
        };

        let error = tauri::async_runtime::block_on(request_health(&config)).expect_err("invalid url");

        assert_eq!(
            error.message,
            "同步服务器地址格式不正确，请填写完整地址，例如 http://127.0.0.1:8787 或 http://[::1]:8787。"
        );
    }

    #[test]
    fn recovers_from_corrupt_sync_state_files() {
        let store = NoteStore::new(test_root("corrupt-sync-state"));
        fs::write(store.sync_state_path(), "{not-json").expect("write corrupt state");

        let state = load_state(&store).expect("load state after recovery");

        assert_eq!(state.last_revision, 0);
        assert!(state.notes.is_empty());
        assert!(state.tombstones.is_empty());
        assert!(!state.device_id.trim().is_empty());
        assert!(!store.sync_state_path().exists());

        let renamed = fs::read_dir(store.base_dir())
            .expect("read test root")
            .filter_map(Result::ok)
            .any(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                name.starts_with("sync_state.corrupt-") && name.ends_with(".json")
            });
        assert!(renamed);
    }

    fn parse_time(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("time")
            .with_timezone(&Utc)
    }
}
