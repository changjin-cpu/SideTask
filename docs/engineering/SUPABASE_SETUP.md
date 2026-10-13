# Mac、Windows 与 Android 同步配置

2026-10-13 更新。Android 复用现有 Mac/Windows 同步协议；已能同步的项目无需重新初始化，手机填写同一项目、公开 key 和应用邮箱账号即可。Android 只有完整控制台。Supabase Free 配额说明沿用 2026-10-04 核对记录，实际额度以官方方案为准。

## 1. 创建免费项目

打开 [Supabase Dashboard](https://supabase.com/dashboard)，注册或登录管理账号。创建组织时选择 **Free**，再创建项目（例如 `sidetask-personal`），设置独立的数据库密码，选择两台电脑都能访问的区域，等待项目就绪。这里的数据库密码仅管理数据库，不是后面在 SideTask 输入的账号密码。

官方 [Free 方案](https://supabase.com/pricing) 当前为 $0/月，包含每项目 500 MB 数据库、5 GB 出站流量及 50,000 月活用户额度；闲置一周后可能暂停，届时从 Dashboard 恢复。个人文字任务先用 Free 即可；本功能不会自动升级方案。不要选择 Pro 或付费附加项。暂停或网络不通时，SideTask 仍保存本机任务。

## 2. 初始化云端数据结构

在项目的 **SQL Editor → New query** 中，粘贴并完整运行仓库里的 [同步 SQL](../../supabase/migrations/202610040001_desktop_sync.sql)。整份脚本使用事务，可以重复运行；仅创建 SideTask 的表与三个 RPC，不改其他业务表。

成功后应有 `public.sidetask_sync_documents` 与 `sidetask_sync_head`、`sidetask_sync_get`、`sidetask_sync_put`。脚本开启 RLS，按登录用户隔离数据，只允许经过用户校验及版本检查的 RPC 写入。不要通过 Table Editor 手工改 `revision` 或删除记录，避免造成版本回退。

项目的 **Data API** 须启用，暴露的 schema 包含 `public`；新项目通常已有此配置。已运行 SQL 仍提示找不到同步接口时，先核对这两项，再在 SQL Editor 执行 `NOTIFY pgrst, 'reload schema';` 刷新接口缓存。`PGRST202` 表示当前接口缓存未找到函数或匹配签名，不能单凭这个错误认定 SQL 没执行。参考 [API 配置](https://supabase.com/docs/guides/api/securing-your-api)、[刷新接口缓存](https://supabase.com/docs/guides/troubleshooting/refresh-postgrest-schema) 与 [错误定义](https://docs.postgrest.org/en/v14/references/errors.html)。

## 3. 创建应用登录账号

进入 **Authentication → Users → Add user → Create new user**，为自己创建邮箱与独立的应用密码，并确认该用户已确认邮箱（创建界面可选 Auto Confirm）。这是同步账号，和 Supabase Dashboard 管理账号、数据库密码分别管理。

当前应用支持已有账号的邮箱/密码登录，注册和密码管理在项目 Dashboard 完成。个人项目可由自己管理这个用户，不需要邀请邮件。若开放给更多人使用，需要另行完善注册、密码重置和邮件服务；Supabase 默认邮件服务仅用于受限测试，不能假定能给任意邮箱发信。参考 [密码认证](https://supabase.com/docs/guides/auth/passwords) 和 [SMTP 限制](https://supabase.com/docs/guides/auth/auth-smtp)。

## 4. 取得公开连接配置

项目 **Connect** 对话框可查看 Project URL 和 Publishable key；也可在 **Settings → API Keys** 复制公开 key。地址形如 `https://项目标识.supabase.co`，key 通常以 `sb_publishable_` 开头。旧项目的 `anon` key 也接受，优先使用 publishable key。详情见 [官方 API key 说明](https://supabase.com/docs/guides/getting-started/api-keys)。

**不要填写 `sb_secret_`、`service_role`、数据库密码或管理访问令牌。** 它们不是桌面客户端连接配置。代码在 Rust 边界再次拒绝服务密钥及非 Supabase HTTPS 项目地址。

## 5. 依次连接电脑和安卓

1. 两台电脑都先导出一份任务 JSON 备份，保管在自己选择的位置。
2. 第一台打开支持同步的新版本，进入 **设置 → Mac、Windows 与安卓同步**。
3. 填写项目地址、公开 key、应用账号邮箱及密码，勾选 **将本机任务与此账号合并**，点击 **登录并启用同步**。
4. 等待首次同步完成，再在第二台电脑或 Android 填写同一项目和账号、同样确认合并。已有双端同步时直接连接手机，保留原账号。
5. 如果两台已有数据，初次连接保留不同 Task ID 的任务；标题相同的独立任务不会自动当作重复删除。同一 Task ID 的内容不同或有并发重排时，界面要求逐项选择保留本机或云端内容，全部确认后才提交。

账号密码仅用于本次登录，输入会清空。访问和刷新令牌保存在 Windows Credential Manager / macOS Keychain / Android Keystore 加密的私有存储；SQLite 仅保存公开配置、账号标识和已确认的同步基线，JSON 导出不含这些凭据。登录失效后重新登录原账号，会沿用基线和未上传的修改；换项目或账号前须先断开。

## 6. 验证三端

使用专门的测试任务，依次在 Mac、Windows 和 Android 确认：

- 第一台创建任务并加入今日，在第二台确认内容和今日安排出现。
- 在第二台完成、撤销完成、移出今日，检查第一台：移出今日应保留任务与 DDL。
- 分别拖动今日、全部任务、截止日期，检查另一台对应顺序；DDL 展示模式为本机偏好，需要选手动排序才能查看手动顺序。
- 一台断网后编辑，再联网；本机应显示待上传，并最终同步。无需重启或重新创建任务。
- 两台离线修改同一任务标题，再联网，应出现冲突选择；两台分别改标题和备注，则应合并。
- 移入回收站、恢复任务后检查另一台；重启两台确认数据仍保留。

Android 在前台进行同步，退出到后台暂停轮询，回到应用后补传；不提供后台常驻或推送。断网仍可保存本机任务。

本机保存后约 2 秒触发补传，在线运行时约每 30 秒检查云端版本；没有变化只取版本号，降低 Free 流量消耗。网络失败可点重试，登录过期需重新登录，冲突待确认期间保留本机与云端版本。窗口位置、皮肤、明暗、DDL 展示模式和编辑草稿不上传。

## 7. 停用、备份与排错

**断开同步**只停用这台电脑并移除其系统登录凭据，本机任务与云端数据保留。若有待上传修改，断开前可以先同步或导出备份。系统凭据清理失败时会暂停自动同步，可点击重试断开。

恢复 JSON 旧备份前，先在本机断开同步，再执行恢复；重新连接会重新合并，旧备份可能包含已经移除的计划或恢复前的内容，需要自己确认。完整 SQLite 安全副本含同步配置和基线，恢复后仍需要系统凭据可用；凭据缺失会要求重新登录。同步不是历史版本备份，重要任务仍应定期导出。

| 提示或现象 | 处理 |
| --- | --- |
| 请先执行 SQL / 未找到接口（PGRST202） | 完整运行同版 SQL；若已执行，检查 Data API 启用及 `public` 暴露，再按第 2 节刷新接口缓存 |
| 响应格式不兼容 | 检查客户端与 SQL 是否同版，不要替换或删改 RPC 返回字段 |
| 登录失败、邮箱未确认 | 在 Authentication 确认用户与密码；这里不接受 Dashboard 密码 |
| 项目已暂停或网络失败 | 在 Dashboard 恢复 Free 项目，检查网络；本机编辑继续保存 |
| 登录已过期或被撤销 | 重新登录原项目与原账号，保留已有同步基线 |
| 内容需要选择 | 逐项核对本机/云端内容；确认时数据有新变化会要求重新核对 |
| 云端版本回退或重置 | 先导出本机备份，核对项目；不要强行用旧版本覆盖，确认后再断开重连 |
| 系统凭据库不可用 | 修复本机 Keychain / Credential Manager 的访问后重试；不会降级为明文存储 |

当前客户端限定 hosted `*.supabase.co` HTTPS 项目，不接入自托管地址或自定义域名。云端每个账号保存一个任务文档，客户端上限 10 MiB / 10,000 任务 / 100,000 计划（含回收站及历史计划）。大量用户、共享协作和 iOS 不在本轮范围。开源用户可以各自按本指南创建自己的 Free 项目；共用维护者项目则消耗维护者的项目额度。

## 开发验证与依赖来源

三方合并、三份 SQLite 客户端同步循环、离线重启、CAS、删除/计划移除/排序、HTTPS 请求解析、过期凭据与错误恢复由合成测试覆盖。SQL 测试使用独立 PostgreSQL 测试库中的合成 auth schema，不使用个人账号。自动化通过不等于个人设备的联网验收。本轮用户已确认 Mac/Windows 正常同步；Android 模拟器覆盖原生 SQLite、Keystore 和真实 HTTPS，只读连接检查不冒充登录后的三端联网。请按第 6 节在个人账号核对 Android 与两台电脑。Android 构建与验证见 [Android 开发说明](ANDROID.md)。

新增直接依赖固定版本：`reqwest 0.13.5`（HTTP/TLS，MIT OR Apache-2.0；[源码](https://github.com/seanmonstar/reqwest/tree/v0.13.5)）、`keyring 3.6.3`（系统凭据库，MIT OR Apache-2.0；[源码](https://github.com/hwchen/keyring-rs/tree/v3.6.3)）、`base64 0.22.1`（仅识别旧 anon key 声明，MIT OR Apache-2.0；[源码](https://github.com/marshallpierce/rust-base64/tree/v0.22.1)）。采用 crates.io 发布包及锁文件校验和，没有复制第三方业务源码。完整实际版本以 Cargo.lock 为准。
