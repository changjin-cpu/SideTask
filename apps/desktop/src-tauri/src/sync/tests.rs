//! Deterministic engine integration tests. Only temporary SQLite databases and
//! in-memory credentials are used; no native credential store or network calls.
use super::*;
use crate::domain::{Action, Snapshot};
use crate::infrastructure::SqliteRepository;
use crate::sync::credentials::{MemoryCredentials, Session};
use crate::sync::transport::Document;
use serde_json::{json, Value};
use std::path::PathBuf;

const DATE: &str = "2026-10-04";
const USER_ID: &str = "68ecf47c-22cb-45df-86e4-771d5bdbbaad";

#[test]
fn three_clients_converge_after_phone_restart_with_independent_edits_and_orders() {
    let temp = TempClients::new();
    let cloud = MockCloud::new();
    let credentials = credentials();
    let mac = temp.open("mac");
    let windows = temp.open("windows");
    let phone = temp.open("android");
    let ids = seed(&mac);
    for client in [&mac, &windows, &phone] {
        sync(client, &cloud, &credentials);
    }
    let phone_device = phone.lock().unwrap().sync.device_id.clone();
    assert_ne!(phone_device, mac.lock().unwrap().sync.device_id);
    assert_ne!(phone_device, windows.lock().unwrap().sync.device_id);
    edit(&mac, &ids[0], json!({"title":"Mac title"}));
    edit(&windows, &ids[0], json!({"notes":"Windows notes"}));
    edit(&phone, &ids[0], json!({"priority":"high"}));
    mutate(
        &mac,
        Action::PlanTask {
            id: ids[2].clone(),
            planned: false,
            date: DATE.into(),
        },
    );
    mutate(
        &windows,
        Action::ReorderTasks {
            scope: "all".into(),
            task_ids: vec![ids[2].clone(), ids[0].clone(), ids[1].clone()],
        },
    );
    mutate(
        &phone,
        Action::ReorderTasks {
            scope: "deadlines".into(),
            task_ids: vec![ids[1].clone(), ids[2].clone(), ids[0].clone()],
        },
    );
    for (client, style) in [(&mac, "paper"), (&windows, "studio"), (&phone, "mono")] {
        mutate(
            client,
            Action::UpdateSettings {
                changes: json!({"uiStyle":style}),
            },
        );
    }
    let pending_phone = state(&phone);
    drop(phone);
    let phone = temp.open("android");
    assert_eq!(state(&phone), pending_phone);
    assert_eq!(phone.lock().unwrap().sync.device_id, phone_device);
    for _ in 0..3 {
        for client in [&phone, &mac, &windows] {
            sync(client, &cloud, &credentials);
        }
    }
    for client in [&mac, &windows, &phone] {
        assert_eq!(data(client), cloud.document().data);
        assert!(!pending(client));
        let committed = data(client);
        let task = committed
            .tasks
            .iter()
            .find(|task| task.id == ids[0])
            .unwrap();
        assert_eq!(task.title, "Mac title");
        assert_eq!(task.notes, "Windows notes");
        assert_eq!(task.priority, "high");
        assert!(!committed
            .plans
            .iter()
            .any(|plan| plan.task_id == ids[2] && plan.date == DATE));
        assert_eq!(
            committed.task_order,
            vec![ids[2].clone(), ids[0].clone(), ids[1].clone()]
        );
        assert_eq!(
            committed.deadline_order,
            vec![ids[1].clone(), ids[2].clone(), ids[0].clone()]
        );
    }
    assert_eq!(snapshot(&mac).settings.ui_style, "paper");
    assert_eq!(snapshot(&windows).settings.ui_style, "studio");
    assert_eq!(snapshot(&phone).settings.ui_style, "mono");
}

