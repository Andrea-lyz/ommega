# Ommega — Andrea-lyz Fork

[![中文](https://img.shields.io/badge/语言-中文-blue)](README.md)
[![English](https://img.shields.io/badge/Language-English-lightgrey)](README.en.md)
[![中文使用说明](https://img.shields.io/badge/使用说明-中文-blue)](ommega远程转发密钥说明.txt)
[![English guide](https://img.shields.io/badge/Guide-English-lightgrey)](ommega-remote-key-guide.en.txt)

> [!IMPORTANT]
> 本仓库是 [jiyin004-jpg/ommega](https://github.com/jiyin004-jpg/ommega) 的社区维护分支，
> 不是上游官方发行版或官方在线服务。当前源码版本为 **1.4.1**，模块作者信息为
> `jiyin004, Andrea-lyz`。

Ommega 是一套 A/B/Server 三端远程 KeyMint 系统：A 端拦截指定应用的 Android
Keystore 请求；满足远程生成条件的请求经 Server 调度到 B 端硬件 KeyMint，
返回证明、签名、解密和密钥协商结果。A 同时承担软件 TA、密钥存储和认证检查。
本 fork 在上游架构上重点补齐了远程身份一致性、原生 StrongBox、冷启动
生命周期、实时 WebUI 配置和 B 端软重启运维。

## 测试环境与支持边界

以下是 **2026-09-05** 的测试记录，不是兼容设备清单。固件版本由设备只读查询确认；
应用版本和验收结果来自此前同日的测试窗口，后续更新不自动继承这些结果。

| 项目 | A 端 | B 端 |
|---|---|---|
| 机型 | 一加 13，PJZ110 | 一加 11，PHB110 |
| Android / API / ABI | Android 16 / API 36 / arm64-v8a | Android 16 / API 36 / arm64-v8a |
| 固件构建号 | `PJZ110_16.0.10.501(CN01)` | `PHB110_16.0.2.400(CN01)` |
| ColorOS 属性 `ro.build.version.oplusrom` | `V16.1.0` | `V16.0.0` |
| 已验证硬件路径 | 原生 StrongBox + Android RKP 曾单独验证；当前远程验收关闭该路径 | 默认 TEE KeyMint；native relay |
| StrongBox 状态 | 有原生实现；启用 Mask 后功能查询为 false，不代表硬件消失 | 当前固件功能查询为 false，历史 profile 为 `strongbox=false`；未验证 B StrongBox |
| `app_attest_key` 功能查询 | true，Mask 保留此项 | true |

Mask 历史验证环境为 LSPosed 2.2.0 / libxposed API 102。A/B 模块在 root 环境运行，
上述结果不能推广为所有 root 管理器、ROM 或机型的兼容承诺。Server 为 Linux x86_64，
使用 physical 模式；1.4.1 CI 提供 musl 构件。

远程验收配置：A `remote=true`、`local_hw=false`、`disable_native_strongbox=true`、
`use_native_strongbox=false`，目标应用使用 GlobalDefault，并启用 Mask。
这验证的是 **A → Server → B TEE**，不是“双端 StrongBox 均通过”。
历史测试使用 Integrity Checker 2.2、Paytm 10.83.12、GMS 26.34.31、Play 商店 52.9.21-34。
三绿曾通过；Paytm 曾进入手机号输入页，也曾间歇出现 `00000`。重启后改善不构成
“已证实误报”或“永久修复”的结论，更不保证支付、绑卡或所有风控场景可用。

A 安装器的最低门槛是 API 29，Mask 的 `minSdk` 也是 29；这只代表安装门槛，
不代表 Android 10 起全部可用。实际运行还依赖 keystore2、KeyMint AIDL、注入器与固件
接口匹配。CI 发布 arm64-v8a 模块；其他系统版本、架构及 B StrongBox 均需独立验证。

## StrongBoxCapabilityMask：何时使用及风险

这是可选的 LSPosed/libxposed APK，不是 A/B ZIP，也无需安装到 Server。
通常只在 **A 端确实有 StrongBox，且希望应用按“未声明 StrongBox”选择路径**时使用。
没有该功能声明的设备通常没有必要安装；不能将它作为 Paytm 告警的通用修复。

模块仅加载到 `system_server`，在启动阶段 hook
`com.android.server.SystemConfig.getAvailableFeatures()`，从返回的功能表中删除
`android.hardware.strongbox_keystore`，保留 `android.hardware.keystore.app_attest_key`。
它是全局操作，影响包括未勾选 Ommega 的应用在内的所有使用该功能表的调用者；没有按包名开关。

适配前提是 ROM 沿用 AOSP 的上述方法和可修改的 Map，以及功能发布/缓存路径，且没有
绕过此表的厂商实现或硬编码结果。Android 16 的进程功能缓存也是选择该启动 hook 的原因，
见 [AOSP ActivityManagerService](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android16-release/services/core/java/com/android/server/am/ActivityManagerService.java)。
“遵循 AOSP”是实现前提，不是 AOSP 认证或对所有 AOSP ROM 的支持保证；目前仅上述 A 固件有实机证据。

使用步骤：

1. 在 A 安装 `StrongBoxCapabilityMask-1.4.1-debug-signed.apk`。
2. 在兼容 libxposed API 102 的 LSPosed 实现中启用，保持静态作用域仅 `system`（系统框架），不要勾选应用。
3. 在允许正常重启的 A 上重启并解锁，让系统功能表及进程缓存重新建立；仅划掉应用不能启用/撤销此 hook。
4. 检查 `pm has-feature android.hardware.strongbox_keystore` 为 false、原有 `app_attest_key` 仍为 true，
   并检查 Mask 日志的 `event=hook_registered` / `event=feature_removed`；必要时用正常测试 App 核对应用进程查询。
5. 撤销时在 LSPosed 禁用模块，再重启 A。APK 使用调试签名，升级前核对签名；签名不同不能直接覆盖安装。

**不要将上述重启步骤套用到禁止硬重启的 B。本测试 B 没有安装 Mask 的必要。**
模块不修改系统 XML、不停止 StrongBox HAL、不删除或迁移已有密钥、不改变真实证明中的
Bootloader 状态，也不阻止应用直接请求 StrongBox。显式请求仍由原系统或 Ommega 路由处理。
原生硬件证明仍可能反映解锁状态；功能查询 false 不能证明设备已隐藏解锁或通过完整性校验。

后果可能包括应用降级到 TEE/其他实现、拒绝运行、依赖 StrongBox 的功能或商店分发行为变化，
以及声明与真实接口不一致被检测。已有密钥不会因此被转换；应用是否继续使用它们由应用决定。
系统进程 hook 还有兼容性、崩溃或启动异常风险。**本项目及 Mask 按现状提供，不保证适用性、
安全性或检测结果；使用者自行承担风险。对数据丢失、设备异常、账户限制、业务或财产损失概不负责
（以适用法律允许的范围为限）。不接受这些风险请勿使用。**

## 与上游的主要区别

### 1. A/B KeyMint 身份严格对齐

- B 端新增 `profile` 任务，读取真实默认 KeyMint HAL 的 Stable AIDL 版本、接口哈希、
  `getHardwareInfo()`、安全级别和 StrongBox 可用性。
- A 端在软件 TA 启动前取得并冻结该 profile；后续证明结果必须携带同一身份，防止
  AIDL 版本、硬件版本或安全级别混用。
- 冷启动网络暂不可用时会等待并重试远程 profile，不再提前冻结错误的本地身份。
- Server 对 B 端 profile 做结构校验；允许真实厂商 HAL 返回空
  `keymint_author`，但拒绝字段缺失、类型错误和仅含空白字符的值。
- A 校验远程 profile、叶证书的 challenge、应用 ID、版本和安全级别；校验错误直接失败。
  当前尚未完成完整证书链的可信根、逐级签名及撤销校验。
- `fallback_local` 只控制允许回退的远程不可用路径；它不会让身份校验错误变成成功。

### 2. 原生 StrongBox 与 Android RKP

- A 端加入 `use_native_strongbox`，可直接复用 A 设备自身的原生 StrongBox KeyMint
  HAL，而 TEE 请求仍走远程 B 端。
- 原生 StrongBox 证明密钥通过 Android RKP
  `IRemotelyProvisionedComponent/strongbox` 获取，不把
  `ATTESTATION_KEYS_NOT_PROVISIONED` 误判为设备没有 StrongBox。
- 原生 HAL 实际返回的安全级别会被校验，避免把 TEE 错报成 StrongBox。

`use_native_strongbox` 是 KeyMint 后端选择，修改后只需重启 Ommega 的 keymint
子进程，不需要重启整台设备：

```toml
# /data/misc/keystore/ommega/config.toml
[main]
use_native_strongbox = true
```

```sh
touch /data/adb/ommega/restart.keymint
```

### 3. 面向应用的三档安全级别策略

`target.txt` 现在只保存普通包名；每个已勾选应用可在 A 端 WebUI 中选择：

| 模式 | 行为 |
|---|---|
| 全局默认 | 遵循设备能力和“禁用原生 StrongBox”总开关 |
| StrongBox | 显式选择 StrongBox 路径，并覆盖全局禁用开关 |
| TEE | 将该应用的安全级别请求改写为 TEE，包括显式 StrongBox 请求 |

显式策略保存在：

```text
/data/misc/keystore/ommega/target-security.toml
```

远程配置页另有默认关闭的“禁用原生 StrongBox”开关。开启后，它只影响
`target.txt` 中处于“全局默认”的应用；显式 StrongBox/TEE 选择优先。

旧 `strongbox_unavailable_packages` 仅保留一次性迁移兼容：当
`target-security.toml` 尚不存在时可转成 TEE 策略，之后不再作为正式配置写回。
`target.txt` 的 `!`、`?` 后缀也不再是正式配置格式。

独立的 StrongBoxCapabilityMask 模块从系统功能表隐藏
`android.hardware.strongbox_keystore`，保留 `android.hardware.keystore.app_attest_key`。
它不停止 StrongBox HAL，也不替代上述请求路由。应用忽略功能表仍显式请求 StrongBox 时，
全局默认加禁用开关会返回 `HARDWARE_TYPE_UNAVAILABLE`；显式 TEE 则改写请求。

### 4. A 端配置即时生效

- `target.txt`、`target-security.toml` 和“禁用原生 StrongBox”由注入器实时读取。
- 远程地址、设备 ID、Token、TLS、日志和普通路由配置保存后即时生效。
- Verified Boot Hash、Verified Boot Key、System/Vendor/Boot SPL 会同步到
  `config.toml` 的 `[trust]` 配置。
- SPL 更新直接作用于运行中的 TA；Hash/Key 改动只回收 Ommega keymint 子进程并等待
  RPC 恢复，不要求设备重启。
- WebUI 写配置时使用 UTF-8 Base64 传递内容，避免用户输入提前结束 shell heredoc。
- 覆盖安装模块不会清空已经保存的 WebUI 信任参数。

### 5. keystore2 冷启动与生物认证生命周期

- Binder hook 会先于远程 RPC 就绪安装，避免漏掉开机早期的授权和维护事件。
- RPC 在 `ommega-rpc-warmup` 后台线程中初始化，不阻塞 hook 安装。
- `OnDeviceUnlocked`、auth token 和用户维护事件会进入内存 mirror 队列，RPC 恢复后
  按顺序重放。
- 修复了用户 0 super key 未初始化时，生物认证绑定密钥创建/读取失败的问题。

本地旧密钥读取同时支持 P-521 裸标量和旧 SEC1 DER 编码；带曲线标识或公钥的
DER 会校验其与 P-521 私钥的一致性。旧 PBKDF2 兼容 KDF 保留历史 64 字节
extract 输出，仅用于旧数据读取；新数据仍使用标准 HKDF，P-521 写入格式不变。

### 6. B 端 SPL WebUI 与服务级运维

- B 端 WebUI 可分别设置 System、Boot、Vendor SPL，保存后立即应用。
- 属性变化时只重载原生 KeyMint 服务和 `keystore2`，模块本身不请求硬重启。
- 配置保存在 `/data/adb/ommega/spl.conf`；留空会选择首次记录的基线值。
  已在较高 SPL 下升级的 keyblob 可能要求保留较高值，首次基线不能直接当作恢复值。
- B 端故意不提供 `post-fs-data.sh`，只以 `service.sh` 作为生命周期入口；后续
  KernelSU 加载或软重启时由 `service.sh` 重新应用 SPL，再启动 relay。
- `service.sh` 会按精确模块路径清理旧 relay，避免旧进程继续占用 `relay.lock`。

> 修改属性不等于硬件 TEE 一定接受了新 Boot SPL。最终值必须以新生成的硬件证明
> 证书为准。
> SPL 涉及凭据加密和 keyblob 升级。必须先确认用户已解锁、没有在途 TEE 调用及可行的恢复路径；
> 不要将 SPL 调整与重启组合操作。禁止硬重启的 B 必须始终遵守该限制，软重启也不是无风险恢复手段。

### 7. B 端 relay 与 Server 稳定性

- B 端 WebUI 可编辑 `relay.conf` 的连接参数和四项日志设置；连接参数支持热加载，
  日志设置在 relay 下次启动时生效。保存 Relay 配置不会重启进程或应用 SPL。
- B 端持久化密钥在 `begin` 返回 `KEY_REQUIRES_UPGRADE` 时，会在原 HAL 上
  调用 `upgradeKey`，原子保存并同步文件/目录后重试一次。alias、证书链、算法和
  HAL 归属保持不变，不生成替代密钥；保存失败会保留内存中的新 blob 并阻止操作，
  后续请求先重试保存。该内存保留不能替代失败写入时的断电保护。
- 长轮询与 TEE 工作线程分离；任务执行期间仍可继续领取任务，降低批量检测延迟。
- 修复任务完成早于 waiter 注册、长轮询超时、重复 request ID 和掉线任务回收等队列
  竞态。
- Server 校验结果结构、非空证明链字段、profile 字段及任务归属；
  非空链检查不是证书签名或可信根验证，A 还会执行远程身份与叶证明字段检查。
- Server 管理后台保留独立的“StrongBox 强健模式”：默认关闭；开启后，仅在 B 端
  StrongBox 返回能力类错误时，才在同一 B 设备上重试 TEE。

远程失败响应保留原有 `error` 文本，并可附带整数 `keymint_error_code`。
B 端只从真实 HAL 的 service-specific 失败中提取该字段；Server 保持 HTTP 500
并转发它，A 端将已识别的负值还原为 KeyMint 错误。旧版失败响应、未知代码、HTTP
鉴权/限流错误仍按通用失败处理，不通过解析错误文本推测硬件错误，也不触发新的
本地回退。成功响应、证明生成和 StrongBox 能力声明不受该协议变更影响。

Server 强健模式与 A 端控制互不等价：

| 控制项 | 生效位置 | 用途 |
|---|---|---|
| `use_native_strongbox` | A 端 KeyMint 后端 | 使用 A 设备真实 StrongBox/RKP |
| 禁用原生 StrongBox | A 端应用路由 | 让全局默认目标应用按无 StrongBox 设备处理 |
| 每应用 StrongBox/TEE | A 端应用路由 | 覆盖全局默认行为 |
| Server StrongBox 强健模式 | B 端任务失败之后 | 能力类错误时重试 TEE；默认关闭，Server 重启后复位 |

### 8. 自动构建与版本管理

GitHub Actions 并行构建 A 模块、B 模块、Linux musl Server 和 Android APK，
保留 A/B workspace 检查、Server 测试、B ELF 检查，并运行 WebUI、版本同步和
StrongBoxCapabilityMask 单元测试。Rust 固定到已验证的 `nightly-2026-09-01`，
A/B/Server 提交 Cargo.lock，按组件缓存 Rust 构建并复用 Gradle 缓存。

当前统一版本为 **1.4.1**。CI 默认使用源码版本，不自动递增或提交版本；
手动 `release_version` 仅覆盖本次构建。当前工作流只忽略纯 `*.md` 修改；TXT 修改仍会触发构建。
全部构建成功后生成 `ommega-<版本>` artifact，包含 A/B 模块 ZIP、Server、
B-app 和 StrongBoxCapabilityMask APK，以及 SHA256SUMS 和源提交信息。
两个 APK 使用 Android 默认 debug 签名；CI 缓存该调试签名身份供后续构建复用，
它不是正式发布签名，私钥不随源码或构建产物分发。
缓存失效可能导致后续签名身份变化，升级前仍需核对证书。

## 系统组成

| 端 | 角色 | 形态 |
|---|---|---|
| A 端（`a-side`） | 拦截目标应用 Keystore；远程 TEE；可选 A 端原生 StrongBox | root 模块，CI 提供 arm64-v8a；安装器检查 KernelSU/Magisk 环境 |
| B 端（`b-side`） | 调用真实硬件 KeyMint，处理 profile/attest/sign/decrypt/agree | root 模块，CI 提供 arm64-v8a；管理器兼容性需单独确认 |
| B 端 App（`b-app`） | 可选的独立 B 端客户端与连接配置界面 | Android APK |
| StrongBoxCapabilityMask | 全局隐藏 StrongBox capability；仅加载到 system_server | libxposed API 102 APK |
| Server（`server`） | 鉴权、任务队列、A/B 调度、设备与后台管理 | Linux x86_64 musl 二进制 |

`remote-only` 表示符合远程生成条件时不允许隐式回退；它不表示所有密钥或方法都在 B 执行。
软件 TA 仅在有 attestation challenge、未提供调用方 attestation key 且远程已启用时
进入远程生成；其余生成仍可在 A 完成。远程密钥的签名、解密及协商由 B 执行，
A 负责本地存储和应用认证检查。B 的请求转换包含 `NO_AUTH_REQUIRED` 和
`ATTEST_KEY` 转 `SIGN`，因此不能将 A 的用户认证描述为 B 硬件直接验证了 A 的认证令牌。

B-app 当前只分发 `attest/sign/decrypt`，未实现 native B 所需的 `profile/agree`，
并通过 B-app 自己的 AndroidKeyStore 身份生成密钥；它目前不能替代 native B 完成同一套验收。

## 快速部署

本 fork 不在 README 中分发公共 Server Token。请自行部署 Server 并设置自定义凭据；
启用 MySQL 后可从管理后台为 A/B 分配独立的角色 Token。

### B 端

安装 B 模块后，可在模块 WebUI 的「Relay 配置」中填写以下项目，
也可直接编辑 `/data/adb/ommega/relay.conf`：

```ini
OMMEGA_RELAY_SERVER=https://<server>:8443
OMMEGA_RELAY_DEVICE_ID=<b-device-id>
OMMEGA_RELAY_MACHINE_ID=
OMMEGA_RELAY_TOKEN=<b-token-or-static-token>
OMMEGA_RELAY_LOG_ENABLED=true
OMMEGA_RELAY_LOG_LEVEL=debug
OMMEGA_RELAY_LOGCAT_ENABLED=true
OMMEGA_RELAY_LOGCAT_LEVEL=info
```

连接配置由 relay 热加载，日志配置在下次启动生效；需要主动触发重新读取时执行：

```sh
touch /data/adb/ommega/restart.all
```

该标记只请求重读配置，不重启 relay 或设备，也不重新初始化日志。

### A 端

安装 A 模块，在 WebUI 或 `/data/adb/ommega/config` 中填写：

```yaml
url: https://<server>:8443
device_id: <b-device-id>
token: <a-token-or-static-token>
remote: true
local_hw: false
tls_insecure: true
debug_logging: false
disable_native_strongbox: false
```

上面是配置格式示例，不是测试配置的完整复刻。`tls_insecure: true` 会跳过 A 的证书验证；
部署可信证书后应按实际环境配置。B 当前 HTTP 客户端接受无效证书，不能仅凭 HTTPS 地址
就宣称已认证 Server 身份；公网部署需自行保护传输与凭据。

在 WebUI 勾选需要接管的应用，并按需选择“全局默认 / StrongBox / TEE”。未加入
`target.txt` 的应用继续使用系统原始 Keystore 路径。

### Server

在二进制工作目录创建 `.env`，最小示例：

```ini
RELAY_BIND=0.0.0.0
RELAY_HTTP_PORT=10886
RELAY_HTTPS_PORT=8443
RELAY_USE_TLS=true
RELAY_TLS_CERTFILE=data/tls/server.crt
RELAY_TLS_KEYFILE=data/tls/server.key
RELAY_TOKEN=<random-static-token>
RELAY_ADMIN_USER=admin
RELAY_ADMIN_PASSWORD=<strong-password>
RELAY_ATTEST_SOURCE=physical
# RELAY_MYSQL_URL=mysql://user:password@127.0.0.1:3306/ommega
# RELAY_SECRET_KEY=<random-secret>
```

- HTTP 默认端口：`10886`
- HTTPS 默认端口：`8443`；证书不存在时自动回退为仅 HTTP
- 健康检查：`/api/health/`
- 公开设备状态：`/status/`
- 管理后台：`/jiyin004/`
- 登录页：`/login/`

Linux 构件 `relay_rs-linux-x86_64-musl` 为静态链接二进制。更新时应先上传临时
文件、核对 SHA-256、保留旧版备份，再停服原子替换并执行健康检查。

## 配置文件与生效方式

| 文件/开关 | 作用 | 生效方式 |
|---|---|---|
| A `target.txt` | 选择接管应用 | 注入器实时读取 |
| A `target-security.toml` | 每应用安全级别 | 注入器实时读取 |
| A `config` | 远程连接与全局 StrongBox 策略 | 保存后实时读取 |
| A `webui-props.sh` / `config.toml [trust]` | Hash、Key、SPL | 实时更新；必要时只重启 Ommega keymint |
| A `config.toml [main].use_native_strongbox` | StrongBox 后端 | 重启 Ommega keymint，不重启设备 |
| B `relay.conf` 连接参数 | Server、设备 ID、机器 ID、Token | WebUI 保存后由 relay 热加载 |
| B `relay.conf` 日志参数 | 文件日志与 logcat 开关、级别 | relay 下次启动时生效 |
| B `spl.conf` | System/Boot/Vendor SPL | 保存即应用；`service.sh` 再应用 |

## 构建

以下每段从仓库根目录独立执行，需先配置相应依赖：

```sh
# A 端模块
cd a-side/source
python build.py
```
```sh
# B 端模块
cd b-side/source
python build.py
```
```sh
# Server
cd server/source
cargo build --locked --release
```
```sh
# B 端 App
cd b-app/source
./gradlew assembleRelease
```
```sh
# StrongBoxCapabilityMask（从仓库根目录进入）
cd StrongBoxCapabilityMask
./gradlew :app:testDebugUnitTest :app:assembleRelease
```

完整发行构建建议使用仓库的
[Build workflow](https://github.com/Andrea-lyz/ommega/actions/workflows/build.yml)。

## 验证范围与限制

1.4.1 源码对应的前六批工作已覆盖远程 profile、证明/签名/解密、
P-256/P-384/P-521/X25519 协商，以及 A 重启前后的认证密钥生命周期。
NoPadding、SHA-224、NONE、OAEP MGF 修复有定向验证，不代表所有算法参数组合都支持。
第六批 A Android workspace 测试为 516 项通过，另有正常应用远程链路验证。
历史 B SPL 和 A 原生 StrongBox/RKP 测试与最终远程 TEE 配置是不同测试窗口。
CI 全绿、Server 部署成功与最终安装包的应用验收是不同层次；不得互相替代。
Wrapped import 未纳入支持验收，完整证书信任链验证及其他未完成工作仍有边界。
不保证所有设备、固件、应用、算法或网络环境可用。

## 上游、署名与许可证

- 上游仓库：[jiyin004-jpg/ommega](https://github.com/jiyin004-jpg/ommega)
- 本 fork：[Andrea-lyz/ommega](https://github.com/Andrea-lyz/ommega)
- 原作者：jiyin004
- Fork 维护与 1.4.x 改动：Andrea-lyz

继续感谢本项目所参考的开源实现：

| 项目 | 作者 | GitHub |
|---|---|---|
| Tricky Store | 5ec1cff | [5ec1cff/TrickyStore](https://github.com/5ec1cff/TrickyStore) |
| Tricky Addon | KOWX712 | [KOWX712/Tricky-Addon-Update-Target-List](https://github.com/KOWX712/Tricky-Addon-Update-Target-List) |
| OhMyKeymint | James Clef（qwq233） | [qwq233/OhMyKeymint](https://github.com/qwq233/OhMyKeymint) |
| KeyAttestation | vvb2060 | [vvb2060/KeyAttestation](https://github.com/vvb2060/KeyAttestation) |
| TEESimulator-RS | Enginex0 | [Enginex0/TEESimulator-RS](https://github.com/Enginex0/TEESimulator-RS) |

项目继续遵循仓库中现有的 AGPL-3.0-or-later、Apache-2.0 等对应源码许可证；本 README
不改变任何第三方代码的原许可证或版权归属。
