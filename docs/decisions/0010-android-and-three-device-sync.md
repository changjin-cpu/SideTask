# ADR-0010：Android 客户端与三端同步

- 日期：2026-10-13
- 状态：已采用并实现；模拟器及发行验证见 [Android 开发说明](../engineering/ANDROID.md)，个人账号三端联网独立记录
- 前置：[ADR-0009](0009-optional-desktop-sync.md)

## 需求

用户明确要求在现有 Mac/Windows 客户端之外增加 Android 应用，先用模拟器开发测试，完成后把 APK 放到 GitHub Release。此前排除手机的范围仅为历史。本轮不增加子任务、协作、日历聚合、通知提醒或跨应用悬浮窗。

## 决定

仍使用 Tauri 2、React、Rust 和 SQLite。Android 与桌面使用同一份 domain/application/infrastructure/sync 模块；桌面入口移动到 desktop.rs，Android 入口及生命周期放到 mobile/，不编译桌面托盘与窗口协调器。任务与计划规则不在手机端重复实现。

Android 使用单任务窗口和底部导航，提供今日、全部、截止与设置，设置进入已完成/回收站。手机编辑采用模态页、显式保存与版本化原生文件暂存草稿；任务 revision 冲突保留草稿，不静默覆盖。共用独立排序与虚拟列表，触摸操作适配不改变截止字段。四套风格与明暗在本机保存。

三端共用现有 protocol 1、账号单文档 CAS 和三方合并，不新增第二套云表，不传 SQLite 文件、窗口设置或草稿。手机恢复前台/联网时唤醒同步，前台编辑自动批量补传；退到后台暂停轮询。第一版不承诺后台实时同步、强制停止后自动联网或推送唤醒。

Android access/refresh token 经本地 Tauri 原生插件使用 AES-GCM 加密后保存到私有 preferences，密钥由 Android Keystore 生成。密文带每次生成的 IV，以设备/项目/账号绑定键作为 AAD；同步会话不通过 JS 命令读取，备份关闭。刷新令牌先持久化再使用；存储失败保留本地任务并提示。HTTPS 使用 AndroidCAStore 中系统 CA，保留域名和证书验证，不信任任意测试证书或放宽生产项目地址约束。

Android APK 用持续保管的发布密钥签名，私钥和密码不得进仓库、CI日志或Release。Release附APK、SHA256及签名证书指纹；后续升级必须使用同一签名。暂不提交应用商店，先直接下载APK。

## 验证

三份临时 SQLite 加合成云端验证离线重启、多字段编辑、三种独立顺序、设备设置隔离和第三端使旧冲突选择失效；现有账号隔离/CAS/删除恢复等测试保留。浏览器手机测试不代替 Android 模拟器。模拟器测试安装、原生 SQLite CRUD、重启/返回键/草稿、网络错误与凭据边界，发布前核对APK签名与16KB对齐。Mac/Windows检查继续运行；真实 Supabase 与实际Mac设备覆盖独立记录，不把模拟器或合成测试写成真机验收。