#[test]
fn third_client_changes_invalidate_a_phone_conflict_choice() {
    let temp = TempClients::new();
    let cloud = MockCloud::new();
    let credentials = credentials();
    let mac = temp.open("mac");
    let windows = temp.open("windows");
    let phone = temp.open("android");
    let ids = seed(&mac);
    for client in [&mac, &windows, &phone] {
        sync(client, &cloud, &credentials);
    }
    edit(&mac, &ids[0], json!({"title":"Mac version"}));
    edit(&phone, &ids[0], json!({"title":"Phone draft"}));
    sync(&mac, &cloud, &credentials);
    let first = conflict(&phone, &cloud, &credentials, None);
    sync(&windows, &cloud, &credentials);
    edit(&windows, &ids[0], json!({"title":"Windows newer version"}));
    sync(&windows, &cloud, &credentials);
    let stale_choice = (first.clone(), choose(&first, ConflictChoice::Local));
    let newer = conflict(&phone, &cloud, &credentials, Some(&stale_choice));
    assert!(newer.remote_revision > first.remote_revision);
    assert!(pending(&phone));
    assert!(data(&phone)
        .tasks
        .iter()
        .any(|task| task.title == "Phone draft"));
    let choice = (newer.clone(), choose(&newer, ConflictChoice::Remote));
    assert!(matches!(
        cycle(&phone, &cloud, &credentials, Some(&choice)),
        Ok(CycleResult::Committed(_))
    ));
    for client in [&mac, &windows] {
        sync(client, &cloud, &credentials);
    }
    assert_eq!(data(&phone), data(&mac));
    assert_eq!(data(&phone), data(&windows));
}
type Hook = Box<dyn FnOnce() + Send>;

struct Server {
    document: Document,
    head_calls: usize,
    get_calls: usize,
    put_calls: usize,
    refresh_calls: usize,
    fail_head: Option<CloudError>,
    fail_refresh: bool,
    reject_put: bool,
    drop_put_response: bool,
    observed_tokens: Vec<String>,
}
struct MockCloud {
    server: Mutex<Server>,
    after_get: Mutex<Option<Hook>>,
    after_put: Mutex<Option<Hook>>,
}
impl MockCloud {
    fn new() -> Self {
        Self {
            server: Mutex::new(Server {
                document: Document {
                    revision: 0,
                    data: SyncData::empty(),
                },
                head_calls: 0,
                get_calls: 0,
                put_calls: 0,
                refresh_calls: 0,
                fail_head: None,
                fail_refresh: false,
                reject_put: false,
                drop_put_response: false,
                observed_tokens: Vec::new(),
            }),
            after_get: Mutex::new(None),
            after_put: Mutex::new(None),
        }
    }
    fn document(&self) -> Document {
        self.server.lock().unwrap().document.clone()
    }
    fn run_hook(hook: &Mutex<Option<Hook>>) {
        let hook = hook.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
    }
}
impl Cloud for MockCloud {
    fn login(
        &self,
        _: &SyncConfig,
        _: &str,
        _: &str,
    ) -> Result<(String, String, Session), CloudError> {
        panic!("cycle must use existing synthetic credentials");
    }
    fn refresh(&self, _: &SyncConfig, user_id: &str, _: &Session) -> Result<Session, CloudError> {
        assert_eq!(user_id, USER_ID);
        let mut server = self.server.lock().unwrap();
        server.refresh_calls += 1;
        if server.fail_refresh {
            return Err(CloudError::Unauthorized);
        }
        Ok(Session {
            access_token: format!("synthetic-access-{}", server.refresh_calls),
            refresh_token: format!("synthetic-refresh-{}", server.refresh_calls),
            expires_at: chrono::Utc::now().timestamp() + 3600,
        })
    }
    fn head(&self, _: &SyncConfig, session: &Session) -> Result<u64, CloudError> {
        let mut server = self.server.lock().unwrap();
        server.head_calls += 1;
        server.observed_tokens.push(session.access_token.clone());
        if let Some(error) = server.fail_head.take() {
            return Err(error);
        }
        Ok(server.document.revision)
    }
    fn get(&self, _: &SyncConfig, _: &Session) -> Result<Document, CloudError> {
        let document = {
            let mut server = self.server.lock().unwrap();
            server.get_calls += 1;
            server.document.clone()
        };
        Self::run_hook(&self.after_get);
        Ok(document)
    }
    fn put(
        &self,
        _: &SyncConfig,
        _: &Session,
        expected: u64,
        data: &SyncData,
    ) -> Result<u64, CloudError> {
        data.validate().map_err(CloudError::Message)?;
        let (revision, drop_response) = {
            let mut server = self.server.lock().unwrap();
            server.put_calls += 1;
            if std::mem::take(&mut server.reject_put) || expected != server.document.revision {
                return Err(CloudError::Stale);
            }
            server.document = Document {
                revision: expected + 1,
                data: data.clone(),
            };
            (
                server.document.revision,
                std::mem::take(&mut server.drop_put_response),
            )
        };
        Self::run_hook(&self.after_put);
        if drop_response {
            Err(CloudError::Message(
                "synthetic response lost after commit".into(),
            ))
        } else {
            Ok(revision)
        }
    }
}

