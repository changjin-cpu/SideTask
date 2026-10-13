mod application;
#[cfg(not(target_os = "android"))]
mod desktop;
mod domain;
mod infrastructure;
#[cfg(target_os = "android")]
mod mobile;
#[cfg(all(test, not(target_os = "android")))]
#[path = "mobile/draft.rs"]
mod mobile_draft_tests;
#[cfg(not(target_os = "android"))]
mod platform;
#[cfg(target_os = "android")]
#[path = "mobile/platform.rs"]
mod platform;
mod sync;
#[cfg(target_os = "windows")]
#[path = "platform/windows_webview_shutdown.rs"]
mod windows_webview_shutdown;
#[cfg(not(target_os = "android"))]
pub(crate) use desktop::task_state;
#[cfg(not(target_os = "android"))]
pub use desktop::{run, AppState};
#[cfg(target_os = "android")]
pub(crate) use mobile::task_state;
#[cfg(target_os = "android")]
pub use mobile::{run, AppState};
