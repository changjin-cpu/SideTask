use super::state::{SyncBinding, SyncState};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// Deliberately no Debug: HTTP errors and diagnostic logs must not expose tokens.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}
impl Session {
    pub fn validate(&self) -> Result<(), String> {
        if self.access_token.is_empty()
            || self.refresh_token.is_empty()
            || self.access_token.len() > 16384
            || self.refresh_token.len() > 16384
            || self.access_token.chars().any(char::is_control)
            || self.refresh_token.chars().any(char::is_control)
            || self.expires_at <= 0
        {
            return Err("登录凭据无效，请重新登录。".into());
        }
        Ok(())
    }
}
pub trait Credentials: Send + Sync {
    fn read(&self, state: &SyncState, binding: &SyncBinding) -> Result<Option<Session>, String>;
    fn write(
        &self,
        state: &SyncState,
        binding: &SyncBinding,
        session: &Session,
    ) -> Result<(), String>;
    fn delete(&self, state: &SyncState, binding: &SyncBinding) -> Result<(), String>;
}
#[cfg(not(target_os = "android"))]
pub struct NativeCredentials;
#[cfg(target_os = "android")]
pub struct NativeCredentials(pub tauri::AppHandle);

#[cfg(target_os = "android")]
impl NativeCredentials {
    fn key(state: &SyncState, binding: &SyncBinding) -> String {
        format!(
            "{:x}",
            Sha256::digest(
                format!(
                    "{}\n{}\n{}",
                    state.device_id, binding.config.project_url, binding.user_id
                )
                .as_bytes()
            )
        )
    }
    fn store(&self) -> tauri::State<'_, tauri_plugin_sidetask_mobile::SecureStore<tauri::Wry>> {
        use tauri::Manager;
        self.0.state()
    }
}
impl NativeCredentials {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    fn entry(state: &SyncState, binding: &SyncBinding) -> Result<keyring::Entry, String> {
        let key = format!(
            "{}\n{}\n{}",
            state.device_id, binding.config.project_url, binding.user_id
        );
        let name = format!("desktop-sync-{:x}", Sha256::digest(key.as_bytes()));
        keyring::Entry::new("com.changjin.sidetask", &name)
            .map_err(|_| "无法访问系统凭据库；本地任务仍可使用。".into())
    }
}
impl Credentials for NativeCredentials {
    fn read(&self, state: &SyncState, binding: &SyncBinding) -> Result<Option<Session>, String> {
        #[cfg(target_os = "android")]
        {
            let raw = self.store().read(&Self::key(state, binding))?;
            raw.map(|raw| {
                let session: Session =
                    serde_json::from_str(&raw).map_err(|_| "登录凭据格式无效，请重新登录。")?;
                session.validate()?;
                Ok(session)
            })
            .transpose()
        }
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        {
            let raw = match Self::entry(state, binding)?.get_password() {
                Ok(raw) => raw,
                Err(keyring::Error::NoEntry) => return Ok(None),
                Err(_) => return Err("无法读取系统保存的登录凭据，请重试或重新登录。".into()),
            };
            let session: Session =
                serde_json::from_str(&raw).map_err(|_| "登录凭据格式无效，请重新登录。")?;
            session.validate()?;
            Ok(Some(session))
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "android")))]
        {
            let _ = (state, binding);
            Err("同步目前仅支持 macOS 和 Windows。".into())
        }
    }
    fn write(
        &self,
        state: &SyncState,
        binding: &SyncBinding,
        session: &Session,
    ) -> Result<(), String> {
        session.validate()?;
        #[cfg(target_os = "android")]
        {
            let raw = serde_json::to_string(session).map_err(|_| "无法编码登录凭据。")?;
            self.store().write(&Self::key(state, binding), &raw)
        }
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        {
            let raw = serde_json::to_string(session).map_err(|_| "无法编码登录凭据。")?;
            Self::entry(state, binding)?
                .set_password(&raw)
                .map_err(|_| "无法把登录凭据保存到系统凭据库；未连接同步。".into())
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "android")))]
        {
            let _ = (state, binding);
            Err("同步目前仅支持 macOS 和 Windows。".into())
        }
    }
    fn delete(&self, state: &SyncState, binding: &SyncBinding) -> Result<(), String> {
        #[cfg(target_os = "android")]
        {
            self.store().delete(&Self::key(state, binding))
        }
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        {
            match Self::entry(state, binding)?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(_) => Err("无法移除系统登录凭据，请重试断开同步。".into()),
            }
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "android")))]
        {
            let _ = (state, binding);
            Err("同步目前仅支持 macOS 和 Windows。".into())
        }
    }
}

#[cfg(test)]
pub struct MemoryCredentials(pub std::sync::Mutex<Option<Session>>);
#[cfg(test)]
impl Credentials for MemoryCredentials {
    fn read(&self, _: &SyncState, _: &SyncBinding) -> Result<Option<Session>, String> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn write(&self, _: &SyncState, _: &SyncBinding, session: &Session) -> Result<(), String> {
        *self.0.lock().unwrap() = Some(session.clone());
        Ok(())
    }
    fn delete(&self, _: &SyncState, _: &SyncBinding) -> Result<(), String> {
        *self.0.lock().unwrap() = None;
        Ok(())
    }
}