struct TempClients(PathBuf);
impl TempClients {
    fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("sidetask-sync-engine-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        Self(directory)
    }
    fn open(&self, name: &str) -> Arc<Mutex<TaskService>> {
        let mut service = TaskService::new(Box::new(
            SqliteRepository::open(&self.0.join(format!("{name}.sqlite"))).unwrap(),
        ))
        .unwrap();
        if service.sync.binding.is_none() {
            let mut sync = service.sync.clone();
            sync.binding = Some(SyncBinding {
                user_id: USER_ID.into(),
                email: "synthetic@example.test".into(),
                config: SyncConfig {
                    project_url: "https://synthetic.supabase.co".into(),
                    publishable_key: "sb_publishable_synthetic_test_key".into(),
                },
            });
            sync.enabled = true;
            service.persist_sync(sync).unwrap();
        }
        Arc::new(Mutex::new(service))
    }
}
impl Drop for TempClients {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn credentials() -> MemoryCredentials {
    MemoryCredentials(Mutex::new(Some(Session {
        access_token: "synthetic-access-initial".into(),
        refresh_token: "synthetic-refresh-initial".into(),
        expires_at: chrono::Utc::now().timestamp() + 3600,
    })))
}
fn mutate(service: &Mutex<TaskService>, action: Action) {
    let mut service = service.lock().unwrap();
    let revision = service.snapshot.revision;
    service.mutate(action, revision).unwrap();
}
fn seed(service: &Mutex<TaskService>) -> Vec<String> {
    for title in ["A", "B", "C"] {
        mutate(
            service,
            Action::CreateTask {
                task: json!({"title":title,"addToToday":true,"dueDate":"2026-10-20","dueTimezone":"UTC"}),
                date: DATE.into(),
            },
        );
    }
    service
        .lock()
        .unwrap()
        .snapshot
        .tasks
        .iter()
        .map(|task| task.id.clone())
        .collect()
}
fn edit(service: &Mutex<TaskService>, id: &str, changes: Value) {
    let mut service = service.lock().unwrap();
    let task = service
        .snapshot
        .tasks
        .iter()
        .find(|task| task.id == id)
        .unwrap();
    let action = Action::UpdateTask {
        id: id.into(),
        changes,
        expected_revision: task.revision,
    };
    let revision = service.snapshot.revision;
    service.mutate(action, revision).unwrap();
}
fn lifecycle(service: &Mutex<TaskService>, id: &str, restore: bool) {
    let mut service = service.lock().unwrap();
    let task = service
        .snapshot
        .tasks
        .iter()
        .find(|task| task.id == id)
        .unwrap();
    let action = if restore {
        Action::RestoreTask {
            id: id.into(),
            expected_revision: task.revision,
        }
    } else {
        Action::TrashTask {
            id: id.into(),
            expected_revision: task.revision,
        }
    };
    let revision = service.snapshot.revision;
    service.mutate(action, revision).unwrap();
}
fn data(service: &Mutex<TaskService>) -> SyncData {
    SyncData::from_snapshot(&service.lock().unwrap().snapshot)
}
fn state(service: &Mutex<TaskService>) -> Value {
    serde_json::to_value(&service.lock().unwrap().sync).unwrap()
}
fn snapshot(service: &Mutex<TaskService>) -> Snapshot {
    service.lock().unwrap().snapshot.clone()
}
fn pending(service: &Mutex<TaskService>) -> bool {
    let service = service.lock().unwrap();
    SyncData::from_snapshot(&service.snapshot) != service.sync.baseline
}
fn sync(
    service: &Mutex<TaskService>,
    cloud: &MockCloud,
    credentials: &dyn Credentials,
) -> Option<u64> {
    match cycle(service, cloud, credentials, None)
        .unwrap_or_else(|error| panic!("{}", error.message()))
    {
        CycleResult::Committed(revision) => revision,
        CycleResult::Conflict(context) => panic!("unexpected conflicts: {:?}", context.conflicts),
    }
}
fn conflict(
    service: &Mutex<TaskService>,
    cloud: &MockCloud,
    credentials: &dyn Credentials,
    resolution: Option<&(ConflictContext, BTreeMap<String, ConflictChoice>)>,
) -> ConflictContext {
    match cycle(service, cloud, credentials, resolution)
        .unwrap_or_else(|error| panic!("{}", error.message()))
    {
        CycleResult::Conflict(context) => context,
        CycleResult::Committed(_) => panic!("expected an unresolved conflict"),
    }
}
fn choose(context: &ConflictContext, choice: ConflictChoice) -> BTreeMap<String, ConflictChoice> {
    context
        .conflicts
        .iter()
        .map(|conflict| (conflict.id.clone(), choice))
        .collect()
}

#[test]
fn two_sqlite_clients_merge_offline_edits_after_restart_and_keep_device_settings() {
    let temp = TempClients::new();
    let cloud = MockCloud::new();
    let credentials = credentials();
    let a = temp.open("mac");
    let b = temp.open("windows");
    let ids = seed(&a);
    sync(&a, &cloud, &credentials);
    assert!(sync(&b, &cloud, &credentials).is_some());
    let b_id = b.lock().unwrap().sync.device_id.clone();
    assert_ne!(a.lock().unwrap().sync.device_id, b_id);
    mutate(
        &a,
        Action::UpdateSettings {
            changes: json!({"theme":"dark","panelWidth":420}),
        },
    );
    mutate(
        &b,
        Action::UpdateSettings {
            changes: json!({"theme":"light","panelWidth":350}),
        },
    );
    assert!(!pending(&a) && !pending(&b));
    edit(&a, &ids[0], json!({"title":"Mac offline title"}));
    edit(&b, &ids[1], json!({"notes":"Windows offline note"}));
    let saved = state(&b);
    drop(b);
    let b = temp.open("windows");
    assert_eq!(state(&b), saved);
    assert_eq!(b.lock().unwrap().sync.device_id, b_id);
    assert!(pending(&b));
    sync(&a, &cloud, &credentials);
    sync(&b, &cloud, &credentials);
    sync(&a, &cloud, &credentials);
    assert_eq!(data(&a), data(&b));
    assert_eq!(data(&a), cloud.document().data);
    assert!(data(&a)
        .tasks
        .iter()
        .any(|t| t.title == "Mac offline title"));
    assert!(data(&a)
        .tasks
        .iter()
        .any(|t| t.notes == "Windows offline note"));
    assert!(!pending(&a) && !pending(&b));
    assert_eq!(snapshot(&a).settings.panel_width, 420.0);
    assert_eq!(snapshot(&b).settings.panel_width, 350.0);
    assert_eq!(snapshot(&a).settings.theme, "dark");
    assert_eq!(snapshot(&b).settings.theme, "light");
    let before = snapshot(&b).revision;
    let gets = cloud.server.lock().unwrap().get_calls;
    assert_eq!(sync(&b, &cloud, &credentials), None);
    assert_eq!(snapshot(&b).revision, before);
    assert_eq!(cloud.server.lock().unwrap().get_calls, gets);
}

#[test]
fn soft_delete_plan_removal_and_three_independent_orders_survive_round_trip() {
    let temp = TempClients::new();
    let cloud = MockCloud::new();
    let credentials = credentials();
    let a = temp.open("mac");
    let b = temp.open("windows");
    let ids = seed(&a);
    sync(&a, &cloud, &credentials);
    sync(&b, &cloud, &credentials);
    mutate(
        &a,
        Action::ReorderToday {
            date: DATE.into(),
            task_ids: vec![ids[1].clone(), ids[0].clone(), ids[2].clone()],
        },
    );
    mutate(
        &a,
        Action::ReorderTasks {
            scope: "all".into(),
            task_ids: vec![ids[2].clone(), ids[0].clone(), ids[1].clone()],
        },
    );
    mutate(
        &a,
        Action::ReorderTasks {
            scope: "deadlines".into(),
            task_ids: vec![ids[0].clone(), ids[2].clone(), ids[1].clone()],
        },
    );
    mutate(
        &a,
        Action::PlanTask {
            id: ids[0].clone(),
            planned: false,
            date: DATE.into(),
        },
    );
    lifecycle(&a, &ids[2], false);
    let expected = data(&a);
    sync(&a, &cloud, &credentials);
    sync(&b, &cloud, &credentials);
    let received = data(&b);
    assert_eq!(received, data(&a));
    assert_eq!(received.tasks, expected.tasks);
    assert_eq!(received.task_order, expected.task_order);
    assert_eq!(received.deadline_order, expected.deadline_order);
    // Merge may compact sparse rank numbers after removal; task membership
    // and relative order (including the hidden tombstone) are the contract.
    let mut daily = received.plans.clone();
    daily.sort_by_key(|plan| plan.sort_order);
    assert_eq!(
        daily.iter().map(|p| &p.task_id).collect::<Vec<_>>(),
        vec![&ids[1], &ids[2]]
    );
    assert!(daily.iter().all(|plan| plan.date == DATE));
    assert_eq!(
        expected.tasks.len(),
        3,
        "deleted records remain as tombstones"
    );
    assert!(expected
        .tasks
        .iter()
        .find(|t| t.id == ids[2])
        .unwrap()
        .deleted_at
        .is_some());
    assert!(!expected.plans.iter().any(|p| p.task_id == ids[0]));
    assert_ne!(expected.task_order, expected.deadline_order);
    assert_eq!(snapshot(&a).settings.ddl_sort, "manual");
    assert_eq!(
        snapshot(&b).settings.ddl_sort,
        "date",
        "the local viewing preference stays local"
    );
    lifecycle(&b, &ids[2], true);
    sync(&b, &cloud, &credentials);
    sync(&a, &cloud, &credentials);
    assert_eq!(data(&a), data(&b));
    assert!(data(&a).tasks.iter().all(|t| t.deleted_at.is_none()));
    assert_eq!(data(&a).task_order, expected.task_order);
    assert_eq!(data(&a).deadline_order, expected.deadline_order);
    assert_eq!(data(&a).plans, received.plans);
}

#[test]
fn conflict_choices_are_rejected_after_local_or_remote_changes() {
    let temp = TempClients::new();
    let cloud = MockCloud::new();
    let credentials = credentials();
    let a = temp.open("mac");
    let b = temp.open("windows");
    let ids = seed(&a);
    sync(&a, &cloud, &credentials);
    sync(&b, &cloud, &credentials);
    edit(&a, &ids[0], json!({"title":"remote 1"}));
    edit(&b, &ids[0], json!({"title":"local 1"}));
    sync(&a, &cloud, &credentials);
    let baseline = state(&b);
    let initial = conflict(&b, &cloud, &credentials, None);
    assert_eq!(initial.conflicts.len(), 1);
    assert_eq!(state(&b), baseline);
    let puts = cloud.server.lock().unwrap().put_calls;
    edit(&b, &ids[0], json!({"title":"local 2"}));
    let resolution = (initial.clone(), choose(&initial, ConflictChoice::Remote));
    let changed_local = conflict(&b, &cloud, &credentials, Some(&resolution));
    assert_ne!(changed_local.local, initial.local);
    edit(&a, &ids[0], json!({"title":"remote 2"}));
    sync(&a, &cloud, &credentials);
    let resolution = (
        changed_local.clone(),
        choose(&changed_local, ConflictChoice::Remote),
    );
    let changed_remote = conflict(&b, &cloud, &credentials, Some(&resolution));
    assert!(changed_remote.remote_revision > changed_local.remote_revision);
    assert_eq!(
        cloud.server.lock().unwrap().put_calls,
        puts + 1,
        "stale choices never upload"
    );
    assert_eq!(state(&b), baseline);
    let resolution = (
        changed_remote.clone(),
        choose(&changed_remote, ConflictChoice::Remote),
    );
    assert!(matches!(
        cycle(&b, &cloud, &credentials, Some(&resolution)),
        Ok(CycleResult::Committed(Some(_)))
    ));
    assert_eq!(data(&b), cloud.document().data);
    assert!(data(&b).tasks.iter().any(|t| t.title == "remote 2"));
    assert!(!pending(&b));
}

#[test]
fn network_failure_and_server_cas_failure_leave_durable_pending_for_retry() {
    let temp = TempClients::new();
    let cloud = MockCloud::new();
    let credentials = credentials();
    let a = temp.open("mac");
    seed(&a);
    let original = state(&a);
    cloud.server.lock().unwrap().fail_head = Some(CloudError::Message("synthetic offline".into()));
    assert!(cycle(&a, &cloud, &credentials, None).is_err());
    assert_eq!(state(&a), original);
    assert_eq!(cloud.document().revision, 0);
    cloud.server.lock().unwrap().reject_put = true;
    assert!(matches!(
        cycle(&a, &cloud, &credentials, None),
        Err(CloudError::Stale)
    ));
    assert_eq!(state(&a), original);
    drop(a);
    let a = temp.open("mac");
    assert!(pending(&a));
    assert_eq!(state(&a), original);
    sync(&a, &cloud, &credentials);
    assert_eq!(data(&a), cloud.document().data);
    assert!(!pending(&a));
}

#[test]
fn lost_success_response_is_recovered_after_restart_without_duplicate_upload() {
    let temp = TempClients::new();
    let cloud = MockCloud::new();
    let credentials = credentials();
    let a = temp.open("mac");
    seed(&a);
    let original = state(&a);
    cloud.server.lock().unwrap().drop_put_response = true;
    assert!(cycle(&a, &cloud, &credentials, None).is_err());
    assert_eq!(cloud.document().revision, 1);
    assert_eq!(state(&a), original);
    drop(a);
    let a = temp.open("mac");
    assert!(pending(&a));
    sync(&a, &cloud, &credentials);
    assert!(!pending(&a));
    assert_eq!(a.lock().unwrap().sync.remote_revision, 1);
    assert_eq!(cloud.server.lock().unwrap().put_calls, 1);
    assert_eq!(data(&a), cloud.document().data);
}

#[test]
fn late_local_edit_is_kept_pending_and_uploaded_by_next_cycle() {
    let temp = TempClients::new();
    let cloud = MockCloud::new();
    let credentials = credentials();
    let a = temp.open("mac");
    let ids = seed(&a);
    let a_late = a.clone();
    let id = ids[0].clone();
    *cloud.after_put.lock().unwrap() = Some(Box::new(move || {
        edit(&a_late, &id, json!({"notes":"written during HTTPS"}))
    }));
    sync(&a, &cloud, &credentials);
    assert!(pending(&a));
    assert!(data(&a)
        .tasks
        .iter()
        .any(|t| t.notes == "written during HTTPS"));
    assert!(cloud
        .document()
        .data
        .tasks
        .iter()
        .all(|t| t.notes.is_empty()));
    assert_eq!(a.lock().unwrap().sync.baseline, cloud.document().data);
    sync(&a, &cloud, &credentials);
    assert!(!pending(&a));
    assert_eq!(data(&a), cloud.document().data);
    assert_eq!(cloud.document().revision, 2);
}

#[test]
fn conflicting_late_edit_does_not_replace_local_data_or_acknowledge_remote() {
    let temp = TempClients::new();
    let cloud = MockCloud::new();
    let credentials = credentials();
    let a = temp.open("mac");
    let b = temp.open("windows");
    let ids = seed(&a);
    sync(&a, &cloud, &credentials);
    sync(&b, &cloud, &credentials);
    edit(&a, &ids[0], json!({"title":"remote changed"}));
    sync(&a, &cloud, &credentials);
    edit(&b, &ids[1], json!({"notes":"local pending"}));
    let baseline = state(&b);
    let b_late = b.clone();
    let id = ids[0].clone();
    *cloud.after_put.lock().unwrap() = Some(Box::new(move || {
        edit(&b_late, &id, json!({"title":"late local changed"}))
    }));
    assert!(cycle(&b, &cloud, &credentials, None).is_err());
    assert_eq!(state(&b), baseline);
    assert!(data(&b)
        .tasks
        .iter()
        .any(|t| t.title == "late local changed"));
    assert!(data(&b).tasks.iter().any(|t| t.notes == "local pending"));
    assert!(cloud
        .document()
        .data
        .tasks
        .iter()
        .any(|t| t.title == "remote changed"));
    let context = conflict(&b, &cloud, &credentials, None);
    assert_eq!(context.conflicts.len(), 1);
    let resolution = (context.clone(), choose(&context, ConflictChoice::Local));
    assert!(matches!(
        cycle(&b, &cloud, &credentials, Some(&resolution)),
        Ok(CycleResult::Committed(_))
    ));
    assert_eq!(data(&b), cloud.document().data);
    assert!(!pending(&b));
}

#[test]
fn concurrent_cloud_write_between_get_and_put_is_retried_without_overwrite() {
    let temp = TempClients::new();
    let cloud = Arc::new(MockCloud::new());
    let credentials = credentials();
    let a = temp.open("mac");
    seed(&a);
    let server = cloud.clone();
    *cloud.after_get.lock().unwrap() = Some(Box::new(move || {
        let mut server = server.server.lock().unwrap();
        server.document.revision = 1;
        // Another client initialized an empty document after our GET.
    }));
    let original = state(&a);
    assert!(matches!(
        cycle(&a, cloud.as_ref(), &credentials, None),
        Err(CloudError::Stale)
    ));
    assert_eq!(state(&a), original);
    assert!(cloud.document().data.tasks.is_empty());
    sync(&a, &cloud, &credentials);
    assert_eq!(cloud.document().revision, 2);
    assert_eq!(cloud.document().data, data(&a));
}

#[test]
fn expired_credentials_and_unauthorized_head_refresh_before_data_requests() {
    for expired in [true, false] {
        let temp = TempClients::new();
        let cloud = MockCloud::new();
        let credentials = credentials();
        let a = temp.open("mac");
        seed(&a);
        if expired {
            credentials.0.lock().unwrap().as_mut().unwrap().expires_at = 1;
        } else {
            cloud.server.lock().unwrap().fail_head = Some(CloudError::Unauthorized);
        }
        sync(&a, &cloud, &credentials);
        let session = credentials.0.lock().unwrap().clone().unwrap();
        assert_eq!(session.access_token, "synthetic-access-1");
        assert_eq!(session.refresh_token, "synthetic-refresh-1");
        let server = cloud.server.lock().unwrap();
        assert_eq!(server.refresh_calls, 1);
        assert_eq!(server.observed_tokens.last().unwrap(), "synthetic-access-1");
        if expired {
            assert_eq!(server.observed_tokens.len(), 1);
        } else {
            assert_eq!(server.observed_tokens.len(), 2);
        }
    }
}

struct RejectCredentialWrite(MemoryCredentials);
impl Credentials for RejectCredentialWrite {
    fn read(&self, state: &SyncState, binding: &SyncBinding) -> Result<Option<Session>, String> {
        self.0.read(state, binding)
    }
    fn write(&self, _: &SyncState, _: &SyncBinding, _: &Session) -> Result<(), String> {
        Err("synthetic secure storage failure".into())
    }
    fn delete(&self, state: &SyncState, binding: &SyncBinding) -> Result<(), String> {
        self.0.delete(state, binding)
    }
}

#[test]
fn refresh_or_secure_storage_failure_never_acknowledges_or_uploads_pending_data() {
    for secure_write_fails in [false, true] {
        let temp = TempClients::new();
        let cloud = MockCloud::new();
        let memory = credentials();
        memory.0.lock().unwrap().as_mut().unwrap().expires_at = 1;
        let credentials: Box<dyn Credentials> = if secure_write_fails {
            Box::new(RejectCredentialWrite(memory))
        } else {
            cloud.server.lock().unwrap().fail_refresh = true;
            Box::new(memory)
        };
        let a = temp.open("mac");
        seed(&a);
        let original = state(&a);
        assert!(cycle(&a, &cloud, credentials.as_ref(), None).is_err());
        assert_eq!(state(&a), original);
        assert!(pending(&a));
        let server = cloud.server.lock().unwrap();
        assert_eq!(server.head_calls, 0);
        assert_eq!(server.put_calls, 0);
        assert_eq!(server.document.revision, 0);
    }
}

#[test]
fn server_revision_rollback_is_blocked_without_upload_or_baseline_change() {
    let temp = TempClients::new();
    let cloud = MockCloud::new();
    let credentials = credentials();
    let a = temp.open("mac");
    let ids = seed(&a);
    sync(&a, &cloud, &credentials);
    edit(&a, &ids[0], json!({"notes":"pending after reset"}));
    let original = state(&a);
    cloud.server.lock().unwrap().document = Document {
        revision: 0,
        data: SyncData::empty(),
    };
    let puts = cloud.server.lock().unwrap().put_calls;
    assert!(cycle(&a, &cloud, &credentials, None).is_err());
    assert_eq!(state(&a), original);
    assert_eq!(cloud.server.lock().unwrap().put_calls, puts);
    assert!(pending(&a));
}

#[test]
fn local_storage_failure_after_cloud_commit_retries_without_republishing() {
    let temp = TempClients::new();
    let cloud = MockCloud::new();
    let credentials = credentials();
    let a = temp.open("mac");
    seed(&a);
    let original = state(&a);
    let connection = rusqlite::Connection::open(temp.0.join("mac.sqlite")).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_sync BEFORE UPDATE ON app_state WHEN NEW.key='sync' BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;").unwrap();
    assert!(cycle(&a, &cloud, &credentials, None).is_err());
    assert_eq!(state(&a), original);
    assert_eq!(cloud.document().revision, 1);
    assert!(pending(&a));
    connection
        .execute_batch("DROP TRIGGER reject_sync;")
        .unwrap();
    drop(connection);
    sync(&a, &cloud, &credentials);
    assert!(!pending(&a));
    assert_eq!(data(&a), cloud.document().data);
    assert_eq!(cloud.server.lock().unwrap().put_calls, 1);
}

#[test]
fn exit_during_network_request_stops_publication_and_keeps_pending_for_restart() {
    use std::sync::atomic::{AtomicBool, Ordering};
    for server_already_committed in [false, true] {
        let temp = TempClients::new();
        let cloud = MockCloud::new();
        let credentials = credentials();
        let a = temp.open("mac");
        seed(&a);
        let original = state(&a);
        let stopping = Arc::new(AtomicBool::new(false));
        let signal = stopping.clone();
        let hook: Hook = Box::new(move || signal.store(true, Ordering::SeqCst));
        if server_already_committed {
            *cloud.after_put.lock().unwrap() = Some(hook);
        } else {
            *cloud.after_get.lock().unwrap() = Some(hook);
        }
        let guard = || {
            if stopping.load(Ordering::SeqCst) {
                Err("synthetic application exit".into())
            } else {
                Ok(())
            }
        };
        assert!(cycle_with_guard(&a, &cloud, &credentials, None, &guard).is_err());
        assert_eq!(state(&a), original);
        assert!(pending(&a));
        assert_eq!(
            cloud.document().revision,
            u64::from(server_already_committed)
        );
        drop(a);
        let a = temp.open("mac");
        assert_eq!(state(&a), original);
        sync(&a, &cloud, &credentials);
        assert!(!pending(&a));
        assert_eq!(cloud.document().revision, 1);
        assert_eq!(cloud.server.lock().unwrap().put_calls, 1);
    }
}
