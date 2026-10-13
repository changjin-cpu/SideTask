#[cfg(target_os = "windows")]
use crate::windows_webview_shutdown;
use crate::{application, domain, infrastructure, platform, sync};
use application::TaskService;
use domain::{Action, Snapshot};
use infrastructure::SqliteRepository;
use serde_json::Value;
use std::io::Write;
use std::sync::Mutex;
use tauri::{Emitter, Manager};

pub struct AppState {
    pub service: Mutex<TaskService>,
    pub dock: Mutex<platform::DockRuntime>,
    pub console: platform::console_window::ConsoleRuntime,
    pub exit: Mutex<platform::exit::ExitRuntime>,
}

pub(crate) fn task_state(app: &tauri::AppHandle) -> Result<tauri::State<'_, AppState>, String> {
    app.try_state::<AppState>()
        .ok_or_else(|| "数据库需要恢复，尚未加载任务；请在恢复控制台选择经过验证的备份。".into())
}

fn initialize_task_state(data_dir: &std::path::Path) -> Result<AppState, String> {
    std::fs::create_dir_all(data_dir).map_err(|e| format!("无法访问本地数据目录：{e}"))?;
    let repository = SqliteRepository::open(&data_dir.join("sidetask.sqlite3"))?;
    let service = TaskService::new(Box::new(repository))?;
    let raw_placement = service.repository.load_placement()?;
    let placement = raw_placement
        .as_deref()
        .and_then(|p| serde_json::from_str(p).ok())
        .unwrap_or_default();
    let console = platform::console_window::ConsoleRuntime::new(raw_placement.as_deref());
    Ok(AppState {
        service: Mutex::new(service),
        console,
        dock: Mutex::new(platform::DockRuntime::new(placement)),
        exit: Mutex::new(platform::exit::ExitRuntime::default()),
    })
}

fn require_window(label: &str, allowed: &[&str]) -> Result<(), String> {
    if allowed.contains(&label) {
        Ok(())
    } else {
        Err("此窗口无权执行该操作。".into())
    }
}

fn authorize_mutation(label: &str, action: &Action) -> Result<(), String> {
    require_window(label, &["console", "edge-panel"])?;
    if matches!(action, Action::ResetDemo { .. }) {
        return Err("正式应用不能用演示数据覆盖任务。".into());
    }
    if label == "console" {
        return Ok(());
    }
    match action {
        Action::CreateTask { task, .. }
            if task.as_object().is_some_and(|fields| {
                fields.get("addToToday").and_then(Value::as_bool) == Some(true)
                    && fields.keys().all(|key| {
                        [
                            "title",
                            "notes",
                            "priority",
                            "dueDate",
                            "dueTime",
                            "dueTimezone",
                            "addToToday",
                        ]
                        .contains(&key.as_str())
                    })
                    && fields
                        .get("notes")
                        .is_none_or(|value| value.as_str() == Some(""))
                    && fields
                        .get("priority")
                        .is_none_or(|value| value.as_str() == Some("normal"))
                    && ["dueDate", "dueTime", "dueTimezone"]
                        .iter()
                        .all(|key| fields.get(*key).is_none_or(Value::is_null))
            }) =>
        {
            Ok(())
        }
        Action::SetCompleted { .. } | Action::PlanTask { .. } | Action::ReorderToday { .. } => {
            Ok(())
        }
        Action::ReorderTasks { scope, .. } if scope == "deadlines" => Ok(()),
        Action::UpdateSettings { changes }
            if changes.as_object().is_some_and(|object| {
                !object.is_empty()
                    && object
                        .keys()
                        .all(|key| ["pinned", "ddlSort", "panelSplit"].contains(&key.as_str()))
            }) =>
        {
            Ok(())
        }
        _ => Err("请在控制台执行此操作。".into()),
    }
}

