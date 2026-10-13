use super::{
    credentials::{Credentials, NativeCredentials},
    data::{merge, ConflictChoice, MergeConflict, SyncData},
    state::{SyncBinding, SyncConfig, SyncState},
    transport::{Cloud, CloudError, Supabase},
};
use crate::{application::TaskService, task_state};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};
use tauri::{Emitter, Manager};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub phase: String,
    pub enabled: bool,
    pub email: Option<String>,
    pub project_url: Option<String>,
    pub last_synced_at: Option<String>,
    pub pending: bool,
    pub error: Option<String>,
    pub conflicts: Vec<MergeConflict>,
}
#[derive(Clone)]
struct ConflictContext {
    local: SyncData,
    remote_revision: u64,
    conflicts: Vec<MergeConflict>,
}
#[derive(Default)]
struct View {
    phase: String,
    error: Option<String>,
    conflict: Option<ConflictContext>,
}
#[derive(Default)]
struct Signal {
    stop: bool,
    wake: bool,
    suspended: bool,
}
pub struct SyncRuntime {
    gate: Mutex<()>,
    view: Mutex<View>,
    signal: Arc<(Mutex<Signal>, Condvar)>,
    credentials: Box<dyn Credentials>,
    cloud: Box<dyn Cloud>,
}
impl SyncRuntime {
    #[cfg(not(target_os = "android"))]
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            gate: Mutex::new(()),
            view: Mutex::new(View::default()),
            signal: Arc::new((Mutex::new(Signal::default()), Condvar::new())),
            credentials: Box::new(NativeCredentials),
            cloud: Box::new(Supabase::new()?),
        })
    }
    #[cfg(target_os = "android")]
    pub fn new_android(app: tauri::AppHandle) -> Result<Self, String> {
        let cloud = Supabase::new_android(&app)?;
        Ok(Self {
            gate: Mutex::new(()),
            view: Mutex::new(View::default()),
            signal: Arc::new((Mutex::new(Signal::default()), Condvar::new())),
            credentials: Box::new(NativeCredentials(app)),
            cloud: Box::new(cloud),
        })
    }
    #[cfg(target_os = "android")]
    pub fn set_active(&self, active: bool) {
        let (mutex, condvar) = &*self.signal;
        if let Ok(mut signal) = mutex.lock() {
            signal.suspended = !active;
            if active {
                signal.wake = true;
            }
            condvar.notify_all();
        }
    }
    pub fn start(app: tauri::AppHandle) {
        let signal = app.state::<Self>().signal.clone();
        app.state::<Self>().wake();
        std::thread::spawn(move || {
            let (mutex, condvar) = &*signal;
            loop {
                let Ok(state) = mutex.lock() else { break };
                let Ok((mut state, _)) =
                    condvar
                        .wait_timeout_while(state, Duration::from_secs(30), |s| !s.wake && !s.stop)
                else {
                    break;
                };
                if state.stop {
                    break;
                }
                if state.suspended {
                    state.wake = false;
                    continue;
                }
                let woke = state.wake;
                state.wake = false;
                drop(state);
                if woke {
                    // Coalesce rapid edits without delaying the local SQLite write.
                    let Ok(state) = mutex.lock() else { break };
                    let Ok((state, _)) =
                        condvar.wait_timeout_while(state, Duration::from_secs(2), |s| !s.stop)
                    else {
                        break;
                    };
                    if state.stop {
                        break;
                    }
                    if state.suspended {
                        continue;
                    }
                }
                let runtime = app.state::<Self>();
                let Ok(_gate) = runtime.gate.try_lock() else {
                    continue;
                };
                let status = runtime.status(&app);
                if status.as_ref().is_ok_and(|s| {
                    s.enabled && s.phase != "signedOut" && (woke || s.phase != "conflict")
                }) {
                    let _ = runtime.run_cycle(&app, None);
                }
            }
        });
    }
    pub fn wake(&self) {
        let (mutex, condvar) = &*self.signal;
        if let Ok(mut signal) = mutex.lock() {
            signal.wake = true;
            condvar.notify_one();
        }
    }
    pub fn stop(&self) {
        let (mutex, condvar) = &*self.signal;
        if let Ok(mut signal) = mutex.lock() {
            signal.stop = true;
            condvar.notify_all();
        }
    }
    fn set_view(&self, phase: &str, error: Option<String>, conflict: Option<ConflictContext>) {
        if let Ok(mut view) = self.view.lock() {
            *view = View {
                phase: phase.into(),
                error,
                conflict,
            };
        }
    }
    pub fn status(&self, app: &tauri::AppHandle) -> Result<SyncStatus, String> {
        let state = task_state(app)?;
        let service = state.service.lock().map_err(|_| "本地任务服务暂不可用。")?;
        let sync = &service.sync;
        let view = self.view.lock().map_err(|_| "同步服务暂不可用。")?;
        let phase = if !sync.enabled {
            "disabled"
        } else if view.phase.is_empty() {
            "idle"
        } else {
            &view.phase
        };
        Ok(SyncStatus {
            phase: phase.into(),
            enabled: sync.enabled,
            email: sync.binding.as_ref().map(|b| b.email.clone()),
            project_url: sync.binding.as_ref().map(|b| b.config.project_url.clone()),
            last_synced_at: sync.last_synced_at.clone(),
            pending: sync.enabled && SyncData::from_snapshot(&service.snapshot) != sync.baseline,
            error: view.error.clone(),
            conflicts: view
                .conflict
                .as_ref()
                .map(|c| c.conflicts.clone())
                .unwrap_or_default(),
        })
    }
    fn notify(&self, app: &tauri::AppHandle) -> Result<SyncStatus, String> {
        let status = self.status(app)?;
        // Conflict values and account identity are only available to the console.
        let _ = app.emit_to("console", "sidetask:sync-changed", &status);
        Ok(status)
    }
    pub fn sign_in(
        &self,
        app: &tauri::AppHandle,
        config: SyncConfig,
        email: String,
        password: String,
        merge_local: bool,
    ) -> Result<SyncStatus, String> {
        let _gate = self.gate.lock().map_err(|_| "同步服务暂不可用。")?;
        config.validate()?;
        if !merge_local {
            return Err("请确认将本机任务合并到此账号后再连接。".into());
        }
        if email.trim().is_empty()
            || email.len() > 320
            || password.is_empty()
            || password.len() > 4096
        {
            return Err("请填写邮箱和密码。".into());
        }
        let state = task_state(app)?;
        crate::platform::exit::ensure_running(&state)?;
        let old = state
            .service
            .lock()
            .map_err(|_| "本地任务服务暂不可用。")?
            .sync
            .clone();
        if old.binding.as_ref().is_some_and(|b| {
            b.config.project_url != config.project_url
                || !b.email.eq_ignore_ascii_case(email.trim())
        }) {
            return Err("请先断开当前账号，再连接其他项目或账号；本机任务会保留。".into());
        }
        self.set_view("syncing", None, None);
        let _ = self.notify(app);
        let result = (|| {
            let (user_id, email, session) = self
                .cloud
                .login(&config, email.trim(), &password)
                .map_err(|e| e.message())?;
            let binding = SyncBinding {
                user_id,
                email,
                config,
            };
            if old
                .binding
                .as_ref()
                .is_some_and(|b| b.user_id != binding.user_id)
            {
                return Err("账号标识已变化，请先断开同步再连接。".into());
            }
            let mut next = old.clone();
            next.binding = Some(binding.clone());
            next.enabled = true;
            next.validate()?;
            let previous = self.credentials.read(&old, &binding)?;
            let mut service = state.service.lock().map_err(|_| "本地任务服务暂不可用。")?;
            crate::platform::exit::ensure_running(&state)?;
            self.credentials.write(&next, &binding, &session)?;
            let saved = service.persist_sync(next);
            if let Err(error) = saved {
                // Restore the previous secure entry when SQLite could not bind this login.
                let rollback = if let Some(previous) = previous {
                    self.credentials.write(&old, &binding, &previous)
                } else {
                    self.credentials.delete(&old, &binding)
                };
                return Err(if rollback.is_err() {
                    format!("{error} 系统凭据回滚失败，请重试登录或断开。")
                } else {
                    error
                });
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.set_view(
                if old.enabled { "signedOut" } else { "error" },
                Some(error.clone()),
                None,
            );
            let _ = self.notify(app);
            return Err(error);
        }
        // A network failure after a successful login keeps the durable binding and pending edits.
        self.run_cycle(app, None)
    }
    pub fn sign_out(&self, app: &tauri::AppHandle) -> Result<SyncStatus, String> {
        let _gate = self.gate.lock().map_err(|_| "同步服务暂不可用。")?;
        let state = task_state(app)?;
        crate::platform::exit::ensure_running(&state)?;
        let old = state
            .service
            .lock()
            .map_err(|_| "本地任务服务暂不可用。")?
            .sync
            .clone();
        // Disable before removing secure credentials. A failed removal is retryable, never restarts uploads.
        if let Some(binding) = &old.binding {
            let mut paused = old.clone();
            paused.enabled = false;
            {
                let mut service = state.service.lock().map_err(|_| "本地任务服务暂不可用。")?;
                crate::platform::exit::ensure_running(&state)?;
                service.persist_sync(paused)?;
            }
            if let Err(error) = self.credentials.delete(&old, binding) {
                self.set_view("error", Some(error.clone()), None);
                let _ = self.notify(app);
                return Err(error);
            }
        }
        let next = SyncState {
            device_id: old.device_id,
            ..SyncState::default()
        };
        {
            let mut service = state.service.lock().map_err(|_| "本地任务服务暂不可用。")?;
            crate::platform::exit::ensure_running(&state)?;
            service.persist_sync(next)?;
        }
        self.set_view("disabled", None, None);
        self.notify(app)
    }
    pub fn sync_now(&self, app: &tauri::AppHandle) -> Result<SyncStatus, String> {
        let _gate = self.gate.lock().map_err(|_| "同步服务暂不可用。")?;
        self.run_cycle(app, None)
    }
    pub fn resolve(
        &self,
        app: &tauri::AppHandle,
        choices: BTreeMap<String, ConflictChoice>,
    ) -> Result<SyncStatus, String> {
        let _gate = self.gate.lock().map_err(|_| "同步服务暂不可用。")?;
        let context = self
            .view
            .lock()
            .map_err(|_| "同步服务暂不可用。")?
            .conflict
            .clone()
            .ok_or("没有待处理的同步冲突。")?;
        if choices.len() != context.conflicts.len()
            || context
                .conflicts
                .iter()
                .any(|c| !choices.contains_key(&c.id))
        {
            return Err("请为每项冲突选择保留本机或云端内容。".into());
        }
        self.run_cycle(app, Some((context, choices)))
    }
    fn run_cycle(
        &self,
        app: &tauri::AppHandle,
        resolution: Option<(ConflictContext, BTreeMap<String, ConflictChoice>)>,
    ) -> Result<SyncStatus, String> {
        let state = task_state(app)?;
        if let Err(error) = crate::platform::exit::ensure_running(&state) {
            self.set_view("error", Some(error), None);
            return self.notify(app);
        }
        let enabled = state
            .service
            .lock()
            .map_err(|_| "本地任务服务暂不可用。")?
            .sync
            .enabled;
        if !enabled {
            return self.notify(app);
        }
        self.set_view("syncing", None, None);
        let _ = self.notify(app);
        let result = cycle_with_guard(
            &state.service,
            self.cloud.as_ref(),
            self.credentials.as_ref(),
            resolution.as_ref(),
            &|| crate::platform::exit::ensure_running(&state),
        );
        match result {
            Ok(CycleResult::Conflict(context)) => self.set_view("conflict", None, Some(context)),
            Ok(CycleResult::Committed(revision)) => {
                self.set_view("idle", None, None);
                if let Some(revision) = revision {
                    let _ = app.emit("sidetask:changed", serde_json::json!({"revision":revision}));
                }
            }
            Err(error) => {
                let phase = if matches!(error, CloudError::Unauthorized) {
                    "signedOut"
                } else {
                    "error"
                };
                self.set_view(phase, Some(error.message()), None);
            }
        }
        self.notify(app)
    }
}
enum CycleResult {
    Conflict(ConflictContext),
    Committed(Option<u64>),
}
#[cfg(test)]
fn cycle(
    service: &Mutex<TaskService>,
    cloud: &dyn Cloud,
    credentials: &dyn Credentials,
    resolution: Option<&(ConflictContext, BTreeMap<String, ConflictChoice>)>,
) -> Result<CycleResult, CloudError> {
    cycle_with_guard(service, cloud, credentials, resolution, &|| Ok(()))
}
fn cycle_with_guard(
    service: &Mutex<TaskService>,
    cloud: &dyn Cloud,
    credentials: &dyn Credentials,
    resolution: Option<&(ConflictContext, BTreeMap<String, ConflictChoice>)>,
    guard: &dyn Fn() -> Result<(), String>,
) -> Result<CycleResult, CloudError> {
    guard().map_err(CloudError::Message)?;
    let (original, local, snapshot) = {
        let service = service
            .lock()
            .map_err(|_| CloudError::Message("本地任务服务暂不可用。".into()))?;
        (
            service.sync.clone(),
            SyncData::from_snapshot(&service.snapshot),
            service.snapshot.clone(),
        )
    };
    let binding = original.binding.as_ref().ok_or(CloudError::Unauthorized)?;
    let mut session = credentials
        .read(&original, binding)
        .map_err(CloudError::Message)?
        .ok_or(CloudError::Unauthorized)?;
    if session.expires_at <= chrono::Utc::now().timestamp() + 60 {
        session = cloud.refresh(&binding.config, &binding.user_id, &session)?;
        // Refresh tokens rotate. Save the replacement before making any data requests.
        {
            let _service = service
                .lock()
                .map_err(|_| CloudError::Message("本地任务服务暂不可用。".into()))?;
            guard().map_err(CloudError::Message)?;
            credentials
                .write(&original, binding, &session)
                .map_err(CloudError::Message)?;
        }
    }
    let head = match cloud.head(&binding.config, &session) {
        Err(CloudError::Unauthorized) => {
            session = cloud.refresh(&binding.config, &binding.user_id, &session)?;
            {
                let _service = service
                    .lock()
                    .map_err(|_| CloudError::Message("本地任务服务暂不可用。".into()))?;
                guard().map_err(CloudError::Message)?;
                credentials
                    .write(&original, binding, &session)
                    .map_err(CloudError::Message)?;
            }
            cloud.head(&binding.config, &session)?
        }
        result => result?,
    };
    if head < original.remote_revision {
        return Err(CloudError::Message(
            "云端版本回退或数据被重置，未自动上传；请先导出备份，确认项目数据后重新连接。".into(),
        ));
    }
    if head == original.remote_revision && head != 0 && local == original.baseline {
        return Ok(CycleResult::Committed(None));
    }
    let remote = if head == original.remote_revision && head != 0 {
        super::transport::Document {
            revision: head,
            data: original.baseline.clone(),
        }
    } else {
        cloud.get(&binding.config, &session)?
    };
    if remote.revision < original.remote_revision {
        return Err(CloudError::Stale);
    }
    let initial = merge(&original.baseline, &local, &remote.data, &BTreeMap::new())
        .map_err(CloudError::Message)?;
    let merged = if initial.conflicts.is_empty() {
        initial.data
    } else {
        let context = ConflictContext {
            local: local.clone(),
            remote_revision: remote.revision,
            conflicts: initial.conflicts.clone(),
        };
        if let Some((expected, choices)) = resolution {
            if expected.local == local
                && expected.remote_revision == remote.revision
                && expected.conflicts == initial.conflicts
            {
                let result = merge(&original.baseline, &local, &remote.data, choices)
                    .map_err(CloudError::Message)?;
                if !result.conflicts.is_empty() {
                    return Ok(CycleResult::Conflict(context));
                }
                result.data
            } else {
                return Ok(CycleResult::Conflict(context));
            }
        } else {
            return Ok(CycleResult::Conflict(context));
        }
    };
    // Verify capacity and local rules before publishing a document to the other device.
    merged.apply_to(&snapshot).map_err(CloudError::Message)?;
    let revision = if remote.revision == 0 || merged != remote.data {
        guard().map_err(CloudError::Message)?;
        cloud.put(&binding.config, &session, remote.revision, &merged)?
    } else {
        remote.revision
    };
    let mut service = service
        .lock()
        .map_err(|_| CloudError::Message("本地任务服务暂不可用。".into()))?;
    guard().map_err(CloudError::Message)?;
    if service.sync.binding != original.binding || !service.sync.enabled {
        return Err(CloudError::Message("同步账号已更改，请重新同步。".into()));
    }
    let current = SyncData::from_snapshot(&service.snapshot);
    let to_apply = if current == local {
        merged.clone()
    } else {
        // Preserve edits committed while the HTTPS request was in flight.
        let late =
            merge(&local, &current, &merged, &BTreeMap::new()).map_err(CloudError::Message)?;
        if !late.conflicts.is_empty() {
            return Err(CloudError::Message(
                "同步期间本地任务发生新修改，已保留；请重新同步并确认冲突。".into(),
            ));
        }
        late.data
    };
    let mut next = original;
    next.baseline = merged;
    next.remote_revision = revision;
    next.last_synced_at = Some(chrono::Utc::now().to_rfc3339());
    let before = service.snapshot.revision;
    let committed = service
        .commit_sync(&to_apply, next, before)
        .map_err(CloudError::Message)?;
    Ok(CycleResult::Committed(
        (before != committed.revision).then_some(committed.revision),
    ))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
