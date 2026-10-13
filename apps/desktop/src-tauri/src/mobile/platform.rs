pub mod exit {
    pub fn ensure_running(state: &crate::AppState) -> Result<(), String> {
        if state.running.load(std::sync::atomic::Ordering::Acquire) {
            Ok(())
        } else {
            Err("应用正在关闭，请稍后重新打开。".into())
        }
    }
}
