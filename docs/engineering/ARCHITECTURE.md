# 技术架构与实现边界

Android扩展（2026-10-13）：桌面运行入口拆至`src-tauri/src/desktop.rs`，手机入口为`src-tauri/src/mobile/`；两者共用domain/application/infrastructure/sync，不复制任务规则或云协议。Android用独立原生凭据插件、前后台同步调度和单任务WebView，桌面窗口层只在桌面编译。用户已确认桌面双端正常同步；本轮 Android 模拟器与凭据/TLS 验证通过，个人账号三端联合联网仍未验。详见[ADR-0010](../decisions/0010-android-and-three-device-sync.md)及[Android验证](ANDROID.md)。

状态：架构契约与当前实现说明。采用 Tauri 2 + React/TypeScript + Rust + SQLite，决策见 [ADR-0003](../decisions/0003-prototype-implementation.md)。当前存储为 SQLite 中的 JSON 快照；schema 3 加入任务回收站，schema 4 增加控制台设备偏好的兼容边界，schema 5 增加展开模式，schema 6 增加独立手动顺序，当前 schema 7 增加可选同步绑定与基线。按窗口 IPC、固定 DDL、备份与退出的设计见 [ADR-0004](../decisions/0004-data-safety-and-fixed-deadlines.md)，控制台位置见 [ADR-0006](../decisions/0006-console-window-preferences.md)，三列表顺序见 [ADR-0008](../decisions/0008-independent-task-order.md)，双端同步见 [ADR-0009](../decisions/0009-optional-desktop-sync.md)，实际协议见 [数据模型](DATA_MODEL.md)。用户已报告 Mac/Windows 正常同步，个人账号三端联调须另行记录，旧 `5dc3a23` 包不含此能力。第一版同时支持两平台、只管理独立大任务、同时提供控制台和边缘小窗，已由用户明确。早期选型记录见 [ADR-0001](../decisions/0001-platform-and-reuse.md)；窗口职责和任务范围见 [ADR-0002](../decisions/0002-console-and-edge-panel.md)。

## 技术选型

使用 Tauri 2、React / TypeScript、Rust 和 SQLite。平台适配集中在 Rust 平台层，任务与设置通过共享业务服务提交。实际依赖版本以工程清单和锁文件为准。

## 一个应用，两种用户界面

采用常见桌面工具的「完整管理窗口 + 常驻快捷面板」结构。控制台负责集中管理，小窗负责学习时快速查看和勾选；两者不是两套 App，也不各自保存一份任务。首版任务为扁平列表，不设计任务树、子任务、清单步骤或完成进度聚合。

| 原生窗口标签 | 用户职责 | 窗口行为 |
| --- | --- | --- |
| `console` | 今日安排、全部任务、DDL、已完成、回收站、任务编辑和设置 | 普通大窗口；可移动、缩放、最小化/最大化；显式打开时正常取得焦点 |
| `edge-panel` | 上方今日、下方 DDL；快速完成/撤销和简短操作；进入控制台管理 | 小窗；可拖动停靠、默认单击展开/外点收起，可选悬停且不抢焦点、允许固定展开；停靠后遵守屏幕边界 |
| `edge-handle` | 小窗折叠后的边缘触发区 | 内部实现窗口，不是第三个管理界面；不进入任务切换器、不取得输入焦点、不访问任务内容 |

