use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u8,
    draft: Option<Value>,
}
pub struct DraftStore {
    path: PathBuf,
}
impl DraftStore {
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.join("mobile-draft-v1.json"),
        }
    }
    pub fn read(&self) -> Result<Option<Value>, String> {
        match std::fs::read(&self.path) {
            Ok(raw) => {
                if raw.len() > 1024 * 1024 {
                    return Err("草稿文件过大，原文件已保留。".into());
                }
                let record: Record =
                    serde_json::from_slice(&raw).map_err(|_| "草稿文件无法读取，原文件已保留。")?;
                if record.version != 1 {
                    return Err("草稿版本不兼容，原文件已保留。".into());
                }
                Ok(record.draft)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err("无法读取本机草稿。".into()),
        }
    }
    pub fn write(&self, draft: Option<Value>) -> Result<(), String> {
        if draft.as_ref().is_some_and(|value| !value.is_object()) {
            return Err("草稿格式无效。".into());
        }
        let raw =
            serde_json::to_vec(&Record { version: 1, draft }).map_err(|_| "草稿格式无效。")?;
        if raw.len() > 1024 * 1024 {
            return Err("草稿过大，尚未暂存。".into());
        }
        let temporary = self
            .path
            .with_extension(format!("{}.pending", uuid::Uuid::new_v4()));
        let result = (|| -> std::io::Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&raw)?;
            file.sync_all()?;
            replace(&temporary, &self.path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
            return Err("无法暂存草稿，请保存任务后再离开应用。".into());
        }
        Ok(())
    }
}
#[cfg(not(windows))]
fn replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}
#[cfg(windows)]
fn replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // Windows test hosts need replacement semantics matching Android's rename.
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn draft_survives_reopen_then_explicit_discard_and_invalid_data_does_not_overwrite() {
        let directory =
            std::env::temp_dir().join(format!("sidetask-draft-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let store = DraftStore::new(&directory);
        assert_eq!(store.read().unwrap(), None);
        let draft = serde_json::json!({"title":"中文草稿", "notes":"offline"});
        store.write(Some(draft.clone())).unwrap();
        assert_eq!(
            DraftStore::new(&directory).read().unwrap(),
            Some(draft.clone())
        );
        assert!(store.write(Some(Value::String("invalid".into()))).is_err());
        assert_eq!(store.read().unwrap(), Some(draft));
        store.write(None).unwrap();
        assert_eq!(DraftStore::new(&directory).read().unwrap(), None);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn corrupt_draft_is_not_reset_when_reading() {
        let directory =
            std::env::temp_dir().join(format!("sidetask-draft-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let store = DraftStore::new(&directory);
        std::fs::write(&store.path, "broken").unwrap();
        assert!(store.read().is_err());
        assert_eq!(std::fs::read(&store.path).unwrap(), b"broken");
        std::fs::remove_dir_all(directory).unwrap();
    }
}
