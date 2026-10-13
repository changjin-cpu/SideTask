mod draft;

use crate::{
    application::TaskService,
    domain::{Action, Snapshot},
    infrastructure::SqliteRepository,
    sync,
};
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use tauri::{Emitter, Manager};

pub struct AppState {
    pub service: Mutex<TaskService>,
    pub running: AtomicBool,
    pub drafts: Mutex<draft::DraftStore>,
}
pub(crate) fn task_state(app: &tauri::AppHandle) -> Result<tauri::State<'_, AppState>, String> {
    app.try_state::<AppState>()
        .ok_or_else(|| "本地数据库无法打开，原数据已保留，请勿卸载应用。".into())
}
fn authorize(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "console" {
        Ok(())
    } else {
        Err("此窗口无权执行该操作。".into())
    }
}
#[tauri::command]
fn get_snapshot(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<Snapshot, String> {
    authorize(&window)?;
    task_state(&app)?
        .service
        .lock()
        .map(|s| s.snapshot.clone())
        .map_err(|_| "本地任务暂不可用。".into())
}
#[tauri::command]
fn mutate(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    action: Value,
    expected_revision: u64,
) -> Result<Snapshot, String> {
    authorize(&window)?;
    let action: Action = serde_json::from_value(action).map_err(|_| "操作格式无效。")?;
    if matches!(action, Action::ResetDemo { .. }) {
        return Err("正式应用不能覆盖演示数据。".into());
    }
    let state = task_state(&app)?;
    crate::platform::exit::ensure_running(&state)?;
    let result = state
        .service
        .lock()
        .map_err(|_| "本地任务暂不可用。")?
        .mutate(action, expected_revision)?;
    let _ = app.emit(
        "sidetask:changed",
        serde_json::json!({"revision":result.revision}),
    );
    if let Some(runtime) = app.try_state::<sync::SyncRuntime>() {
        runtime.wake();
    }
    Ok(result)
}
#[tauri::command]
fn sync_status(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<sync::SyncStatus, String> {
    authorize(&window)?;
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
    authorize(&window)?;
    let config = sync::state::SyncConfig::new(project_url, publishable_key)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.try_state::<sync::SyncRuntime>()
            .ok_or("同步服务尚未就绪。")?
            .sign_in(&app, config, email, password, merge_local)
    })
    .await
    .map_err(|_| "登录中断，请重试。")?
}
#[tauri::command]
async fn sync_sign_out(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<sync::SyncStatus, String> {
    authorize(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.try_state::<sync::SyncRuntime>()
            .ok_or("同步服务尚未就绪。")?
            .sign_out(&app)
    })
    .await
    .map_err(|_| "断开中断，请重试。")?
}
#[tauri::command]
async fn sync_now(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<sync::SyncStatus, String> {
    authorize(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.try_state::<sync::SyncRuntime>()
            .ok_or("同步服务尚未就绪。")?
            .sync_now(&app)
    })
    .await
    .map_err(|_| "同步中断，请重试。")?
}
#[tauri::command]
async fn sync_resolve(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    choices: std::collections::BTreeMap<String, sync::data::ConflictChoice>,
) -> Result<sync::SyncStatus, String> {
    authorize(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.try_state::<sync::SyncRuntime>()
            .ok_or("同步服务尚未就绪。")?
            .resolve(&app, choices)
    })
    .await
    .map_err(|_| "冲突处理中断，请重试。")?
}
#[tauri::command]
fn mobile_active(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    active: bool,
) -> Result<(), String> {
    authorize(&window)?;
    if let Some(runtime) = app.try_state::<sync::SyncRuntime>() {
        runtime.set_active(active);
    }
    Ok(())
}
#[tauri::command]
fn get_startup_recovery(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Value, String> {
    authorize(&window)?;
    Ok(
        serde_json::json!({"required": app.try_state::<AppState>().is_none(), "message": app.try_state::<StartupFailure>().map(|e| e.0.clone())}),
    )
}
struct StartupFailure(String);

#[tauri::command]
fn mobile_load_draft(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<Option<Value>, String> {
    authorize(&window)?;
    task_state(&app)?
        .drafts
        .lock()
        .map_err(|_| "草稿服务暂不可用。")?
        .read()
}
#[tauri::command]
fn mobile_save_draft(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    draft: Option<Value>,
) -> Result<(), String> {
    authorize(&window)?;
    task_state(&app)?
        .drafts
        .lock()
        .map_err(|_| "草稿服务暂不可用。")?
        .write(draft)
}

#[tauri::command]
async fn mobile_time_zone(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<String, String> {
    authorize(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<tauri_plugin_sidetask_mobile::SecureStore<tauri::Wry>>()
            .time_zone()
    })
    .await
    .map_err(|_| "无法读取系统时区。".to_string())?
}

#[cfg(all(feature = "mobile-smoke", not(debug_assertions)))]
compile_error!("mobile-smoke must never be enabled in a release APK");

// Dedicated emulator builds can exercise Keystore across process restarts
// without reading any real account or accepting arbitrary session contents.
#[cfg(feature = "mobile-smoke")]
#[tauri::command]
async fn mobile_network_probe(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    project_url: String,
    publishable_key: String,
) -> Result<u16, String> {
    authorize(&window)?;
    let config = sync::state::SyncConfig::new(project_url, publishable_key)?;
    tauri::async_runtime::spawn_blocking(move || {
        sync::transport::Supabase::new_android(&app)?.probe(&config)
    })
    .await
    .map_err(|_| "Android HTTPS probe interrupted".to_string())?
}

#[cfg(feature = "mobile-smoke")]
#[tauri::command]
async fn mobile_credential_probe(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    action: String,
) -> Result<bool, String> {
    authorize(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        use sync::credentials::{Credentials, NativeCredentials, Session};
        let state = sync::state::SyncState {
            device_id: "00000000-0000-4000-8000-000000000051".into(),
            ..Default::default()
        };
        let binding = sync::state::SyncBinding {
            config: sync::state::SyncConfig::new(
                "https://synthetic.supabase.co".into(),
                "sb_publishable_synthetic_test_only".into(),
            )?,
            user_id: "00000000-0000-4000-8000-000000000052".into(),
            email: "synthetic@example.test".into(),
        };
        let store = NativeCredentials(app);
        match action.as_str() {
            "write" => {
                store.write(
                    &state,
                    &binding,
                    &Session {
                        access_token: "synthetic-android-probe-access".into(),
                        refresh_token: "synthetic-android-probe-refresh".into(),
                        expires_at: 2147483647,
                    },
                )?;
                Ok(true)
            }
            "read" => Ok(store.read(&state, &binding)?.is_some_and(|session| {
                session.access_token == "synthetic-android-probe-access"
                    && session.refresh_token == "synthetic-android-probe-refresh"
            })),
            "delete" => {
                store.delete(&state, &binding)?;
                Ok(store.read(&state, &binding)?.is_none())
            }
            _ => Err("Unknown emulator credential probe".into()),
        }
    })
    .await
    .map_err(|_| "Emulator credential probe interrupted".to_string())?
}

#[tauri::mobile_entry_point]
pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_sidetask_mobile::init());
    #[cfg(feature = "mobile-smoke")]
    let builder = builder.invoke_handler(tauri::generate_handler![
        get_snapshot,
        mutate,
        sync_status,
        sync_sign_in,
        sync_sign_out,
        sync_now,
        sync_resolve,
        mobile_active,
        get_startup_recovery,
        mobile_time_zone,
        mobile_network_probe,
        mobile_load_draft,
        mobile_save_draft,
        mobile_credential_probe
    ]);
    #[cfg(not(feature = "mobile-smoke"))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        get_snapshot,
        mutate,
        sync_status,
        sync_sign_in,
        sync_sign_out,
        sync_now,
        sync_resolve,
        mobile_active,
        mobile_time_zone,
        mobile_load_draft,
        mobile_save_draft,
        get_startup_recovery
    ]);
    let app = builder
        .setup(|app| {
            let result = (|| -> Result<TaskService, String> {
                let data_dir = app
                    .path()
                    .app_data_dir()
                    .map_err(|_| "无法访问本地目录。")?;
                std::fs::create_dir_all(&data_dir).map_err(|_| "无法创建本地目录。")?;
                TaskService::new(Box::new(SqliteRepository::open(
                    &data_dir.join("sidetask.sqlite3"),
                )?))
            })();
            match result {
                Ok(service) => {
                    app.manage(AppState {
                        service: Mutex::new(service),
                        running: AtomicBool::new(true),
                        drafts: Mutex::new(draft::DraftStore::new(&app.path().app_data_dir()?)),
                    });
                    match sync::SyncRuntime::new_android(app.handle().clone()) {
                        Ok(runtime) => {
                            app.manage(runtime);
                            sync::SyncRuntime::start(app.handle().clone());
                        }
                        Err(_) => {
                            app.manage(StartupFailure(
                                "同步网络初始化失败，本地任务仍可使用。".into(),
                            ));
                        }
                    }
                }
                Err(message) => {
                    app.manage(StartupFailure(message));
                }
            }
            tauri::WebviewWindowBuilder::new(
                app,
                "console",
                tauri::WebviewUrl::App("index.html?surface=mobile".into()),
            )
            .title("侧笺 · SideTask")
            .build()?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("SideTask could not start; existing data preserved");
    app.run(|app, event| {
        if let Some(runtime) = app.try_state::<sync::SyncRuntime>() {
            match event {
                tauri::RunEvent::Resumed => runtime.set_active(true),
                tauri::RunEvent::WindowEvent {
                    event: tauri::WindowEvent::Focused(active),
                    ..
                } => runtime.set_active(active),
                tauri::RunEvent::Exit => {
                    if let Some(state) = app.try_state::<AppState>() {
                        state.running.store(false, Ordering::Release);
                    }
                    runtime.stop();
                }
                _ => {}
            }
        }
    });
}