#[tauri::command]
fn get_snapshot(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<Snapshot, String> {
    require_window(window.label(), &["console", "edge-panel", "edge-handle"])?;
    let mut snapshot = task_state(&app)?
        .service
        .lock()
        .map(|s| s.snapshot.clone())
        .map_err(|_| "本地任务服务暂不可用。".to_string())?;
    if window.label() == "edge-handle" {
        snapshot.tasks.clear();
        snapshot.plans.clear();
        snapshot.task_order.clear();
        snapshot.deadline_order.clear();
    }
    Ok(snapshot)
}
#[tauri::command]
fn mutate(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    action: Value,
    expected_revision: u64,
) -> Result<Snapshot, String> {
    let action: Action =
        serde_json::from_value(action).map_err(|e| format!("操作格式无效：{e}"))?;
    authorize_mutation(window.label(), &action)?;
    let state = task_state(&app)?;
    let snapshot = {
        let mut service = state.service.lock().map_err(|_| "本地任务服务暂不可用。")?;
        platform::exit::ensure_running(&state)?;
        service.mutate(action, expected_revision)?
    };
    // Notification failure must not turn a committed transaction into a reported failure.
    if let Err(error) = app.emit(
        "sidetask:changed",
        serde_json::json!({"revision": snapshot.revision}),
    ) {
        eprintln!("snapshot notification: {error}");
    }
    if let Some(sync) = app.try_state::<sync::SyncRuntime>() {
        sync.wake();
    }
    Ok(snapshot)
}

#[tauri::command]
fn sync_status(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<sync::SyncStatus, String> {
    require_window(window.label(), &["console"])?;
    app.try_state::<sync::SyncRuntime>()
        .ok_or("同步服务尚未就绪。")?
        .status(&app)
}
#[tauri::command]
async fn sync_sign_in(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    project_url: String,
    publishable_key: String,
    email: String,
    password: String,
    merge_local: bool,
) -> Result<sync::SyncStatus, String> {
    require_window(window.label(), &["console"])?;
    let config = sync::state::SyncConfig::new(project_url, publishable_key)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.try_state::<sync::SyncRuntime>()
            .ok_or("同步服务尚未就绪。")?
            .sign_in(&app, config, email, password, merge_local)
    })
    .await
    .map_err(|_| "登录操作中断，请重试。")?
}
#[tauri::command]
async fn sync_sign_out(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<sync::SyncStatus, String> {
    require_window(window.label(), &["console"])?;
    tauri::async_runtime::spawn_blocking(move || {
        app.try_state::<sync::SyncRuntime>()
            .ok_or("同步服务尚未就绪。")?
            .sign_out(&app)
    })
    .await
    .map_err(|_| "断开操作中断，请重试。")?
}
#[tauri::command]
async fn sync_now(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<sync::SyncStatus, String> {
    require_window(window.label(), &["console"])?;
    tauri::async_runtime::spawn_blocking(move || {
        app.try_state::<sync::SyncRuntime>()
            .ok_or("同步服务尚未就绪。")?
            .sync_now(&app)
    })
    .await
    .map_err(|_| "同步操作中断，请重试。")?
}
#[tauri::command]
async fn sync_resolve(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    choices: std::collections::BTreeMap<String, sync::data::ConflictChoice>,
) -> Result<sync::SyncStatus, String> {
    require_window(window.label(), &["console"])?;
    tauri::async_runtime::spawn_blocking(move || {
        app.try_state::<sync::SyncRuntime>()
            .ok_or("同步服务尚未就绪。")?
            .resolve(&app, choices)
    })
    .await
    .map_err(|_| "冲突处理操作中断，请重试。")?
}
#[tauri::command]
async fn window_action(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    action: String,
    payload: Option<Value>,
) -> Result<(), String> {
    platform::window_action(&app, &window, &action, payload.unwrap_or(Value::Null))
}
#[tauri::command]
fn get_monitors(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<Value, String> {
    require_window(window.label(), &["console"])?;
    task_state(&app)?;
    platform::get_monitors(&app)
}

#[tauri::command]
async fn get_window_status(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<platform::WindowStatus, String> {
    require_window(window.label(), &["console"])?;
    platform::get_window_status(&app)
}

#[tauri::command]
fn get_console_position_status(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<platform::console_window::PositionStatus, String> {
    require_window(window.label(), &["console"])?;
    platform::console_window::status(&app)
}

fn read_usage_guide_for_window(state: &AppState, label: &str) -> Result<bool, String> {
    require_window(label, &["console"])?;
    state
        .service
        .lock()
        .map_err(|_| "任务服务暂不可用。")?
        .repository
        .read_usage_guide_seen()
}

fn acknowledge_usage_guide_for_window(state: &AppState, label: &str) -> Result<(), String> {
    require_window(label, &["console"])?;
    let mut service = state.service.lock().map_err(|_| "任务服务暂不可用。")?;
    platform::exit::ensure_running(state)?;
    service.repository.acknowledge_usage_guide()
}

#[tauri::command]
async fn get_usage_guide_seen(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<bool, String> {
    let state = task_state(&app)?;
    read_usage_guide_for_window(&state, window.label())
}

#[tauri::command]
async fn acknowledge_usage_guide(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<(), String> {
    let state = task_state(&app)?;
    acknowledge_usage_guide_for_window(&state, window.label())
}

#[tauri::command]
fn get_pending_exit(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Option<platform::exit::ExitRequest>, String> {
    require_window(window.label(), &["console", "edge-panel"])?;
    platform::exit::pending(&app, window.label())
}

#[tauri::command]
async fn resolve_exit(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    request_id: u64,
    allow: bool,
) -> Result<(), String> {
    require_window(window.label(), &["console", "edge-panel"])?;
    platform::exit::resolve(&app, window.label(), request_id, allow)
}

#[tauri::command]
async fn export_backup(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Value, String> {
    require_window(window.label(), &["console"])?;
    let content = task_state(&app)?
        .service
        .lock()
        .map_err(|_| "任务服务暂不可用。")?
        .export_backup()?;
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("exports");
    let path = write_export(&directory, &content)?;
    Ok(serde_json::json!({"path":path.to_string_lossy()}))
}

fn write_export(directory: &std::path::Path, content: &str) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir_all(directory).map_err(|e| format!("无法创建备份目录：{e}"))?;
    let path = directory.join(format!("SideTask-{}.json", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|e| format!("无法创建备份文件：{e}"))?;
    if let Err(error) = file
        .write_all(content.as_bytes())
        .and_then(|_| file.sync_all())
    {
        drop(file);
        let _ = std::fs::remove_file(&path);
        return Err(format!("备份写入失败：{error}"));
    }
    Ok(path)
}

#[tauri::command]
async fn preview_restore(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    content: String,
) -> Result<application::RestorePreview, String> {
    require_window(window.label(), &["console"])?;
    task_state(&app)?
        .service
        .lock()
        .map_err(|_| "任务服务暂不可用。")?
        .preview_restore(&content)
}

#[tauri::command]
async fn restore_backup(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    content: String,
    expected_revision: u64,
) -> Result<application::RestoreResult, String> {
    require_window(window.label(), &["console"])?;
    let state = task_state(&app)?;
    let result = {
        let mut service = state.service.lock().map_err(|_| "任务服务暂不可用。")?;
        platform::exit::ensure_running(&state)?;
        service.restore_backup(&content, expected_revision)?
    };
    if let Err(error) = app.emit(
        "sidetask:changed",
        serde_json::json!({"revision":result.snapshot.revision}),
    ) {
        eprintln!("restore notification: {error}");
    }
    Ok(result)
}

#[tauri::command]
async fn get_startup_recovery(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Option<platform::startup_recovery::RecoveryStatus>, String> {
    require_window(window.label(), &["console"])?;
    platform::startup_recovery::status(&app)
}

#[tauri::command]
async fn recover_startup_backup(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    candidate_id: String,
) -> Result<infrastructure::recovery::RecoveryOutcome, String> {
    require_window(window.label(), &["console"])?;
    platform::startup_recovery::recover(&app, &candidate_id)
}

#[tauri::command]
async fn restart_after_recovery(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<(), String> {
    require_window(window.label(), &["console"])?;
    platform::startup_recovery::restart(&app)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            let _ = platform::open_console(app, Value::Null);
        }))
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            mutate,
            window_action,
            get_monitors,
            get_window_status,
            get_console_position_status,
            get_usage_guide_seen,
            acknowledge_usage_guide,
            get_pending_exit,
            resolve_exit,
            export_backup,
            preview_restore,
            restore_backup,
            get_startup_recovery,
            recover_startup_backup,
            restart_after_recovery,
            sync_status,
            sync_sign_in,
            sync_sign_out,
            sync_now,
            sync_resolve
        ])
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            match initialize_task_state(&data_dir) {
                Ok(state) => {
                    app.manage(state);
                    platform::setup(app)?;
                    match sync::SyncRuntime::new() {
                        Ok(runtime) => {
                            app.manage(runtime);
                            sync::SyncRuntime::start(app.handle().clone());
                        }
                        Err(_) => eprintln!(
                            "sync network initialization unavailable; local tasks remain usable"
                        ),
                    }
                }
                Err(error) => {
                    // The failed repository/service was dropped before entering
                    // offline recovery: there is no writable task connection.
                    app.manage(platform::startup_recovery::RecoveryState::new(
                        data_dir, error,
                    ));
                    platform::setup_recovery(app)?;
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("SideTask could not initialize; existing data has not been reset");
    app.run(|app, event| match event {
        tauri::RunEvent::Exit => {
            if let Some(sync) = app.try_state::<sync::SyncRuntime>() {
                sync.stop();
            }
            platform::cleanup();
            #[cfg(target_os = "windows")]
            windows_webview_shutdown::close_after_exit_authorized(app);
        }
        tauri::RunEvent::ExitRequested { api, .. } if !platform::exit::is_authorized(app) => {
            api.prevent_exit();
            if let Err(error) = platform::exit::request(app) {
                eprintln!("exit request: {error}");
            }
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => {
            let _ = platform::open_console(app, Value::Null);
        }
        _ => {}
    });
}

#[cfg(test)]
mod security_tests {
    use super::*;

    #[test]
    fn usage_guide_command_handlers_reject_other_windows_without_changing_data() {
        let directory = std::env::temp_dir().join(format!(
            "sidetask-guide-command-test-{}",
            uuid::Uuid::new_v4()
        ));
        let state = initialize_task_state(&directory).unwrap();
        let original = serde_json::to_value(&state.service.lock().unwrap().snapshot).unwrap();
        assert!(!read_usage_guide_for_window(&state, "console").unwrap());
        for label in ["edge-panel", "edge-handle", "unknown"] {
            assert!(read_usage_guide_for_window(&state, label).is_err());
            assert!(acknowledge_usage_guide_for_window(&state, label).is_err());
        }
        assert!(!read_usage_guide_for_window(&state, "console").unwrap());
        assert!(state
            .service
            .lock()
            .unwrap()
            .repository
            .load_placement()
            .unwrap()
            .is_none());
        acknowledge_usage_guide_for_window(&state, "console").unwrap();
        assert!(read_usage_guide_for_window(&state, "console").unwrap());
        assert_eq!(
            serde_json::to_value(&state.service.lock().unwrap().snapshot).unwrap(),
            original
        );
        drop(state);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn edge_panel_can_create_only_a_simple_today_task() {
        for task in [
            serde_json::json!({"title":"快速任务", "addToToday":true}),
            serde_json::json!({"title":"快速任务", "notes":"", "priority":"normal", "dueDate":null, "dueTime":null, "dueTimezone":null, "addToToday":true}),
        ] {
            let action = Action::CreateTask {
                task,
                date: "2026-09-25".into(),
            };
            assert!(authorize_mutation("edge-panel", &action).is_ok());
            assert!(authorize_mutation("console", &action).is_ok());
            assert!(authorize_mutation("edge-handle", &action).is_err());
            assert!(authorize_mutation("unknown", &action).is_err());
        }
        for task in [
            serde_json::json!({"title":"任务", "addToToday":false}),
            serde_json::json!({"title":"任务"}),
            serde_json::json!({"title":"任务", "addToToday":true, "dueDate":"2026-10-01"}),
            serde_json::json!({"title":"任务", "addToToday":true, "dueTimezone":"Asia/Shanghai"}),
            serde_json::json!({"title":"任务", "addToToday":true, "priority":"high"}),
            serde_json::json!({"title":"任务", "addToToday":true, "notes":"完整编辑"}),
            serde_json::json!({"title":"任务", "addToToday":true, "completed":true}),
        ] {
            let action = Action::CreateTask {
                task,
                date: "2026-09-25".into(),
            };
            assert!(authorize_mutation("edge-panel", &action).is_err());
            assert!(authorize_mutation("console", &action).is_ok());
        }
    }

    #[test]
    fn lifecycle_mutations_are_console_only() {
        for action in [
            Action::TrashTask {
                id: "task".into(),
                expected_revision: 1,
            },
            Action::RestoreTask {
                id: "task".into(),
                expected_revision: 1,
            },
        ] {
            assert!(authorize_mutation("console", &action).is_ok());
            for label in ["edge-panel", "edge-handle", "unknown"] {
                assert!(authorize_mutation(label, &action).is_err());
            }
        }
    }

    #[test]
    fn legacy_split_default_is_read_only_and_explicit_split_survives_sqlite_reopen() {
        use crate::infrastructure::Repository;

        let directory = std::env::temp_dir().join(format!(
            "sidetask-panel-split-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("sidetask.sqlite3");
        let mut initial = Snapshot::demo("2026-09-24");
        initial.tasks[0].deleted_at = Some("2026-09-24T00:00:00Z".into());
        let mut repository = SqliteRepository::open(&path).unwrap();
        repository.save(&initial).unwrap();
        drop(repository);

        let mut legacy = serde_json::to_value(&initial).unwrap();
        legacy["settings"]
            .as_object_mut()
            .unwrap()
            .remove("panelSplit");
        let legacy_json = serde_json::to_string(&legacy).unwrap();
        {
            let connection = rusqlite::Connection::open(&path).unwrap();
            connection
                .execute(
                    "UPDATE app_state SET value=?1 WHERE key='snapshot'",
                    rusqlite::params![legacy_json],
                )
                .unwrap();
        }
        let mut service =
            TaskService::new(Box::new(SqliteRepository::open(&path).unwrap())).unwrap();
        assert_eq!(service.snapshot.settings.panel_split, 54);
        assert_eq!(
            serde_json::to_value(&service.snapshot).unwrap(),
            serde_json::to_value(&initial).unwrap()
        );
        {
            let connection = rusqlite::Connection::open(&path).unwrap();
            let stored: String = connection
                .query_row(
                    "SELECT value FROM app_state WHERE key='snapshot'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                stored, legacy_json,
                "loading defaults must not rewrite the old snapshot"
            );
        }

        service
            .mutate(
                Action::UpdateSettings {
                    changes: serde_json::json!({"panelSplit": 67}),
                },
                initial.revision,
            )
            .unwrap();
        drop(service);
        let reopened = TaskService::new(Box::new(SqliteRepository::open(&path).unwrap())).unwrap();
        let mut expected = serde_json::to_value(&initial).unwrap();
        expected["settings"]["panelSplit"] = serde_json::json!(67);
        expected["revision"] = serde_json::json!(initial.revision + 1);
        assert_eq!(serde_json::to_value(&reopened.snapshot).unwrap(), expected);
        drop(reopened);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn edge_panel_can_save_split_without_access_to_other_window_settings() {
        for changes in [
            serde_json::json!({"panelSplit": 65}),
            serde_json::json!({"panelSplit": 30, "pinned": true, "ddlSort": "priority"}),
        ] {
            let action = Action::UpdateSettings { changes };
            assert!(authorize_mutation("edge-panel", &action).is_ok());
            assert!(authorize_mutation("console", &action).is_ok());
            assert!(authorize_mutation("edge-handle", &action).is_err());
            assert!(authorize_mutation("unknown", &action).is_err());
        }
        for changes in [
            serde_json::json!({"panelSplit": 65, "panelWidth": 400}),
            serde_json::json!({"panelSplit": 65, "edgeEnabled": false}),
            serde_json::json!({"panelSplit": 65, "usageGuideSeen": true}),
        ] {
            assert!(authorize_mutation("edge-panel", &Action::UpdateSettings { changes }).is_err());
        }
    }

    #[test]
    fn handle_unknown_windows_and_edge_settings_cannot_escalate() {
        let completed = Action::SetCompleted {
            id: "test".into(),
            completed: true,
            expected_revision: 1,
        };
        assert!(authorize_mutation("edge-panel", &completed).is_ok());
        assert!(authorize_mutation("edge-handle", &completed).is_err());
        assert!(authorize_mutation("untrusted", &completed).is_err());
        let allowed = Action::UpdateSettings {
            changes: serde_json::json!({"pinned":true,"ddlSort":"priority"}),
        };
        let rejected = Action::UpdateSettings {
            changes: serde_json::json!({"pinned":true,"edgeEnabled":false}),
        };
        assert!(authorize_mutation("edge-panel", &allowed).is_ok());
        assert!(authorize_mutation("edge-panel", &rejected).is_err());
        assert!(authorize_mutation("console", &rejected).is_ok());
        assert!(authorize_mutation(
            "console",
            &Action::ResetDemo {
                date: "2026-09-25".into()
            }
        )
        .is_err());
    }

    #[test]
    fn task_order_permissions_match_each_surface() {
        let today = Action::ReorderToday {
            date: "2026-09-28".into(),
            task_ids: vec!["task".into()],
        };
        let deadlines = Action::ReorderTasks {
            scope: "deadlines".into(),
            task_ids: vec!["task".into()],
        };
        let all = Action::ReorderTasks {
            scope: "all".into(),
            task_ids: vec!["task".into()],
        };
        for action in [&today, &deadlines, &all] {
            assert!(authorize_mutation("console", action).is_ok());
            assert!(authorize_mutation("edge-handle", action).is_err());
            assert!(authorize_mutation("untrusted", action).is_err());
        }
        assert!(authorize_mutation("edge-panel", &today).is_ok());
        assert!(authorize_mutation("edge-panel", &deadlines).is_ok());
        assert!(authorize_mutation("edge-panel", &all).is_err());
    }

    #[test]
    fn exports_are_unique_complete_and_private_without_overwriting_existing_files() {
        let directory =
            std::env::temp_dir().join(format!("sidetask-export-test-{}", uuid::Uuid::new_v4()));
        let first = write_export(&directory, "{\"tasks\":[]}").unwrap();
        let second = write_export(&directory, "{\"tasks\":[1]}").unwrap();
        assert_ne!(first, second);
        assert_eq!(first.parent(), Some(directory.as_path()));
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "{\"tasks\":[]}");
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "{\"tasks\":[1]}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&first).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn capability_grants_keep_internal_events_and_console_data_operations_private() {
        let shared: Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        let tasks: Value =
            serde_json::from_str(include_str!("../capabilities/tasks.json")).unwrap();
        let console: Value =
            serde_json::from_str(include_str!("../capabilities/console.json")).unwrap();
        for capability in [&shared, &tasks, &console] {
            assert!(capability.get("remote").is_none());
            assert!(!capability["windows"]
                .as_array()
                .unwrap()
                .contains(&Value::String("*".into())));
            for permission in capability["permissions"].as_array().unwrap() {
                let permission = permission.as_str().unwrap();
                assert!(![
                    "core:default",
                    "core:event:default",
                    "core:event:allow-emit",
                    "core:event:allow-emit-to"
                ]
                .contains(&permission));
            }
        }
        assert_eq!(console["windows"], serde_json::json!(["console"]));
        assert!(!tasks["windows"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("edge-handle")));
        for command in [
            "allow-mutate",
            "allow-export-backup",
            "allow-preview-restore",
            "allow-restore-backup",
            "allow-resolve-exit",
            "allow-get-startup-recovery",
            "allow-get-console-position-status",
            "allow-get-usage-guide-seen",
            "allow-acknowledge-usage-guide",
            "allow-recover-startup-backup",
            "allow-restart-after-recovery",
            "allow-sync-status",
            "allow-sync-sign-in",
            "allow-sync-sign-out",
            "allow-sync-now",
            "allow-sync-resolve",
        ] {
            assert!(!shared["permissions"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(command)));
        }
        for command in [
            "allow-get-startup-recovery",
            "allow-get-console-position-status",
            "allow-get-usage-guide-seen",
            "allow-acknowledge-usage-guide",
            "allow-recover-startup-backup",
            "allow-restart-after-recovery",
            "allow-sync-status",
            "allow-sync-sign-in",
            "allow-sync-sign-out",
            "allow-sync-now",
            "allow-sync-resolve",
        ] {
            assert!(!tasks["permissions"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(command)));
            assert!(console["permissions"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(command)));
        }
    }

    #[test]
    fn failed_startup_does_not_return_an_empty_task_service_or_replace_corrupt_bytes() {
        let directory =
            std::env::temp_dir().join(format!("sidetask-startup-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("sidetask.sqlite3");
        let bytes = b"synthetic damaged database - preserve this evidence";
        std::fs::write(&path, bytes).unwrap();
        assert!(initialize_task_state(&directory).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
