use super::{
    credentials::Session,
    data::SyncData,
    state::{SyncConfig, MAX_DOCUMENT_BYTES, MAX_REVISION},
};
use reqwest::blocking::{Client, Response};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use std::{io::Read, time::Duration};

pub enum CloudError {
    Unauthorized,
    Stale,
    Message(String),
}
impl CloudError {
    pub fn message(&self) -> String {
        match self {
            Self::Unauthorized => "登录已过期或被撤销，请重新登录；本地修改仍保留。".into(),
            Self::Stale => "另一台设备已更新云端，请重试同步。".into(),
            Self::Message(message) => message.clone(),
        }
    }
}
#[derive(Clone)]
pub struct Document {
    pub revision: u64,
    pub data: SyncData,
}
pub trait Cloud: Send + Sync {
    fn login(
        &self,
        config: &SyncConfig,
        email: &str,
        password: &str,
    ) -> Result<(String, String, Session), CloudError>;
    fn refresh(
        &self,
        config: &SyncConfig,
        user_id: &str,
        session: &Session,
    ) -> Result<Session, CloudError>;
    fn head(&self, config: &SyncConfig, session: &Session) -> Result<u64, CloudError>;
    fn get(&self, config: &SyncConfig, session: &Session) -> Result<Document, CloudError>;
    fn put(
        &self,
        config: &SyncConfig,
        session: &Session,
        expected: u64,
        data: &SyncData,
    ) -> Result<u64, CloudError>;
}
pub struct Supabase {
    client: Client,
    #[cfg(test)]
    test_endpoint: Option<String>,
}
impl Supabase {
    #[cfg(all(target_os = "android", feature = "mobile-smoke"))]
    pub fn probe(&self, config: &SyncConfig) -> Result<u16, String> {
        config.validate()?;
        self.client
            .get(format!("{}/auth/v1/settings", config.project_url))
            .header("apikey", &config.publishable_key)
            .send()
            .map(|response| response.status().as_u16())
            .map_err(|_| "Android HTTPS connection failed".into())
    }
    #[cfg(not(target_os = "android"))]
    pub fn new() -> Result<Self, String> {
        Self::build(None)
    }
    #[cfg(target_os = "android")]
    pub fn new_android(app: &tauri::AppHandle) -> Result<Self, String> {
        use base64::Engine;
        use tauri::Manager;
        let encoded = app
            .state::<tauri_plugin_sidetask_mobile::SecureStore<tauri::Wry>>()
            .certificates()?;
        if encoded.is_empty() || encoded.len() > 512 {
            return Err("系统信任证书不可用。".into());
        }
        let certificates = encoded
            .into_iter()
            .map(|value| {
                let der = base64::engine::general_purpose::STANDARD
                    .decode(value)
                    .map_err(|_| "系统证书格式无效。")?;
                reqwest::Certificate::from_der(&der).map_err(|_| "系统证书格式无效。".to_string())
            })
            .collect::<Result<Vec<_>, String>>()?;
        Self::build(Some(certificates))
    }
    fn build(certificates: Option<Vec<reqwest::Certificate>>) -> Result<Self, String> {
        let builder = Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(25))
            .redirect(reqwest::redirect::Policy::none());
        // Read Android system CAs natively; never disable hostname verification.
        let builder = if let Some(certificates) = certificates {
            builder.tls_certs_only(certificates)
        } else {
            builder
        };
        let client = builder.build().map_err(|_| "无法初始化同步网络连接。")?;
        Ok(Self {
            client,
            #[cfg(test)]
            test_endpoint: None,
        })
    }
    fn request<T: DeserializeOwned>(
        &self,
        config: &SyncConfig,
        path: &str,
        token: Option<&str>,
        body: Value,
    ) -> Result<T, CloudError> {
        config.validate().map_err(CloudError::Message)?;
        let base = &config.project_url;
        #[cfg(test)]
        let base = self.test_endpoint.as_ref().unwrap_or(base);
        let mut request = self
            .client
            .post(format!("{base}{path}"))
            .header("apikey", &config.publishable_key)
            .json(&body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().map_err(|_| {
            CloudError::Message(
                "连接失败，请检查网络及项目地址；本地任务已保存，稍后会重试。".into(),
            )
        })?;
        if path == "/auth/v1/token?grant_type=refresh_token" && response.status().as_u16() == 400 {
            return Err(CloudError::Unauthorized);
        }
        decode(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
    };
    fn mock(
        status: &str,
        body: &str,
        extra_headers: &str,
    ) -> (Supabase, Arc<Mutex<String>>, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let length_header = if extra_headers
            .to_ascii_lowercase()
            .contains("content-length:")
        {
            String::new()
        } else {
            format!("Content-Length: {}\r\n", body.len())
        };
        let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{length_header}Connection: close\r\n{extra_headers}\r\n{body}");
        let captured = Arc::new(Mutex::new(String::new()));
        let record = captured.clone();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let length = socket.read(&mut buffer).unwrap();
                if length == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..length]);
                let request = String::from_utf8_lossy(&bytes);
                if let Some(split) = request.find("\r\n\r\n") {
                    let size = request[..split]
                        .lines()
                        .find_map(|line| {
                            line.to_lowercase()
                                .strip_prefix("content-length: ")
                                .and_then(|value| value.parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= split + 4 + size {
                        break;
                    }
                }
            }
            *record.lock().unwrap() = String::from_utf8(bytes).unwrap();
            socket.write_all(response.as_bytes()).unwrap();
        });
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        (
            Supabase {
                client,
                test_endpoint: Some(endpoint),
            },
            captured,
            server,
        )
    }
    fn config() -> SyncConfig {
        SyncConfig::new(
            "https://test.supabase.co".into(),
            "sb_publishable_test_only_123456789".into(),
        )
        .unwrap()
    }
    fn session() -> Session {
        Session {
            access_token: "synthetic-access".into(),
            refresh_token: "synthetic-refresh".into(),
            expires_at: chrono::Utc::now().timestamp() + 3600,
        }
    }
    #[test]
    fn head_uses_owner_authentication_without_uploading_task_data() {
        let (cloud, capture, server) = mock("200 OK", r#"{"revision":42}"#, "");
        assert_eq!(
            cloud
                .head(&config(), &session())
                .unwrap_or_else(|e| panic!("{}", e.message())),
            42
        );
        server.join().unwrap();
        let request = capture.lock().unwrap().to_lowercase();
        assert!(request.starts_with("post /rest/v1/rpc/sidetask_sync_head "));
        assert!(request.contains("authorization: bearer synthetic-access"));
        assert!(request.contains("apikey: sb_publishable_test_only_123456789"));
        assert!(!request.contains("tasks"));
    }
    #[test]
    fn password_login_and_refresh_never_return_secret_server_errors() {
        let (cloud, _, server) = mock(
            "400 Bad Request",
            r#"{"error":"synthetic-password SECRET"}"#,
            "",
        );
        let error = cloud
            .login(&config(), "test@example.invalid", "synthetic-password")
            .err()
            .unwrap();
        server.join().unwrap();
        assert!(!error.message().contains("synthetic-password"));
        assert!(!error.message().contains("SECRET"));
        let (cloud, _, server) = mock(
            "400 Bad Request",
            r#"{"error_code":"refresh_token_not_found"}"#,
            "",
        );
        assert!(matches!(
            cloud.refresh(
                &config(),
                "00000000-0000-0000-0000-000000000001",
                &session()
            ),
            Err(CloudError::Unauthorized)
        ));
        server.join().unwrap();
    }
    #[test]
    fn login_validates_user_and_refresh_preserves_bound_identity() {
        let body = r#"{"access_token":"access","refresh_token":"refresh","expires_in":3600,"user":{"id":"00000000-0000-0000-0000-000000000001","email":"test@example.invalid"}}"#;
        let (cloud, captured, server) = mock("200 OK", body, "");
        let (id, _, session) = cloud
            .login(&config(), "test@example.invalid", "test-password")
            .unwrap_or_else(|e| panic!("{}", e.message()));
        assert_eq!(id, "00000000-0000-0000-0000-000000000001");
        assert_eq!(session.refresh_token, "refresh");
        server.join().unwrap();
        assert!(captured.lock().unwrap().contains("grant_type=password"));
        let (cloud, _, server) = mock("200 OK", body, "");
        assert!(matches!(
            cloud.refresh(&config(), "00000000-0000-0000-0000-000000000002", &session),
            Err(CloudError::Unauthorized)
        ));
        server.join().unwrap();
    }
    #[test]
    fn incompatible_protocol_invalid_data_and_unsafe_revisions_are_rejected() {
        for body in [
            r#"{"revision":1,"protocolVersion":2,"data":{"tasks":[],"plans":[],"taskOrder":[],"deadlineOrder":[]}}"#,
            r#"{"revision":1,"protocolVersion":1,"data":{"tasks":[],"plans":[],"taskOrder":["missing"],"deadlineOrder":[]}}"#,
        ] {
            let (cloud, _, server) = mock("200 OK", body, "");
            assert!(cloud.get(&config(), &session()).is_err());
            server.join().unwrap();
        }
        let (cloud, _, server) = mock("200 OK", r#"{"revision":9007199254740992}"#, "");
        assert!(cloud.head(&config(), &session()).is_err());
        server.join().unwrap();
    }
    #[test]
    fn stale_cas_and_redirects_are_not_silently_retried() {
        let (cloud, captured, server) = mock("409 Conflict", "{}", "");
        assert!(matches!(
            cloud.put(&config(), &session(), 8, &SyncData::default()),
            Err(CloudError::Stale)
        ));
        server.join().unwrap();
        assert!(captured.lock().unwrap().contains("\"expected_revision\":8"));
        let (cloud, _, server) = mock("302 Found", "{}", "Location: http://127.0.0.1:1/secret\r\n");
        assert!(cloud.head(&config(), &session()).is_err());
        server.join().unwrap();
    }
    #[test]
    fn oversized_content_length_is_rejected_before_loading_body() {
        let (cloud, _, server) = mock("200 OK", "{}", "Content-Length: 999999999\r\n");
        assert!(cloud
            .head(&config(), &session())
            .err()
            .unwrap()
            .message()
            .contains("容量"));
        server.join().unwrap();
    }
}
fn decode<T: DeserializeOwned>(mut response: Response) -> Result<T, CloudError> {
    let status = response.status();
    // Never return raw server bodies (auth errors may contain personally identifying data).
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(CloudError::Unauthorized);
    }
    if status.as_u16() == 409 {
        return Err(CloudError::Stale);
    }
    if !status.is_success() {
        let hint = if status.as_u16() == 404 {
            "请先在 Supabase SQL Editor 执行 SideTask 同步脚本。"
        } else if status.as_u16() == 429 {
            "请求暂时过多，请稍后重试。"
        } else if status.as_u16() == 400 {
            "请求被项目拒绝，请检查账号、密码以及同步 SQL 版本。"
        } else {
            "云端暂不可用，请稍后重试；本地任务已保存。"
        };
        return Err(CloudError::Message(hint.into()));
    }
    let limit = MAX_DOCUMENT_BYTES + 1024 * 1024;
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(CloudError::Message("云端数据超过支持容量，未应用。".into()));
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| CloudError::Message("同步响应读取失败，本地修改仍保留。".into()))?;
    if bytes.len() > limit {
        return Err(CloudError::Message("云端响应超过支持容量，未应用。".into()));
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        CloudError::Message("云端响应格式不兼容，未应用；请检查同步 SQL 版本。".into())
    })
}
#[derive(Deserialize)]
struct AuthUser {
    id: String,
    email: Option<String>,
}
#[derive(Deserialize)]
struct AuthResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
    user: AuthUser,
}
impl AuthResponse {
    fn into_parts(self) -> Result<(String, String, Session), CloudError> {
        uuid::Uuid::parse_str(&self.user.id)
            .map_err(|_| CloudError::Message("云端账号标识无效。".into()))?;
        if self.expires_in <= 0 || self.expires_in > 604800 {
            return Err(CloudError::Message("云端登录有效期无效。".into()));
        }
        let session = Session {
            access_token: self.access_token,
            refresh_token: self.refresh_token,
            expires_at: chrono::Utc::now().timestamp() + self.expires_in,
        };
        session.validate().map_err(CloudError::Message)?;
        let email = self.user.email.ok_or_else(|| {
            CloudError::Message("仅支持邮箱账号，请在项目中创建邮箱用户。".into())
        })?;
        Ok((self.user.id, email, session))
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Head {
    revision: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireDocument {
    revision: u64,
    protocol_version: u32,
    data: SyncData,
}
fn check_revision(revision: u64) -> Result<u64, CloudError> {
    if revision > MAX_REVISION {
        Err(CloudError::Message("云端版本超出支持范围，未应用。".into()))
    } else {
        Ok(revision)
    }
}
impl Cloud for Supabase {
    fn login(
        &self,
        config: &SyncConfig,
        email: &str,
        password: &str,
    ) -> Result<(String, String, Session), CloudError> {
        let response: AuthResponse = self.request(
            config,
            "/auth/v1/token?grant_type=password",
            None,
            json!({"email":email,"password":password}),
        )?;
        response.into_parts()
    }
    fn refresh(
        &self,
        config: &SyncConfig,
        user_id: &str,
        session: &Session,
    ) -> Result<Session, CloudError> {
        let response: AuthResponse = self.request(
            config,
            "/auth/v1/token?grant_type=refresh_token",
            None,
            json!({"refresh_token":session.refresh_token}),
        )?;
        let (id, _, session) = response.into_parts()?;
        if id != user_id {
            return Err(CloudError::Unauthorized);
        }
        Ok(session)
    }
    fn head(&self, config: &SyncConfig, session: &Session) -> Result<u64, CloudError> {
        let head: Head = self.request(
            config,
            "/rest/v1/rpc/sidetask_sync_head",
            Some(&session.access_token),
            json!({}),
        )?;
        check_revision(head.revision)
    }
    fn get(&self, config: &SyncConfig, session: &Session) -> Result<Document, CloudError> {
        let document: WireDocument = self.request(
            config,
            "/rest/v1/rpc/sidetask_sync_get",
            Some(&session.access_token),
            json!({}),
        )?;
        if document.protocol_version != 1 {
            return Err(CloudError::Message(
                "同步协议版本不兼容，请更新客户端或 SQL。".into(),
            ));
        }
        document.data.validate().map_err(CloudError::Message)?;
        Ok(Document {
            revision: check_revision(document.revision)?,
            data: document.data,
        })
    }
    fn put(
        &self,
        config: &SyncConfig,
        session: &Session,
        expected: u64,
        data: &SyncData,
    ) -> Result<u64, CloudError> {
        data.validate().map_err(CloudError::Message)?;
        let result: Head = self.request(
            config,
            "/rest/v1/rpc/sidetask_sync_put",
            Some(&session.access_token),
            json!({"expected_revision":expected,"protocol_version":1,"document":data}),
        )?;
        let revision = check_revision(result.revision)?;
        if revision == 0 || revision < expected {
            return Err(CloudError::Message("云端提交版本无效，未确认同步。".into()));
        }
        Ok(revision)
    }
}
