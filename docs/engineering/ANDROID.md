# Android 开发与发布

Android 使用一个完整控制台，提供今日计划、全部任务、截止日期、已完成、回收站与同步设置。没有边缘小窗、悬浮把手、托盘或跨应用悬浮权限。任务服务、SQLite schema 7 和 Supabase protocol 1 与桌面共用；已有 Mac/Windows 项目无需重新运行 SQL 或换账号。

## 工具链

本轮实际使用 Node 24、Rust 1.98.1、JDK 21、Android SDK 36、Build Tools 35.0.0、NDK 28.0.13004108、Gradle 8.14.3、AGP 8.11.0。最低 Android 8 / API 26；APK 包含 arm64-v8a 和 x86_64，不包含旧 32 位 ABI。请同时保留 Cargo.lock 和 package-lock.json；Gradle 版本见已提交工程。

安装 [Tauri Android 前置环境](https://v2.tauri.app/start/prerequisites/#android)，设置 JAVA_HOME、ANDROID_HOME 和 NDK_HOME。在 apps/desktop 执行：

```sh
npm ci
rustup target add aarch64-linux-android x86_64-linux-android
npm run tauri -- android build --ci --apk --target aarch64 x86_64
```

gen/android 是正式源码，包含 MainActivity、图标、权限及 Gradle 定制，不要重新初始化后覆盖它。生成的本机路径、JNI 库、编译产物与签名材料均忽略。

Windows NDK 无法稳定处理含中文的 target 路径，建议把 CARGO_TARGET_DIR 放到独立英文目录。未启用符号链接权限时，可使用仓库脚本编译后校验复制 .so，再调用 Gradle：

```powershell
./scripts/build-android-windows.ps1 -SdkRoot 'D:/android/sdk' -JdkRoot 'D:/android/jdk21' -NativeTargetDir 'D:/android/target' -Profile release
```

脚本仅接受指定的符号链接失败，不跳过其他编译错误；不会修改系统开发者设置。CI 的 Linux Android 作业使用标准构建，不执行此回退。

## 模拟器验收

使用独立 SideTask_API35 AVD、API 35 Google APIs x86_64。无需连接手机。仅在 debug 构建启用 mobile-smoke：

```powershell
./scripts/build-android-windows.ps1 -SdkRoot 'D:/android/sdk' -JdkRoot 'D:/android/jdk21' -NativeTargetDir 'D:/android/target' -Profile debug -Targets x86_64 -Smoke
adb -s emulator-5554 install -r src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
adb -s emulator-5554 shell am start -n com.changjin.sidetask/.MainActivity
node scripts/android-emulator-smoke.mjs
```

首次验收要求空的合成任务库。--reset 仅允许名称为 SideTask_API35 的模拟器、未连接同步且任务标题全部匹配测试命名时清理测试应用，拒绝真实设备。报告写入忽略的 artifacts/android。可通过 SIDETASK_TEST_PROJECT_URL 和 SIDETASK_TEST_PUBLISHABLE_KEY 环境变量做只读 Auth settings HTTPS 检查；不会读取个人任务或登录账号。

覆盖原生 SQLite 创建/编辑、完成/撤销、移出今日不改 DDL、回收站恢复、返回/前后台、进程重启、原生草稿恢复、触摸排序、Keystore 密文及 JS 权限边界。三份临时 SQLite 的共享引擎测试覆盖离线修改、三端合并及过期冲突选择。它们不等于三个个人设备同时登录的联网验收。桌面同步成功由用户报告；Android 与个人账号的实际三端联网需安装后按 [同步配置](SUPABASE_SETUP.md) 核对。

## 发布

正式 APK 不启用 mobile-smoke；release 构建启用该 feature 会编译失败。WebView 调试仅 debug 开启。用持续保管的发布密钥签名，先执行 zipalign -P 16，再使用 [apksigner](https://developer.android.com/tools/apksigner) 签名并验证。验证 ZIP 对齐和原生 ELF LOAD 对齐，依据 [Android 16 KB 页面说明](https://developer.android.com/guide/practices/page-sizes)。签名私钥、密码、keystore.properties 不得入库或上传 Release；更新需沿用同一签名并提高 versionCode。

Windows 发布签名可使用 scripts/sign-android.ps1：指定 SDK、unsigned APK、私有 SigningDirectory 和 OutputApk。脚本读取该目录的 sidetask-release.jks 与当前用户 DPAPI 加密的 password.dpapi.xml，通过临时环境变量向 apksigner 传递密码，完成后清除，不把密码打印到日志。目录仅授予本人访问；换机器前安全备份私钥并重新加密密码，DPAPI 文件不能直接跨机器使用。

发布签名 APK、SHA256SUMS.txt 与证书 SHA-256 指纹；实际安装发布 APK检查启动、任务保存/重启和更新覆盖。无云账号测试时须在 Release 明确列出，不把 TLS 成功当作已登录同步。不要把 unsigned CI APK 当作用户安装包。

Android 仅在前台自动同步，回到应用后补传；不提供后台常驻、推送或 DDL 提醒。任务及草稿在应用私有目录，卸载或清除存储会删除未同步内容；重要内容先在电脑同步并导出备份。草稿用独立版本化 JSON、同步写入和原子替换，失败提示且保留原文件；不上传云端。登录令牌使用 Android Keystore 密钥和 AES-GCM，关闭系统备份，JS 无令牌读取命令。

## 来源与许可

使用 Tauri 2.11.6 / CLI 2.11.5 生成 Android 工程及本地插件骨架，保留生成文件许可头；实际版本以锁文件为准。[Tauri](https://github.com/tauri-apps/tauri) 为 MIT OR Apache-2.0，[Gradle Wrapper](https://github.com/gradle/gradle/tree/v8.14.3) 为 Apache-2.0，AndroidX / Material 为 Apache-2.0。业务界面、Keystore 插件与协议适配为本项目实现，没有复用第三方任务应用源码。SDK/NDK/模拟器和 JDK 仅安装于仓库外，不作为项目附件分发。

## 本轮验证记录（2026-10-13）

共享核心 243 项 Windows Rust、114 项前端业务、6 项工具、17 项手机/同步 UI 定向通过。API 35 x86_64 模拟器的 8 项原生检查通过；签名 release APK 的非调试标记、创建/完成、进程重启、同步初始化及同签名覆盖保留数据通过。调试构建用于检查 Keystore 密文与 JS 权限边界，正式包不包含探针。发布包 ZIP 与两个 ABI 的 ELF LOAD 均为 16 KB 对齐。

[Android CI 38391367388](https://github.com/zhouchji-ops/Sidetask/actions/runs/38391367388) 成功构建 ARM64/x64 unsigned APK 并验证 ZIP 对齐；实际供下载的是本机签名并安装验收的包。宿主网络直连出现过间歇失败，模拟器使用宿主已有 HTTP 代理后真实 Auth settings HTTPS 验证成功，未禁用证书或主机名校验。此检查不登录个人账号、不读取云任务；用户报告桌面双端正常，个人三端联调尚未联合验收。

[桌面 CI 38391646889](https://github.com/zhouchji-ops/Sidetask/actions/runs/38391646889) 同样通过：Windows 243 / Mac 236 项 Rust、各 114 项前端业务、6 项工具、214 项 UI（零重试）、前端构建、fmt、严格 Clippy、npm 审计及原生安装包构建。SQL 与 Rust 审计作业通过；Rust 审计保留既有 allowed warnings，不等于零告警。Android 编译与包验证对应 166f37a，桌面对应 5fcf275；两者之间只有 CI 并发分组变化。签名 APK 由同一产品代码在 Windows NDK 构建，验收摘要见 Release。
