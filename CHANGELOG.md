# 更新日志

本文件从 2.0.0 起逐版本记录 A / B / Server 与可选 APK 的更新要点；更早版本和完整发布说明见
[GitHub Releases](https://github.com/Andrea-lyz/ommega/releases)。文末保留仍影响当前行为的
早期修复摘要。

版本号在 A / B / Server / B-app / StrongBoxCapabilityMask 之间统一。除特别说明外，一次
发布中只更新正常发布的模块 ZIP 不会替你更新 Server，也不会安装可选 APK。

README 不再内嵌逐版本更新要点：发版时只更新版本号，并在这里追加条目。发布步骤见
[RELEASING.md](RELEASING.md)。

## 2.4.2 — 2026-11-12

A 端自动续签、本地出证时序与 WebUI 暗色显示（仅 A 端；B、Server 无改动，无需单独更新 Server）。

**自动续签的三个独立缺陷**

- **会漏签。** 续签线程原先用相对的 `thread::sleep` 等待，在 Android 上该等待基于不计入深度
  休眠的时钟。于是「7 个清醒小时后到期」的等待会跨过整夜休眠继续挂着：TEE 证书进入续签窗口、
  甚至越过到期时间，也从未开始签发，页面与日志都显示健康——因为根本没有发起过尝试。现在改为在
  **绝对墙钟截止时间**上等待（`clock_nanosleep` 配 `CLOCK_REALTIME` 与 `TIMER_ABSTIME`）：
  休眠经过的时间与系统校时都计入，信号中断后沿用同一截止时间而不是重新计时，其他内核错误记录后
  回退两分钟、不会忙循环。等待仍限制在 2 分钟到 1 天之间。**它不会唤醒休眠中的设备**，到期的
  截止时间会在设备恢复运行时立即处理。
- **自己把自己锁死。** `auto-run` 先取签发锁，随后调用的 `issue()` 又对同一个文件加锁；`flock`
  按打开的文件描述持有，同一进程内第二次必然失败。于是每一次无人值守续签都报「已有签发在进行中」
  而从不真正签发，6 小时退避又把重试推到更后面。现在拆成调用方持锁的 `issue_locked()` 与自行
  取锁的 `issue()`。
- **失败不显示。** 无人值守的失败原先只写进 `feedback.json`，而它在下一次状态读取时就被覆盖；
  `policy.json` 的 `last_error` 一直是空，页面因此长期显示健康的卡片。现在把原因和尝试时间一起
  持久化，在 `status` 中暴露并渲染到 RKP 卡片上。

**本地出证时序**

- 本地出证（attested）的 `generateKey` 与未校验的一次消耗相同的 secure-world 时间片，成对比较
  两者的校验方因此测得接近零的增量：10 组配对里有 4~6 组差值为非正，`dAtt/initSign` 停在 0.1
  附近。普通 Operation 延迟补不上这个差，因为两条路径都要付。
- 新增 `RetrieveCertSigningInfo` 钩子，在本地叶证书成功编码之后、密钥可被封存或被观察到之前，
  计入一次采样自 24–40 ms 的证书开销。覆盖热 keybox 缓存、调用方指定的本地签发者、本地出证的
  认证兼容密钥与允许的远程回退；自签名叶、生成失败与远程返回的链不计入。
- A 端实测：`dAtt` 2.0/−2.2/−2.9 ms 变为 32.9/39.2/35.0 ms，非正配对 4/6/5 变为 0/0/0。

**WebUI 暗色**

- 诊断页日志框在暗色下不可读。两个缺陷叠加：底色引用 `--om-surface-alt`，而这个变量从未被任何
  主题定义，永远落到浅色字面兜底；唯一的暗色覆盖又挂在 `.om-modern[data-theme=dark]` 上，但主题
  实际写在 `:root`，那条规则不可能命中。实测暗色对比度仅约 1.04。现在浅色与两条暗色主题路径都
  定义了该变量，日志框只引用 token，实测为 `rgb(27,30,39)` 底配 `rgb(240,239,247)` 字，满足
  WCAG AA。

**随包组件**

- 模块内置的 AProbe 从 1.6（versionCode 7）更新到 1.8.8-paytm（versionCode 19），带聚焦的时序
  诊断。签名身份不变（`CN=Ommega AProbe`），可直接覆盖升级。

升级说明：只更新 A 端模块 ZIP 即可，B 端与 Server 无需改动。

- 自动续签修复位于 keymint 守护进程内，**必须通过模块部署流程重载，仅替换 `ommega-rkp` 工具
  不会生效**。安装后原有用例、已导入的种子与已签发的 keybox 全部保留。
- 出证时序改动在 KeyMint 与 TA 两侧，同样需要重载守护进程生效。
- WebUI 修复需要模块重新打包后刷新页面；建议同时强制刷新一次以避开 WebView 缓存。
- 若你的 TEE 证书已经临近或越过到期时间，本次更新不会追溯补签：部署后请在「设置 → 自动化」
  确认状态，必要时手动签发一次。手动签发会消耗一次注册身份，属于设备侧操作。

验证：CI 全绿（A、B、Server、APK、UI）。新增 7 项续签计时回归、3 项主题对比度回归与出证时序的
配对断言，均在本地通过；把实现改回相对等待、恢复字面兜底或去掉时序计入时对应回归会失败。暗色
日志框经桌面 Chromium 以同一份 CSS 实测计算颜色确认；出证时序在 A 端实测（见上）。**尚未在真机
WebView 内验证，也未在设备上验证自动续签能按时触发**，这两项仍需实机验收。

## 2.4.1 — 2026-11-09

本地模式下的诊断修正（仅 A 端；B、Server 无改动）。

- 修复本地模式下诊断页误报「当前模式：中继」：模式改为读取守护进程的 `data/relay-mode`。
- 修复调试日志总阀切换后「已完成复现，解锁诊断」按钮不立即出现（状态刷新被节流）。
- 请求异常把 keystore 代理（uid 1017）上的失败归因到真正发起请求的应用，不再被当作系统身份隐藏。
- 共享 uid 的应用（例如 Google Play 服务 10120 / 99910120）能正确解析包名。
- 受管列表之外的应用通过 KernelSU 桥接取真实应用名（如「融合搜索服务」「Google Play 服务」）。
- 诊断测试增至 10 项，覆盖失败归因与共享 uid 解析。

升级说明：只更新 A 端模块 ZIP 即可，覆盖安装保留数据与已签发的 keybox。

## 2.4.0 — 2026-11-09

A 端诊断分级页面与设备体检（仅 A 端；B、Server 无改动）。

- 设置里的诊断改为分级页面：一个调试日志总阀、复现后解锁、七个功能子页（请求异常 / 设备体检 /
  KeyMint / 注入器 / 中继 / 属性视图 / SOTER），异常按应用聚合显示名称、包名与原因，概览页同步
  展示。
- 设备体检改为 WebUI 内原生检查；AProbe 作为可选组件打包发布签名版，可一键安装并自动加入属性
  视图与自动授权策略。
- 指纹签名证书新增签发 keybox 选择：TEE 或 StrongBox 槽位二选一，可直接导入 Samsung StrongBox
  keybox；槽位缺失时回退并明确提示。
- 认证绑定的指纹签名改由 A 端本地出证，中继模式下不再出现 `KEY_USER_NOT_AUTHENTICATED`。
- 设备属性视图修复：向 `*_for_attestation` 发布目标身份、跳过无法改写的长属性、宿主缺失属性不再
  中断视图（解决部分应用的 -66）。
- 日志控制：总阀同时接管本地与中继的 KeyMint / 注入器等级，新增清除日志、日志轮转上限与 SOTER
  详细叙述。
- keybox 槽位与签发 keybox 选择文件的属主统一为 `keystore:keystore` 0600，导入与启动时自动
  归一化。
- WebUI 切换更跟手：诊断数据缓存与节流、日志有界读取、应用列表离屏渲染。

升级说明：只更新 A 端模块 ZIP 即可。

## 2.3.1 — 2026-11-02

修复 2.3.0 安装包缺少 `ommega-rkp`。

- 2.3.0 的模块 ZIP 内包含 `ommega-rkp`，但安装脚本逐个解压二进制，没有把新工具加进列表，安装后
  「设置 → 自动化」整页状态显示不可用（工具不存在）。
- 现在安装脚本按 ABI 解压并校验 `ommega-rkp`，缺失时安装会直接失败而不是静默留空。
- CI 新增门禁：`prepare` 会核对 `build.py` 打包的每个二进制都出现在安装脚本里。
- 本版仅安装脚本变化，A 端功能与 2.3.0 相同；已装 2.3.0 的设备直接覆盖安装 2.3.1 即可，数据与已签发
  的 keybox 全部保留。

## 2.3.0 — 2026-11-02

A 端自动化页与 RKP 签发（仅 A 端；B、Server 无改动，无需单独更新 Server）。

- 「设置 → 自动化」由浮窗改为完整二级页面，移除原有的四项 keybox 下载来源：改为导入用户自备的
  CDI 材料包与 keybox，TEE 与 StrongBox 各自一个槽位，互不复用。
- 材料包沿用 RKP 工作台的 version 1 约定（`dice_chain_hex` / `signing` / `device` /
  `issuance`）。内容以 AES-256-GCM 加密保存于 `data/rkp/seed.enc`，密钥是每台设备随机生成的
  `data/rkp/master.key`（均 0600）；从其它设备拷回的密文会显示「无法解密」，而不是等到签发时才
  失败。模块不内置任何种子或证书。
- 移植 Duck ToolBox 的 RKP 引擎（MIT，`a-side/source/rkp-engine`）：每次签发取新 challenge、
  生成新的 P-256 证明密钥、用导入的叶身份签 CSR v3，校验返回链后导出 keybox。签发结果与手工导入
  走同一套槽位校验，链的级别与 `O=` 必须和所选槽位一致。
- 新增 `ommega-rkp` 工具：`status` / `due` / `seed-import` / `seed-remove` /
  `keybox-import` / `policy` / `issue` / `auto-run`。签发前会按目标槽位自动改写材料包里的
  `security_level`，避免用为 TEE 捕获的材料去签 StrongBox。
- 自动续签：按服务器下发的 `time_to_refresh_hours` 计算临期。keymint 守护进程启动 5 分钟后首次
  检查，之后**睡到下一个临期时刻**（上限 1 天，空闲时每天一次），失败后 6 小时内不重试；与手动
  签发共用文件锁，网络与 keybox 写入都在子进程，不会阻塞 TA。
- 应用策略页在本地模式下自动隐藏「认证签名策略」「本地 keygen 时序整形（EWMA）」「CPU 与设备
  属性」三张卡片，远程模式照常显示。
- 修复本地模式下顶部刷新总是提示「部分状态无法读取，请重试」。
- 修复 keybox 到期时间解析把 UTCTime 的两位年当成四位年，导致显示成 2611 年。

升级说明：只更新 A 端模块 ZIP 即可，B 端与 Server 无需改动。更新后刷新 WebUI 生效；keymint 由
部署流程重载，无需重启设备。

验证：CI 全绿（A、B、Server、APK、UI）。真机 PJZ110 上验证了 TEE 与 StrongBox 两级签发并安装
到对应槽位（链为 `O=TEE` / `O=StrongBox`，各 4 张证书）、引擎 29 项单元测试在设备通过、自动
续签首轮判定「未临期」且未消耗请求。

## 2.2.2 — 2026-11-01

A 端 SOTER 多身份与应用列表计数。

- A 端 SOTER TA 支持按应用分配身份：本地随机、导入私钥、远程中继三类，每个应用单独指定；
  未指定的应用使用默认身份。身份存储为 `/data/misc/keystore/ommega/data/soter/identities.json`
  （0600），解析失败一律拒绝服务。
- A 端新增 SOTER 总控开关（身份主页顶部）：只写看门狗标志，关闭后所有应用回落到原厂 HAL，
  分配关系保留；界面不再提供「重启软件 TA」。
- 应用列表新增「SOTER N 次」角标，与「证书 N 次」同款胶囊、不同配色，仅在总控开启时显示；
  短名称与徽章同行，长名称整组换行，名称不被截断。
- SOTER 身份管理改走 Rust 管理接口（`soter-read` / `soter-write` / `soter-import`）：目录锁、
  比较-替换、原子替换 + fsync + 读回校验；私钥只在设备端按路径读取，不经过页面。
- 中继身份按应用生效，可配置 UID 映射与跳过 TLS 校验；具名中继不再收到本机账本设备 ID。
- 反回滚计数器改为每应用只增不减（带时钟下限），修复微信开通时
  `counter is not greater than that in server`。
- 看门狗状态备注改为按状态推导：停用后不再残留旧的启动失败原因。
- 修复自动认证签名的兼容性回归。

升级说明：本版只有 A 端模块与界面变化，B 端与 Server 无代码改动（版本号统一）。A 模块更新后
`service.sh` 会重启注入器与裸服务，SOTER 开关与身份配置保留；微信等已绑定应用无需重新开通。

验证：本地 Rust（fmt / clippy `-D warnings` / 52 单测）与界面（148 项，含浏览器验收）全绿；
A 端实机完成微信指纹支付开通、`SoterChecker` 全项通过与按应用中继回环验证；本版无 B 端改动，
未对 B 做新的实机复测。

## 2.2.1 — 2026-10-31

- B 端 service 启动时关闭 wlan0 Wi-Fi power-save；设置在本机重连后保持，降低 relay 抖动。
- A/B KSU 更新元数据与 2.2.1 版本保持一致。
## 2.2.0 — 2026-10-31

- A/B 模块加入 KSU/Magisk `updateJson`，发布后可从 KSU 管理器检查并升级。
- 新增 `v*` tag release workflow：自动构建 A/B 模块、生成 `ommega-a-update.json` / `ommega-b-update.json` 和稳定别名 zip，发布 GitHub Release。
- A 端 local-keygen EWMA 差值目标调整为 85ms；Duck Detector 时序信号清除。
- 保留 A/B/Server 2.2.0 统一版本号和既有功能。
`r`n## 2.1.4 — 2026-10-29

B 端 WebUI 状态卡修复。

- Server connection 卡片不再依赖 WebView 里的 curl/wget：设备有 `curl` 时优先读取
  Server 的 `/api/status/`；没有 `curl` 或请求不可用时，读取 relay 日志最后一条
  `b/poll status=`，显示 200/204、429、401/403、5xx 或 stale。
- 修复原生 Android（没有 `/system/bin/curl`）上误报 “Invalid server response” /
  “Some status reads failed” 的问题；relay 实际 poll 204 时卡片显示 “Server connected”
  和最后 poll 时间。移除不兼容的 busybox wget 状态读取路径。
- 本次只改 B 端 WebUI；B relay 代码、A/Server、`service.sh` 与 TEE/KeyMint 未改。
  Server 2.1.3 的 poll 独立配额修复保持不变。
- A/Server、B-app、StrongBoxCapabilityMask 相对 2.1.3 只有版本号变化。升级时只需更新
  B 模块即可获得状态卡修复；两个 APK 仍为 debug 签名。
- 验证：B 端刷新/状态测试 26/26 通过，UI checks 与 master CI 全绿；客户测试设备状态卡
  显示 “Server connected / Relay last poll succeeded (HTTP 204)”。

## 2.1.3 — 2026-10-29

- **Server**：B 端 `GET /api/b/poll/` 心跳从 relay 工作配额中拆出，使用独立的
  `RELAY_POLL_RATE_LIMIT_REQUESTS`（默认 0，不限制）。2.1.2 的 80 ms 短轮询约
  12.5 次/秒（约 4.5 万次/小时），会在大约 27 分钟内打满默认 2 万/小时的工作配额，
  随后返回 429 并按 `Retry-After` 退避；期间 B 端从在线列表消失、任务延迟。现在心跳
  不再消耗 A 任务和 `b/result` 的配额，工作配额仍由 `RELAY_DATA_RATE_LIMIT_REQUESTS`
  约束。
- **A/B**：`version` 权限标签改为从 B 端真实 TEE 的 `profile` 快照读取（一次性
  AES-256/GCM 密钥，读取后立即删除），本地生成/导入与远程证明使用同一来源；新增证书与
  profile 不一致时拒绝，旧 relay 保持兼容并告警。同时保留 tag 701
  `creationDateTime` 的修复。
- **A**：远程签名算法改为从密钥特性读取，修复 `missing tag Algorithm` 导致的
  `SERVICE_ERROR` / `Failed to begin operation`；注入器接受 Android 的非零
  Parcelable presence 标记（0 仍为 null），避免同一密钥出现分裂的 keystore 视图。
- **A**：软件 crypto 与 KeyMint 对齐：3DES ECB/CBC PKCS#7、NONEwithRSA、AES-CTR 的
  16 字节初始计数器、AES-192-GCM，以及 12–16 字节 GCM tag；KmProbe 在 A 上达到 10/10。
- 升级：Server 需单独替换 `relay_rs`（poll 修复在 Server 侧）；A/B 模块按需更新；
  B-app 与 StrongBoxCapabilityMask 相对 2.1.2 只有版本号变化。

## 2.1.2 — 2026-10-29

- 修复 A 端应用页大多数应用只显示包名、拿不到应用名称的问题。管理器把应用信息桥作为
  Android WebView 注入对象提供，只有经该对象调用才有效；旧代码先把方法摘下来再调用，
  每次都被 WebView 以「Java bridge method can't be invoked on a non-injected object」
  拒绝，于是除随包的 `applist.json` 外，所有应用名都退回包名。现在批量查询和逐包重试都
  保留实时 `ksu` 接收者，应用页恢复显示真实名称。
- 浏览器预览桥同样拒绝“摘下再调”，冒烟测试与新增回归用例在该模式下运行，避免同类回归
  再次只在真机上暴露。
- 本次只有 A 端 WebUI 变化；B 端、Server 与两个可选 APK 相对 2.1.1 只有版本号变化。

## 2.1.1 — 2026-10-29

- 修复 A 端应用页搜索框在冷启动时必须点两次才弹出键盘、且中文候选词无法输入的问题。
  Android WebView 不会为随页面解析出的输入框建立文本输入连接（系统侧为 `TYPE_NULL`），
  中文提交因此被丢弃，而按键类英文仍能进入，表现为“英文可输、中文不行”。现在进入应用页时
  先重建输入框且不抢占焦点，首次点击即可弹出键盘并正常输入中文。
- 应用搜索改为独立原生输入框与独立控制器，不再复用旧 Material 搜索控件的事件、样式和清空
  逻辑。列表刷新不读写输入内容，中文组字期间不筛选；应用菜单关闭不再把焦点抢回菜单按钮。
- 除搜索与上述焦点修复外，各组件相对 2.1.0 只有版本号变化。

## 2.1.0 — 2026-10-29

- A/B 双端 WebUI 统一为 Material You 设计系统：共享色调、圆角、状态卡、底部导航/侧栏和
  表单组件，覆盖概览、应用、设置、连接页及多级弹窗；保留简体中文 / English 和浅色 / 深色，
  在 320–1024 px 宽度下自适应。
- A 应用页改为系统已安装应用列表，启动和手动刷新都会重新枚举，不再按 `target.txt` 过滤。
  每个应用进入独立策略页，可分别设置作用域、接管模式（全局默认 / StrongBox / TEE）、
  认证签名（自动 / 支付兼容 / 保持认证要求）和 CPU/设备属性。
- 作用域写入收敛为单应用开关：只有打开或关闭某个应用的“启用作用域”才改写 `target.txt`；
  列表不再提供批量写入、“显示全部”或已卸载应用清理。卸载后刷新只移除列表项，文件条目保留。
  全局作用域开启时忽略 `target.txt`，单应用作用域暂停，其他应用策略仍可用。
- A 新增每应用认证计数和最近一次 B 端延迟。只在证书链验证和密钥保存成功后计数，数据存入
  现有 `keymaster.db`；共享 UID 显示共享标记，失败调用和内部重试不计数。
- A 新增系统能力掩码弹窗，可独立关闭 StrongBox 与 device ID attestation capability，
  保留 `app_attest_key`；CPU/设备属性继续使用 exp6 的 Zygisk 视图，属性由 B 端提供，
  不提供手工改值。认证签名策略改为应用页内按应用保存，正数密钥 ID 兼容仍是独立全局开关。
- B 端 WebUI 重做；SPL/OS 状态读取使用只读查询，设置仍通过保存并应用；连接草稿、
  Token 掩码、语言和主题行为保留。
  B 端新增只读电池遥测，经现有认证 poll 上报；Server 在状态接口中增加
  `battery_percent`、`battery_charging`、`battery_updated_at_ms`。缺失、非法或过期数据显示
  未知，不补 0，也不沿用上一个读数。
- Server 对电池字段做独立校验并更新队列元数据，兼容旧 B 与旧 Server；B/Server 的协议、
  负值/越界/非法状态测试和 `server/source/README.md` 一并补齐。A/B 顶部刷新现在等待完整
  状态读取、显示忙碌状态，显式刷新在读取期间只排队一次，失败后可重试。
- 新增共享 UI 资源、设计预览与离线浏览器 smoke，覆盖 A/B 双语和浅深色、320–1024 px、
  应用列表/作用域、刷新、电池、能力弹窗；新增 WebUI bundle 补丁校验和发布包校验脚本。
- 升级时分别更新 A/B 模块与 Server；A 安装后重启以加载 Zygisk 新库。旧 `target.txt`
  条目会保留，可在应用页按应用重新开关。

## 2.0.1 — 2026-10-26

- A 端新增独立的“CPU 与设备属性”设置入口。可在二级应用列表中按名称或包名搜索、查看
  应用图标并自定义作用域，也可选择是否包含系统应用；此作用域独立于 KeyMint 应用列表和
  全局开关。
- 属性值由 B 端自动提供，A 端不再写死设备型号，也不提供手工改值入口。集成保留 exp6
  的 Zygisk 属性视图逻辑；配置默认关闭且作用域为空，缺失或无效配置会跳过应用。
- 调整应用图标加载与缺省图标显示，并统一弹窗的系统返回手势、浏览器返回和 Escape 行为。
- 从独立 `zygisk_maskcpu` 模块迁移时，先停用该模块，再启用 Ommega 内置功能；A 端安装
  更新后需重启 A 使 Zygisk 加载新库。

## 2.0.0 — 2026-10-23

- 认证签名策略按应用自动判断是否使用本地认证密钥；全局作用域只覆盖普通应用
  （每个用户 10000–19999 的应用 ID），工程模式需要匹配的 Binder SELinux 上下文和完整
  进程名。策略和全局作用域集中在设置页，应用页与概览不再重复显示。
- SOTER 的 ASK、AuthKey 与签名会话固定在同一后端。`local_only` 选择写入账本并在重载后
  保留，签名会话使用 A 端分配的不透明句柄；网络错误和远端认证错误不会改成本地签名。
- 有效 grant 把隔离进程纳入同一 Ommega 调用方作用域。委派绑定真实 UID、锚进程 PID、
  启动时间和开机标识，密钥访问仍按授权向量检查 `GET_INFO` / `USE`；撤销授权、进程退出
  或 UID 复用后失效。
- 其余：App AttestKey 只返回叶证书委派、认证密钥本地签发、GMS 认证窗口、
  system_server grant 探测、设置页多级弹窗与返回手势，以及 CI 缓存和工具链提速。

## 早期修复摘要

1.9.0 及更早版本的完整说明见 Releases。以下修复仍直接影响当前行为：

- **1.6.0**：镜像重放期间不再把 ommega 路由调用打成 `SYSTEM_ERROR`（改为有界等待，
  `poisoned` 仍 fail-closed）；EC 密钥生成恢复 AOSP 语义（EC + ENCRYPT 只警告不拒绝）；
  RSA 与 ECDSA 按密钥请求的摘要选择签名方案，修复非 SHA-256 RSA 密钥无法签名和摘要不匹配
  的静默错误签名。B 端 `spl-control.sh` 拒绝 SPL/OS 版本降级，A 端新增
  `event=local_sign` 诊断日志。
- **1.4.6**：检测器兼容改为全局正数 key id 开关，写入 `target-compat.toml` 的
  `[positive_key_id]`；默认关闭，只影响改动后新建的密钥。
- **1.4.5**：为进入软件 TA 的 HAL 入口补记“安全世界往返”耗时；`debug_logging` 开启后
  记录 `event=remote_attest_chain`。
- **1.4.2**：同一 operation 的重叠调用在注入器边界返回 `OPERATION_BUSY`，对齐 AOSP 并发
  契约。
- **1.4.1**：覆盖远程 profile、证明/签名/解密、P-256/P-384/P-521/X25519 协商，以及 A
  重启前后的认证密钥生命周期。