三个标签由 Rust 的 `WindowCoordinator` 统一持有和恢复，每个标签至多一个实例；设置是 `console` 内的页面，不另开常驻设置窗口。UI 请求「打开设置」或「打开任务详情」，由协调器显示/恢复已有控制台并跳转；不能每点一次就创建新窗口。Tauri 的普通多窗口可使用 `WebviewWindow`，窗口标签必须唯一；平台特殊小窗行为由平台层适配。[Tauri 多窗口 API](https://v2.tauri.app/reference/javascript/api/namespacewebviewwindow/)、[窗口配置](https://v2.tauri.app/reference/config/#windowconfig)

### 生命周期与恢复入口

- 用户从应用图标启动或再次启动时，打开已有 `console`；首次使用在控制台完成基本配置。登录启动在已有配置下只恢复菜单栏/托盘和边缘入口，避免主动弹出大窗。单实例回调负责显示、取消最小化和聚焦，不能假定插件默认完成这些动作。[Tauri Single Instance](https://v2.tauri.app/plugin/single-instance/)
- 点击控制台关闭按钮表示隐藏控制台；保存成功或明确处理未保存草稿后，拦截关闭并隐藏。Rust 应用、数据库服务和边缘入口继续运行。边缘小窗关闭只表示收起并解除固定。
- 菜单栏/托盘保留「打开控制台」「显示/收起小窗」「设置」「退出 SideTask」。若托盘创建失败，不把唯一可恢复的控制台隐藏掉；保留可见错误和重试入口。[Tauri System Tray](https://v2.tauri.app/learn/system-tray/)
- 显式「退出 SideTask」才进入正常退出流程：暂停新悬停动作、处理编辑草稿（选择保存时仍允许该次提交），确认退出后停止接收新写入并等待在途事务，最后注销快捷键和托盘并结束应用。取消退出恢复正常操作。退出处理不能被“关闭即隐藏”规则重新拦截；系统关机/结束进程是另外的恢复场景，不承诺阻止系统退出。[Tauri WindowEvent](https://docs.rs/tauri/latest/tauri/enum.WindowEvent.html)、[RunEvent](https://docs.rs/tauri/latest/tauri/enum.RunEvent.html)

### 大小窗口的焦点协调

仅在悬停模式，控制台获得焦点时取消悬停展开计时，并收起未固定的小窗；小窗若正在编辑，应先保留/处理草稿，不粗暴隐藏输入。固定小窗可继续展示并接受明确的勾选，避免把控制台编辑复制到第二个表单。控制台在前台时，指针路过把手不会自动展开小窗；点击把手或快捷键属于显式意图，允许打开。离开控制台后恢复悬停能力，但要求指针先离开触发区再重新进入，防止刚关闭大窗就误弹小窗。

这些规则由同一个协调器处理 `consoleFocused`、`panelPinned`、交互保护和动作代次；不由两个 WebView 相互发“隐藏对方”消息。仅因小窗刷新任务、收到完成事件或应用设置，不应激活任何窗口。

Mac 的小窗和把手显示时保持非激活；用户明确点击快速添加入口、分区或尺寸调整控件后，小窗才通过限定窗口来源的 `focusPanel` 请求取得键盘焦点，把手始终不获焦。请求不重新显示已经隐藏的小窗，执行前检查可见状态；分区/尺寸请求还检查指针位置，明确快速输入使用 `reason: input` 以支持键盘与辅助功能激活，不要求鼠标移动。隐藏或再次显示前恢复非激活状态。无变化的尺寸点按取消保留已经确认的几何缓存，避免一次无操作的隐藏/显示丢掉键盘入口。该 Mac 路径不宣称 Windows 原生焦点已验证；当前真机与工具限制见内部验收记录（本地保留，未随公开仓库分发）。

### 展开模式与外部点击

`Settings.revealMode` 为 click（默认）/hover；只有 hover 使用原生悬停计时。Mac 使用 AppKit NSEvent 的 local/global mouse-down monitor，覆盖左/右/其他键；local 判定目标是否为 panel/handle 或其附属窗口，global 观察其他应用。事件原样交还，不拦截输入，不通过窗口失焦推测点击，不使用需要权限的键盘监控或事件 tap。回调只向协调线程发带时间的消息；显隐/手势/模式的边界挡住迟到事件，固定、拖动、缩放与退出确认不收起。主线程退出时移除 monitor。

外点隐藏只改窗口运行态，任务与快速输入组件保留。Windows 通过独立消息线程的 WH_MOUSE_LL 观察真实按钮按下，经物理事件点、原生 child/root/owner 与 capture/menu 路由区分内外；回调始终放行原点击，只经 channel 唤醒协调器。事件时间戳保留排队年龄，统一状态机保护固定、手势、退出和较新的展开；输入锁仍保护 hover，但不挡显式外点。纯失焦不收起，悬停不激活。生命周期、API/许可及系统边界见[Windows 外点说明](../research/WINDOWS_OUTSIDE_CLICK.md)；单屏验证不关闭完整多屏门槛。

### 小窗快速新增与退出协作

今日标题与底部的＋共用 `QuickTodayAdd`；只发送既有 `createTask(addToToday=true)`，不增加持久化字段或第二份任务库。草稿按窗口在内存注册到 `DraftProvider`；浏览器预览隐藏后保留挂载。退出运行态携带 requestId 与当前处理窗口，按 edge-panel→console 的顺序批准，同一请求两处均完成后才授权退出。小窗批准后保持冻结，任一处取消通过受限原生事件解除冻结；失效请求、越阶段批准被拒绝。

退出请求的前端监听必须指定对应窗口标签：Tauri全局`Any`监听仍会接收`emit_to`，只限定发送目标不能保证阶段隔离。`get_pending_exit`继续由Rust按实际调用方补齐遗漏请求；取消事件全局广播以解冻两窗。该规则的原生复现与回归见内部验收记录（本地保留，未随公开仓库分发）。

## 分层与依赖方向

```mermaid
flowchart TD
    CONSOLE[React 控制台] --> CMD[Tauri 命令与订阅]
    PANEL[React 边缘小窗] --> CMD
    HANDLE[边缘把手] --> WIN[窗口协调器与状态机]
    CMD --> APP[同一个 Rust AppState / 用例服务]
    APP --> DOMAIN[任务/计划/排序规则]
    APP --> REPO[Repository 接口]
    REPO --> DB[SQLite 事务/迁移/备份]
    CMD --> WIN
    APP --> SETTINGS[设置服务]
    SETTINGS --> REPO
    SETTINGS --> WIN
    WIN --> PORT[平台窗口接口]
    PORT --> MAC[macOS AppKit 适配]
    PORT --> WINDOWS[Windows Win32 适配]
```

领域规则不依赖 UI 和系统窗口。前端不直接写数据库；窗口控制只接受明确的用户意图，不允许任意 shell 或文件访问。应用的一个 Rust 核心维护 Repository、用例服务和窗口协调器；各 WebView 仍可能运行在系统独立进程中，不能把“一个应用”误解为所有 UI 共用 JavaScript 内存。[Tauri Process Model](https://v2.tauri.app/concept/process-model/)、[State Management](https://v2.tauri.app/develop/state-management/)

SQLite 是任务和持久化设置的事实来源，React 状态只是各窗口自己的展示缓存。MVP 使用统一写入入口，窗口不直接操作 SQLite 或把 `localStorage` 当另一份主库。Task 只表示一项完整大任务；加入今日是同一 ID 的计划引用，勾选表示整个任务完成。

小窗今日快记沿用 `createTask(addToToday=true)` 与同一TaskService事务，不因快速新增加持久化字段。Rust仅向edge-panel开放带今日计划的创建意图，把手没有写权限。输入行打开前注册独立owner的运行态保护并主动请求焦点，结束后按平台与展开模式恢复焦点策略；共享退出采用前述 requestId 两阶段协议，错窗口、过期或重复回复不能绕过控制台草稿。恢复模式原退出门禁保持独立。

建议模块归属如下，先在现有单个桌面工程内划分，不为首版额外拆微服务或独立后台程序：

| 模块 | 职责 |
| --- | --- |
| `src/surfaces/console`、`src/surfaces/edge-panel` | 两种用户窗口的入口、布局和页面组合；把手由平台窗口层维护 |
| `src/features/tasks`、`daily-plan`、`settings` | 可共享 UI、交互与展示查询；无数据库或 OS 代码 |
| `src/lib/ipc` | 类型化命令、订阅生命周期、快照刷新、错误映射 |
| `src-tauri/src/application` | 任务/每日计划/设置用例、统一命令入口、快照协议 |
| `src-tauri/src/domain` | 扁平任务模型、DDL 和排序等规则 |
| `src-tauri/src/infrastructure` | SQLite Repository、事务、迁移和备份 |
| `src-tauri/src/platform` | 窗口协调器、几何/焦点/显示器/托盘/快捷键及两平台适配 |

以上是职责划分，目录名仍有目标结构与现有实现的差异，实际入口以工程源码为准。共享的是业务规则、组件和协议，不是跨窗口可直接读写的前端 store。

### 跨窗口一致性

当前每次变更携带外层快照的 `expectedRevision`，更新任务、完成/撤销、移入回收站和恢复任务还携带 Task 的 `expectedRevision`。Rust 服务在同一写入锁内校验、交由 Repository 事务提交，成功后才发布新快照；计划和设置目前使用全局快照版本，没有独立的计划修订表。完成/撤销、移入/恢复均使用明确目标状态。旧表单与另一窗口并发冲突时保留草稿、刷新已提交值并提示重新确认，禁止静默最后写入覆盖。

当前 JSON 快照使用 `Snapshot.revision`，整份备份恢复时提高当前全局及任务版本以拒绝旧写入。UI 先完成事件订阅，再请求快照；获取期间收到更新则继续刷新，旧响应不得覆盖较新快照。事件只通知版本变化，不传整份任务内容，也不是任务事实、可靠消息队列或写成功凭证。重新显示、重新连接、睡眠恢复时重新取快照，事件发送失败不能把已经提交的事务误报为未提交。当前协议见 [数据模型](DATA_MODEL.md)。Tauri 支持事件向指定窗口分发，但一致性协议需要 SideTask 自己实现。[Tauri Rust → Frontend](https://v2.tauri.app/develop/calling-frontend/)

### 设备设置与生效状态

任务内容、计划关系和设备设置分别建模。屏幕/边缘/尺寸/延迟、控制台窗口位置、快捷键和登录启动等属于本机设置，不写入 Task。通过设置页换屏时先校验并提交，再隐藏小窗、应用新几何；提交失败保留旧设置和旧窗口状态。用户主动拖动则允许实时跨过接缝，在释放后重新停靠并持久化，不能用程序化迁移规则阻止用户移动。

数据库提交和 OS 窗口/快捷键操作不能组成同一个原子事务，所以返回结果区分「未保存」「已保存并生效」「已保存但应用失败」。应用失败时保留上一套安全的实际状态（没有安全状态则隐藏小窗，保留控制台/托盘入口），显示原因和重试；不把设置开关显示为已经生效。运行态记录 `appliedRevision` 和实际能力，重启后重新应用持久化期望值。主动拖动使用暂态位置，释放时校准为合法停靠矩形后保存；停靠状态缩放仍受边界约束。保存失败提示并恢复上一套合法持久化几何。区分系统回调与用户修改，避免回调写入循环。

### 控制台几何的当前实现

上述设备设置 revision / appliedRevision 是目标模型。当前小窗仍由 `DockRuntime` 持有三项 placement 字段；新增 `ConsoleRuntime` 单独持有控制台普通矩形、最大化偏好、采样代次、候选和错误。数据库仍只有既有 `app_state`：`save_console_placement` 在 IMMEDIATE 事务读取最新 placement，只替换 `console` 子对象，不读取或重编码 snapshot，也不改任务 revision / 比较基线。小窗 `save_placement(snapshot, edge_json)` 则只合并 `monitorName` / `monitorPosition` / `offset`，与该次 snapshot 提交保持原子性；不会用旧完整 placement 覆盖 console。元数据错误回滚对应事务，位置通知独立于任务变更事件。

`console_geometry.rs` 处理屏幕选择、工作区可达性、客户区尺寸和外框差。持久化尺寸与偏移采用逻辑单位，普通矩形与最大化分开，最小化/全屏临时尺寸不覆盖普通记录。`console_coordinates.rs` 在 Mac 将窗口和工作区按各自来源 scale 转成统一 AppKit 逻辑平面；Windows 输入仍为物理桌面像素。屏幕原生原点保留为匹配提示，不混入 Mac 逻辑矩形计算。控制台允许可操作的跨屏位置；标题栏失去可达性或目标屏消失时才校准，工作区过小时放宽本次最小尺寸。

`console_window.rs` 隐藏创建普通控制台，先应用并确认正常几何，再应用并确认可选最大化，最后由明确的启动意图显示和聚焦；失败保留旧记录并显示窗口及错误。原生事件只记代次/投递命令，工作线程在业务锁外采样与调用窗口，稳定变化合并保存，隐藏和批准退出前补一次 flush。部分恢复或最大化失败阻止自动采样覆盖旧记录；“重试保存位置”重新采集并保存当前位置，“不保存本次位置”先采集再忽略，均不回弹到旧目标。退出 flush 的元数据失败如实记录，但不阻止已完成草稿确认的退出；草稿保存失败仍受原退出协议保护。

源码接线与自动化覆盖不等于系统行为通过。本阶段Mac已开始单屏原生复验；默认fullsize内容的标题区另用AppKit contentLayoutRect主线程实测，不从outer/inner高度差推断。Windows另由开发机负责；重启、混合 DPI、多屏变化、最大化和焦点/退出的当前验证范围见内部验证记录（本地保留，不随公开仓库分发）。决定与固定一手参考见 ADR-0006。

### 按窗口限制系统能力

控制台允许完整任务管理和设置命令，小窗只获得完成/撤销、计划、今日/DDL 排序和快速新建等必要能力；完整任务编辑按 Task ID 跳转控制台，把手仅能请求展开/位置调整。`reorderToday` 允许两个任务窗口调用；`reorderTasks` 的 all 仅允许控制台，deadlines 允许控制台和小窗，内部边缘把手没有任务写权限。窗口创建、退出、备份文件访问和系统设置由 Rust 入口集中校验。Tauri capability 以窗口标签划定权限；自定义应用命令默认可被各窗口调用，初始化时必须显式纳入权限清单或执行调用方校验，不能只隐藏按钮。事件只发给需要刷新的已知窗口，也不经事件触发可信写入。[Tauri Capabilities](https://v2.tauri.app/security/capabilities/)

### 任务生命周期与恢复边界

回收站使用既有 Task 和统一变更入口，不创建第二份任务库或新窗口。`trashTask` / `restoreTask` 仅允许 `console` 调用，边缘小窗和把手拒绝这两个动作。`deletedAt` 与完成状态独立：移入只写删除时刻和任务版本，恢复只清除此标记并更新任务版本；Task ID、标题、备注、DDL、完成状态/时间和所有日期的 Plan 均保留。同状态重试仍先检查版本，检查通过才按幂等规则处理。已删除任务拒绝普通编辑、完成和计划变更，具体校验见 [数据模型](DATA_MODEL.md#schema-3-实际兼容与生命周期契约)。

UI 从同一快照派生普通列表与回收站。正常列表、全局搜索、小窗和计数排除已删除记录；回收站独立搜索并展示原完成状态。全量 Task 索引仍保留已删除记录，使另一窗口删除任务时，正在编辑的详情能够保留本地草稿并显示冲突。详情是否挂载不能只由当前列表成员资格决定；行内、详情或提示条恢复同一任务均不得丢弃草稿，保存前需显式重新确认最新版本。移入回收站前的“继续编辑 / 保存后移入 / 放弃修改并移入”与恢复后“查看任务”的导航保护遵循 [UX](../product/UX.md#回收站与任务恢复)。后台同步不主动抢焦点；显式查看才把焦点移到详情。

生命周期阶段曾将数据库 schema 1 / 2 升到 3，控制台元数据、展开模式与排序阶段分别升到4/5/6；当前识别1–7，旧1–6先对完整四文件检查副本验证、创建 `before-schema-7` 一致性备份，再事务升级到7，保留 snapshot / placement 原字节并初始化独立 sync 记录。启动时坏库恢复入口只在真实服务未建立时使用，与正常回收站恢复不同；不得以空服务伪装健康空库。可移植 JSON 导出 v3、读取 v1 / v2 / v3，包含回收站、全部计划及两份独立手动顺序，不包含同步身份、基线或凭据。绑定同步时拒绝整份导入预览和执行，须先断开；之后先备份，再替换任务/计划/顺序并保留本机设置及 console / edge 偏好。旧版 JSON 按整集合恢复，缺失的手动顺序默认空，不隐含合并。文件格式和旧版校验细节见数据模型。

本轮只提供单任务软删除和恢复，不提供自动过期清空、永久删除或批量清空。方案来源与范围见 [生命周期计划](../research/TASK_LIFECYCLE.md)；当前验收范围见内部验证记录（本地保留，不随公开仓库分发），尤其不能从浏览器回归推断 Windows 或多屏通过。

### 三列表排序的职责边界

今日使用原有 `Plan.sortOrder`，全部任务和 DDL 分别使用 `Snapshot.taskOrder` / `deadlineOrder`。三份顺序属于列表，不写入 Task；任务勾选、DDL 修改和计划关系仍只有一份事实。`reorderTasks` 通过相同 TaskService 事务校验完整活动集合与旧快照版本，只有 DDL 重排同时把本机 ddlSort 切成 manual，不改变截止信息或任务修订号。

前端 `useTaskReorder` 管理指针捕获、插入线、滚动与键盘意图，复用 `VirtualTaskList`，不把已挂载的可见行当成完整集合。手势开始记住列表上下文和版本，松手只提交一次，拖回原位或取消不提交；失败继续显示已提交顺序。已拖动行在虚拟滚动中保持挂载，小窗每个排序会话持有独立交互保护。搜索、已完成及回收站不开放部分集合排序。两个任务窗口共用同一已提交快照，不保存第二套本地列表顺序。

## 多屏：不把「收起」实现成移出屏幕

以下防溢出规则适用于 `edge-panel`、`edge-handle` 停靠后的显示、展开/收起与缩放，以及相关弹层。显式 `DraggingDock` 状态允许用户主动移动经过屏幕接缝；释放/取消后先恢复合法停靠矩形，再恢复普通展开/收起规则。不能把“收起不溢出”实现成禁止移动或永久绑定某屏。`console` 是普通系统窗口，允许正常跨屏或横跨两屏；恢复时确保标题栏和基本操作区可见。

维护一个边缘面板与独立的窄把手窗口。折叠时边缘面板真隐藏，只有把手可见；把手完全在目标屏幕内部，不额外拦截邻屏鼠标。展开过程：

1. 重新取目标屏幕工作区和缩放比例，解析已保存的逻辑宽高与边缘偏移。
2. 在边缘面板隐藏状态下计算合法外框，先设置位置和尺寸。
3. 显示面板。动画仅改变内部内容/透明度并裁剪在固定外框内，不移动原生窗口越过屏幕边缘。
4. 收起时取消交互计时，内部淡出后隐藏边缘面板；减少动态效果模式直接隐藏。

原生窗口操作必须串行协调并附带操作代次标记。等待位置/尺寸设置成功后，再确认屏幕仍存在、目标矩形仍合法且当前仍要求展开，才允许显示。显示器变化、显式关闭或新请求使旧计时器/异步回调失效，避免过期的展开动作重新弹出窗口。

系统阴影可能不属于 `outerRect`。优先关闭原生外阴影、在窗口内部绘制；保留时必须预留实际阴影范围，并验证物理像素截图。日期选择器、菜单、工具提示也须定位约束，不能只约束主窗口。

### B33 小窗坐标与尺寸会话的当前实现

Windows 辅助窗显式设置最小客户区尺寸为 1×1 DIP，正常控制台保留自己的最小尺寸。旧分支150%单屏曾在首显采样到把手从27px短暂扩为202px，属于内部验收记录（本地保留，未随公开仓库分发）。后续 `windows_visibility.rs` 先以 `SW_SHOWNA` 保持已确认框，再同步 Tao 可见状态，并为迟到主线程回调保留显示事务取消；该 Windows 修复随本轮整合纳入，Mac 显示路径保留。显示后继续严格核对实际框与scale，失配先隐藏、确认隐藏，再重新定位并仅重试一次；再次失败保持隐藏/作废缓存/报告，不缓存错误边界。e4217f4 的原生A/B结果见[生命周期说明](../research/WINDOWS_WINDOW_LIFECYCLE.md)。

Windows 最终退出在 requestId 草稿协议明确批准后才关闭各 WebView2 controller；取消退出、普通控制台关闭和小窗隐藏不进入清理。正常退出及恢复后重启的清理路径保留平台隔离；旧 e4217f4 未再出现1412的样本不替代整合版真实退出复验。

启动入口按 console / edge-panel / edge-handle 分开动态加载；整棵对应应用载入后再挂载 Store 与导航桥，避免桥先消费导航而目标处理器尚未安装。启动恢复门禁和数据库快照契约不变，加载失败发生在草稿挂载前，可安全重试。暂停小窗后协调器跳过屏幕/鼠标/几何查询，500ms 检查设置；原生失败仍按正常频率重试。Windows 主键查询遵循 `SM_SWAPBUTTON`，托盘左键只投递既有控制台恢复服务，不在主回调里等待原生几何。

`edge_coordinates.rs`将Mac的cursor/monitor/window分别按主屏/目标屏/窗口自身scale归一到全局逻辑平面。停靠形状仍按真实目标DPI在局部物理像素计算，再投影成逻辑矩形交Logical setter；Windows继续使用物理桌面坐标和Physical setter。原始显示器原点仅作持久化身份提示，真实DPI参与缓存签名。工作区必须包含在屏幕内，NaN、零尺寸、不可表示坐标拒绝定位。

隐藏迁移遇到不同DPI时先用小外框选中目标屏，确认scale后设置最终外框；Windows `WM_DPICHANGED` 会再次调整尺寸，不能仅凭setter返回成功。显示前要求两次实际外框采样与目标一致且在工作区内，并复核屏幕签名；失败隐藏辅助窗、作废缓存，控制台保留反馈/重试入口。同屏实时缩放仍先缩小、再移动、再放大；确认期间不盲目修复可见窗口，失败退回隐藏恢复。未变化的把手跳过重复定位。

尺寸IPC采用带session的`start/preview/commit/cancel`。只有start能创建会话，需匹配开始时完整Settings；preview只更运行态，commit一次事务保存尺寸与沿边位置。取消/设置冲突/显示器变化/提交失败结束会话并恢复原placement；后续旧preview或commit不可重启。屏幕查询失败也取消并隐藏，不留下活动预览。设置在dock锁后读取，业务锁不跨原生调用；提交前复查最新设置与屏幕签名。前端尺寸仍为CSS逻辑单位，不乘devicePixelRatio。

固定上游语义、许可证与自动化/原生证据边界见[小窗坐标复查](../research/EDGE_COORDINATES.md)。这不是对Windows混合DPI、Mac输入命中或睡眠恢复的通过声明。

### 几何不变量

统一使用一种平台边界内的坐标表示后，工作区为 `W=(x,y,w,h)`，面板外框为 `P`，应用边距为 `m`：

```text
P.left   >= W.left + m
P.top    >= W.top + m
P.right  <= W.right - m
P.bottom <= W.bottom - m
```

`m` 与尺寸需在极小工作区内一起钳制；有效最大尺寸优先于配置最小尺寸。宽高由逻辑单位按当前屏幕变换，不直接把 CSS px、AppKit point 与 Windows physical px 混算。负屏幕坐标是合法情况，不截成零。原点翻转只由平台层转换一次。

拖动把手/标题空白区可改变沿边位置，并可主动换边或跨屏重新停靠，不要求先打开设置。协调器进入 `DraggingDock` 后保存旧停靠快照、固定/展开状态和动作代次，取消 hover/hide 计时，抑制程序化定位及系统回调写库。跨 DPI 期间处理系统尺寸变更，但不把它误当成一次新的用户配置提交。

释放后以指针所在可用显示器和最近合法左/右停靠边计算候选位置；接缝优先使用平台指针所属屏幕，仍歧义则取最近有效候选，无候选则取消。校准缩放、尺寸、外框后吸附，再带开始时设置 revision 一次保存屏幕标识、边缘和沿边偏移。几何校准不能通过滑出旧屏的收起动画实现。原生拖动取消、非正常指针捕获丢失或无效落点取消迁移；Esc 能否由原生拖动会话接收需要平台 POC，不能依赖未获焦 WebView 的按键事件。

拖动保留原有 pinned/展开状态，折叠把手不因拖动自动展开。取消或普通保存失败恢复最近已提交的合法停靠；目标/旧屏断开则重新枚举并安全回退。控制台并发位置更新或屏幕变化应使旧拖动代次失效；出现 revision 冲突时采用最新已提交设置，不能拿旧快照反向覆盖它。恢复悬停前清理旧回调并等待指针离开触发区。固定展开不影响拖动权限。

普通缩放使用内侧边/角手柄，平台层在应用新矩形前约束；仅监听 `onResized` 后纠正可能已经出现一帧意外越界，不符合停靠状态要求。主动拖动和缩放分别建模，不能因为拖动例外而放宽自动展开/收起的约束。

### 显示器变化

持久化显示器标识、边缘、相对偏移、逻辑尺寸，不保存枚举下标作为身份。标识不可稳定识别时用可解释的匹配策略并回退主屏；不保证所有系统都提供永久稳定 ID。

启动、显示器连接/断开、排列/分辨率/缩放变化、Dock/任务栏变化、睡眠唤醒时重算。展开前也刷新工作区。目标屏幕消失时先隐藏边缘面板，安全迁移把手，保护未提交编辑。操作系统热插拔瞬间可能主动重排窗口，必须真机观察，不能只靠公式宣称绝对无任何瞬态问题。

## 焦点、悬停与系统能力

- 悬停查看不激活应用；用户点击编辑再允许输入。Tauri 通用 API 不一定覆盖全部焦点语义，必要时使用 AppKit / Win32。
- 使用明确状态机、可取消计时器和交互保护计数；输入法、菜单期间不因悬停离开自动收起，拖拽/缩放期间两种模式均保护；单击外点可隐藏编辑并保留草稿，详见 UX。
- 把手检测优先使用窗口局部进入/离开事件和平台能力；若需指针采样，应有低频/空闲策略并测 CPU，不能无期限忙轮询。
- 托盘/菜单栏提供显式恢复入口。
- 默认不请求与核心场景无关的权限；若 POC 发现某平台实现需要额外权限，记录原因并评估替代路径。
- 普通桌面和最大化窗口是基础要求；全屏、macOS Spaces、Windows 虚拟桌面必须分别验证。Windows 的所有虚拟桌面可见能力不能由 Tauri 通用接口直接承诺。独占全屏/安全桌面等不作为首版保证场景，必须在兼容性说明中明示。

## 可选 Mac 与 Windows 同步

本机 SQLite 仍承担日常读写。`src-tauri/src/sync/` 分离纯数据投影与合并、Supabase 传输、系统凭据和同步运行态；React 的 `SyncSettings` 只负责配置、展示状态和用户选择，经 `src/lib/sync.ts` 调用受控 IPC。没有第二份 WebView 任务主库，也不把同步网络请求放入组件或窗口平台层。项目初始化见 [Supabase 配置说明](SUPABASE_SETUP.md)。

云文档仅包含 tasks、plans、taskOrder、deadlineOrder；今日顺序随 Plan.sortOrder 同步。Settings、placement、使用说明标记、草稿和本机修订号不参与跨设备状态。客户端从上次基线、本机已提交数据、当前云文档进行三方合并；独立修改自动合并，冲突暂停上传并要求逐项选择。应用合并结果时保留本机偏好、提高受影响任务和快照的本机版本，既有编辑冲突保护继续生效。

Supabase 以登录用户拥有的单份版本化文档保存数据。SQL 启用 RLS 并撤销客户端直接写表权限，RPC 从 `auth.uid()` 确定用户，不接受客户端任意指定所有者。提交携带预期云版本，相同内容重复提交保持幂等，版本竞争重新读取合并。Rust 在网络等待期间不持有本地任务写锁，提交回本机前再次核对本机版本；snapshot 与新的同步基线由同一 Repository 事务保存，失败不发布虚假成功。

后台以短延迟合并本机更改，并定期检查云端版本；没有依赖 Supabase Realtime 来提供离线一致性。离线保留本机修改，恢复网络后继续尝试。`sync_status` / `sync_sign_in` / `sync_sign_out` / `sync_now` / `sync_resolve` 仅允许控制台调用，`sidetask:sync-changed` 只向控制台发布同步状态；普通任务变化仍沿用既有提交后通知更新大小窗。

邮箱密码只用于本次登录。访问/刷新令牌由 Rust 保存在 macOS 钥匙串或 Windows 系统凭据库中，不返回 WebView、不进入 SQLite 或 JSON 备份；公开项目配置和账号绑定与无令牌基线存于 schema7 的独立 sync 记录。系统凭据库不可用会报错，不降级成明文文件。断开先停止自动上传，清理失败保留绑定并允许重试；清理完成后本机任务和云端文档仍保留。

这些是当前源码协议，不是生产上线或双端实测结论。尚需实际项目初始化、账号隔离/版本竞争验证、两平台凭据库和 Mac ↔ Windows 断网重连验收；新同步包的构建与上传必须另有 CI 证据，不能沿用旧包结果。

## 可靠性与交付

数据保存在系统应用数据目录；导出为有 `schemaVersion` 的 JSON，恢复前先备份、校验，再事务导入。数据库在线备份使用 SQLite 备份机制或一致性快照，不直接复制一个正在写入的单独 `.db` 文件。

任务状态、完成时间与修订号原子提交；写失败不广播成功。崩溃恢复、磁盘写失败、迁移失败、重复点击等用例必须验证。

POC 确定最低 OS / CPU 架构矩阵。之后使用 macOS 与 Windows 各自构建执行器生成安装包；自动构建成功不等于多屏窗口行为通过。签名、公证、自动更新是各自交付事项，内部试用包可先明确未签名状态，公开发布前完善分发方案。

## 官方依据与限制

以下是 API 依据，不能替代项目实测：

- [Apple NSScreen.visibleFrame](https://developer.apple.com/documentation/appkit/nsscreen/visibleframe)：工作区排除 Dock / 菜单栏，不应缓存不更新。
- [Microsoft MONITORINFO](https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-monitorinfo)：`rcWork` 使用虚拟屏幕坐标，允许负值。
- [Tauri DPI](https://v2.tauri.app/reference/javascript/api/namespacedpi/)：逻辑/物理尺寸和位置要显式区分。
- [Tauri Window API](https://v2.tauri.app/reference/javascript/api/namespacewindow/)：当前文档有 monitor workArea/scaleFactor；对锁定的发行版本需再次核实。`setVisibleOnAllWorkspaces` 不支持 Windows；macOS 已获焦窗口不能只靠 `setFocusable(false)` 取消焦点。
- [Apple windowWillResize](https://developer.apple.com/documentation/appkit/nswindowdelegate/windowwillresize(_:to:))、[Windows WM_SIZING](https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-sizing)：可用于约束将应用的窗口尺寸。
- [Apple 屏幕参数变化通知](https://developer.apple.com/documentation/appkit/nsapplication/didchangescreenparametersnotification)、[Windows WM_DPICHANGED](https://learn.microsoft.com/en-us/windows/win32/hidpi/wm-dpichanged)：屏幕环境变化需要处理。
- [Apple nonactivatingPanel](https://developer.apple.com/documentation/appkit/nswindow/stylemask-swift.struct/nonactivatingpanel)、[fullScreenAuxiliary](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct/fullscreenauxiliary)：描述非激活面板及全屏辅助窗口的系统机制。

### 首次常驻说明

说明确认状态属于本机设备元数据：`placement.usageGuideSeen`，专用console-only命令和Repository IMMEDIATE合并事务。不经业务mutate、不改Settings或任务revision，不因确认而重新应用窗口。前端hook独立处理读取/保存代次，慢读取不能撤回已完成确认；成功才隐藏，错误与重试不卸载任务/设置草稿。正常控制台内联提示，设置保留帮助；恢复模式没有正常AppState，不展示或调用该功能。
