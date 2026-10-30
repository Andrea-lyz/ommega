# 更新日志

本文件从 2.0.0 起逐版本记录 A / B / Server 与可选 APK 的更新要点；更早版本和完整发布说明见
[GitHub Releases](https://github.com/Andrea-lyz/ommega/releases)。文末保留仍影响当前行为的
早期修复摘要。

版本号在 A / B / Server / B-app / StrongBoxCapabilityMask 之间统一。除特别说明外，一次
发布中只更新正常发布的模块 ZIP 不会替你更新 Server，也不会安装可选 APK。

## 2.1.4 — 2026-10-29

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
