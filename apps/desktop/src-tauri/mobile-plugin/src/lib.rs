use serde::{Deserialize, Serialize};
use tauri::{
    plugin::{Builder, PluginHandle, TauriPlugin},
    Manager, Runtime,
};

pub struct SecureStore<R: Runtime>(PluginHandle<R>);
#[derive(Serialize)]
struct Request<'a> {
    key: &'a str,
    value: Option<&'a str>,
}
#[derive(Deserialize)]
struct Response {
    value: Option<String>,
}
#[derive(Deserialize)]
struct CertificateResponse {
    certificates: Vec<String>,
}
impl<R: Runtime> SecureStore<R> {
    pub fn time_zone(&self) -> Result<String, String> {
        self.0
            .run_mobile_plugin::<Response>(
                "timeZone",
                Request {
                    key: "",
                    value: None,
                },
            )
            .map_err(|_| "无法读取系统时区。".to_string())?
            .value
            .ok_or_else(|| "系统时区为空。".into())
    }
    pub fn certificates(&self) -> Result<Vec<String>, String> {
        self.0
            .run_mobile_plugin::<CertificateResponse>(
                "systemCertificates",
                Request {
                    key: "",
                    value: None,
                },
            )
            .map(|response| response.certificates)
            .map_err(|_| "无法读取系统信任证书。".into())
    }
    pub fn read(&self, key: &str) -> Result<Option<String>, String> {
        self.0
            .run_mobile_plugin::<Response>("readSession", Request { key, value: None })
            .map(|response| response.value)
            .map_err(|_| "无法读取系统保存的登录凭据，请重新登录。".into())
    }
    pub fn write(&self, key: &str, value: &str) -> Result<(), String> {
        self.0
            .run_mobile_plugin::<Response>(
                "writeSession",
                Request {
                    key,
                    value: Some(value),
                },
            )
            .map(|_| ())
            .map_err(|_| "无法安全保存登录凭据；未连接同步。".into())
    }
    pub fn delete(&self, key: &str) -> Result<(), String> {
        self.0
            .run_mobile_plugin::<Response>("deleteSession", Request { key, value: None })
            .map(|_| ())
            .map_err(|_| "无法清理登录凭据，请重试断开。".into())
    }
}
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    // No JS commands or permissions expose session plaintext to the WebView.
    Builder::new("sidetask-mobile")
        .setup(|app, api| {
            let handle =
                api.register_android_plugin("com.changjin.sidetask.mobile", "SecureStorePlugin")?;
            app.manage(SecureStore(handle));
            Ok(())
        })
        .build()
}
