# Android 第三方材料

Android 工程骨架及 buildSrc 源自 Tauri CLI 2.11.5 生成模板；Tauri Android runtime 为 Cargo.lock 固定的 2.11.6。版权归 Tauri Apps Contributors，许可为 MIT OR Apache-2.0，完整文本见 [MIT](LICENSE-MIT) / [Apache-2.0](LICENSE-APACHE)。来源：https://github.com/tauri-apps/tauri 。保留生成文件的现有许可头，本项目修改包含 MainActivity、配置、图标和自有凭据插件。

Gradle Wrapper 8.14.3 为 Apache-2.0，保留 gradlew/gradlew.bat 的原有版权与许可头，来源：https://github.com/gradle/gradle/tree/v8.14.3 。它仅用于构建，不装入 APK。

Android 直接组件采用固定版本：AndroidX WebKit 1.14.0、AppCompat 1.7.1、Activity KTX 1.10.1、Lifecycle Process 2.10.0、Material Components 1.12.0，均为 Apache-2.0。来源分别为 https://android.googlesource.com/platform/frameworks/support/ 和 https://github.com/material-components/material-components-android 。Kotlin 构建插件 1.9.25 为 Apache-2.0，来源：https://github.com/JetBrains/kotlin 。实际 Gradle 依赖关系以工程解析结果为准。

前端许可继续随应用提供 public/third-party/frontend-notices.txt 和 time-dependencies.txt。本记录只说明新增 Android 组件及生成工程，不声称代替全部 Rust 传递依赖的许可。
