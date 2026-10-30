# Ommega

[![版本](https://img.shields.io/badge/版本-2.1.4-blue)](https://github.com/Andrea-lyz/ommega/releases)
[![许可证](https://img.shields.io/badge/许可证-AGPL--3.0--or--later-green)](LICENSE)
[![中文](https://img.shields.io/badge/语言-中文-blue)](README.md)
[![English](https://img.shields.io/badge/Language-English-lightgrey)](README.en.md)
[![使用说明](https://img.shields.io/badge/使用说明-中文-blue)](ommega远程转发密钥说明.txt)
[![English guide](https://img.shields.io/badge/Guide-English-lightgrey)](ommega-remote-key-guide.en.txt)

> 把 A 端应用的 Android Keystore 请求，经自建 Server 调度到 B 端设备的真实 KeyMint/TEE
> 上执行：证明链、签名、解密和密钥协商都由 B 端硬件完成，结果再原路返回应用。

> [!IMPORTANT]
> 本仓库是 [jiyin004-jpg/ommega](https://github.com/jiyin004-jpg/ommega) 的社区维护分支，
> 不是上游官方发行版，也不提供公共在线服务或公共 Token。当前版本 **2.1.4**，模块作者为
> `jiyin004, Andrea-lyz`。维护目标是 **1 台 A + 1 台 B** 的专用部署，不以一个 B 服务
> 多个 A 为适配目标。

> [!WARNING]
> **B 端请使用不存放个人数据的备用设备，并在设备之外保留可用备份。**
> B 端会改写 System/Boot/Vendor 安全补丁级别（SPL）并重载原生 KeyMint，涉及凭据加密与
> keyblob 单向升级。若 root 在重启后丢失，SPL 覆写不会随之恢复，可能出现**密码正确却
> 无法解锁、凭据加密数据不可用**，且不保证无损恢复。不要用主力机当 B，也不要把 root
> 的可用性当成必然前提。详见[安全与风险](#安全与风险)。

## 这是什么

Ommega 是一套 A / Server / B 三端协作的远程 KeyMint 系统。

- **A 端**是 root 模块。它接管目标应用的 Keystore/KeyMint 调用，提供软件 TA、本地密钥
  存储和认证检查；远程模式下把证明、签名、解密和协商请求转发给 Server。
- **Server** 是自建的 Linux 服务。负责 Token 鉴权、任务队列、A/B 配对调度、设备在线
  状态和管理后台。
- **B 端**是 root 模块。native relay 轮询任务，直接调用本机真实 KeyMint HAL（TEE 或
  StrongBox），返回硬件铸造的证书链与操作结果。

本 fork 维护远程身份一致性、原生 StrongBox/RKP、冷启动与生物认证生命周期、WebUI 配置
以及 B 端服务级运维。它不面向免 root 设备、非 Android 平台，也不承诺所有 ROM、机型或
算法组合可用。

| 项目 | 说明 |
|---|---|
| 当前版本 | **2.1.4**（A / B / Server / 两个 APK 统一版本号） |
| 部署形态 | 1 台 A + 1 台 B + 1 个自建 Server |
| A / B | Android 12 及以上，arm64-v8a，KernelSU / APatch / Magisk |
| Server | Linux x86_64（musl 静态二进制） |
| 许可证 | A/B 为 AGPL-3.0-or-later；Server 声明 MIT；部分源码 Apache-2.0 |

## 工作原理

```mermaid
flowchart LR
    App["目标应用"] -->|"Keystore / KeyMint Binder"| A["A 端模块<br/>injector · 软件 TA · relay client"]
    A -->|"HTTPS 任务 + Token"| S["Server<br/>鉴权 · 队列 · 调度"]
    S -->|"poll / result"| B["B 端模块<br/>native relay"]
    B -->|"Stable AIDL"| HAL["真实 KeyMint HAL<br/>TEE / StrongBox"]
    B -.->|"结果"| S
    S -.->|"结果"| A
    A -.->|"证书链 / 签名 / 解密"| App
```

一次远程请求的典型路径：

1. 应用调用 Keystore，A 端注入器按作用域判断是否接管；未命中的应用继续走系统原始路径。
2. A 端软件 TA 生成或选择密钥。满足远程条件的建钥请求携带应用 ID（tag 709）发给 Server。
3. Server 鉴权后把任务放进队列，B 端 relay 轮询取走任务。
4. B 端调用真实 HAL 完成 `profile` / `attest` / `sign` / `decrypt` / `agree` / `soter`，
   把结果写回 Server。
5. A 端校验远程 profile 与叶证书字段，保存密钥并返回应用。远程密钥的后续签名、解密和
   协商同样由 B 执行，A 只做本地存储与认证检查。

`remote-only` 表示符合远程生成条件时**不允许隐式回退**，并不表示所有操作都在 B 执行：

- 远程生成需要 `remote: true`、存在 attestation challenge，且未指定签发密钥或指定了
  远程 App AttestKey。指定本地签发密钥时仍在 A 完成。
- B 保留请求中的认证授权列表（用户 SID、认证类型、超时、`UNLOCKED_DEVICE_REQUIRED`
  509），但 B 无法验证 A 的用户认证令牌；需要认证的远程硬件操作仍可能被 B 拒绝。
- 未指定签发密钥时，A 原样返回 B 的完整证书链，不为认证密钥额外签发子证书。
- StrongBox 请求不会在任何一层被静默降级为 TEE：A 以 `HARDWARE_TYPE_UNAVAILABLE`
  拒绝远程降级标记，Server 只转发 B 的能力错误。

## 组件

| 目录 | 角色 | 形态 |
|---|---|---|
| [`a-side`](a-side/source/README.md) | 拦截目标应用 Keystore；软件 TA；远程 TEE；可选 A 端原生 StrongBox | root 模块，含 WebUI，CI 提供 arm64-v8a |
| [`b-side`](b-side/source/README.md) | 调用真实硬件 KeyMint，处理 `profile/attest/sign/decrypt/agree/soter` | root 模块，含 WebUI 与 SPL 控制 |
| [`server`](server/source/README.md) | 鉴权、任务队列、A/B 调度、设备与后台管理 | Linux x86_64 musl 二进制 |
| [`b-app`](b-app/source) | 可选的独立 B 端客户端与连接配置界面 | Android APK，只实现 `attest/sign/decrypt` |
| [`StrongBoxCapabilityMask`](StrongBoxCapabilityMask/README.md) | 全局隐藏 StrongBox 与设备 ID 证明 capability | libxposed API 102 APK，仅加载到 `system_server` |
| [`ui`](ui/README.md) | A/B WebUI 共享样式、设计预览与离线 smoke | 开发资源，随模块打包 |

**B-app 不能替代 native B**：它没有实现 `profile` / `agree`，并使用自己的
AndroidKeyStore 身份生成密钥，无法完成同一套验收。

## 下载

[Releases](https://github.com/Andrea-lyz/ommega/releases) ·
[发布 CI](https://github.com/Andrea-lyz/ommega/actions/workflows/build.yml)

| 发布资产 | 用途 |
|---|---|
| `ommega-a-release-arm64-v8a-2.1.4-<commit>.zip` | A 端 root 模块，含 WebUI |
| `ommegaclient-b-release-arm64-v8a-2.1.4-<commit>.zip` | B 端原生 relay 模块，含 WebUI 与 SPL 控制 |
| `relay_rs-linux-x86_64-musl` | 自建 Linux Server，单独更新 |
| `Ommega-B-app-2.1.4-debug-signed.apk` | 可选 B 客户端，不是 A 端 App，也不等同 native B 模块 |
| `StrongBoxCapabilityMask-2.1.4-debug-signed.apk` | 可选 A 端 LSPosed 模块 |
| `SHA256SUMS` / `build-info.json` | 校验和、产品版本、源提交与签名说明 |

GitHub 自动生成的 Source code 压缩包不是可安装模块。两个 APK 均为 debug 签名，覆盖安装
前应核对证书；更新 A/B ZIP 不会替你更新 Server，也不会安装可选 APK。

### 2.1.4 更新要点

- B 端 WebUI 的 Server connection 卡片不再依赖 WebView 里的 curl/wget：有 `curl` 时读取
  Server 的 `/api/status/`，没有或不可用时读取 relay 日志最后一条 `b/poll status=`，
  显示 200/204、429、401/403、5xx 或 stale。
- 修复原生 Android（没有 `/system/bin/curl`）上误报 “Invalid server response” 的问题；
  relay 实际 poll 204 时显示 “Server connected” 和最后 poll 时间。
- 本次只改 B 端 WebUI；B relay 代码、A/Server、`service.sh`、TEE/KeyMint 未改。
- 只需更新 B 模块即可获得该修复；其他组件相对 2.1.3 只有版本号变化。

逐版本说明见 [CHANGELOG.md](CHANGELOG.md)。

## 快速部署

准备：

- 一台可用的 Linux x86_64 Server（或自行交叉编译 `relay_rs`）。
- 两台 Android 12+、arm64-v8a 的 root 设备，建议都使用受支持的 KernelSU / APatch /
  Magisk 环境。
- 自建 Token 与（建议）可信 TLS 证书。仓库不提供公共 Server 或公共 Token。
- **B 端必须是可丢弃数据的备用机**，并已完成设备外备份。

### 1. 部署 Server

把 musl 二进制放到工作目录，创建 `.env` 后启动：

```sh
mkdir -p /opt/ommega && cd /opt/ommega
cp /path/to/relay_rs-linux-x86_64-musl ./relay_rs
chmod +x relay_rs
```

```ini
# /opt/ommega/.env
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

```sh
./relay_rs
```

生产环境请用 systemd 或同类工具托管，并确保工作目录就是 `.env` 与 `data/` 所在目录。
默认端口与入口：

| 路径 | 用途 |
|---|---|
| `10886` | HTTP（默认） |
| `8443` | HTTPS；证书缺失时自动回退为仅 HTTP |
| `/api/health/` | 健康检查 |
| `/status/` | 公开设备状态（含在线 B 与电量遥测） |
| `/jiyin004/` | 管理后台 |
| `/login/` | 登录页 |

启用 MySQL 后可在管理后台为 A/B 分配独立角色 Token。更新 `relay_rs` 时应先上传临时
文件、核对 SHA-256、保留旧版备份，再停服务原子替换并做健康检查；不要覆盖正在运行的
可执行文件。

### 2. 安装 B 端

安装 `ommegaclient-b-*.zip` 后，在模块 WebUI 的「连接」页填写，或直接编辑
`/data/adb/ommega/relay.conf`：

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

- `SERVER`、`DEVICE_ID`、`TOKEN` 为必填；`MACHINE_ID` 可选。
- 连接参数由 relay 热加载；日志参数在 relay 下次启动时生效。
- 需要主动触发重读时执行 `touch /data/adb/ommega/restart.all`。该标记只请求重读配置，
  不重启 relay 或设备，也不会重新初始化日志。
- SPL / OS 版本在 WebUI「设置 → 设备属性」中修改并应用，写入文件为
  `/data/adb/ommega/spl.conf`。日期与 OS 版本只升不降；**升级或回滚模块时必须保留
  `spl-applied.conf`**。
- **禁止硬重启 B。** 需要重载时只使用已批准的 `service.sh` 服务级流程。

SPL 的完整规则、下限计算与风险见 [B 端说明](b-side/source/README.md)。

### 3. 安装 A 端

安装 `ommega-a-*.zip`，在 WebUI「设置 → 远程配置」中填写。对应文件为
`/data/misc/keystore/ommega/config`，WebUI 通过 `/data/adb/ommega/ommegadata/config`
访问同一份配置：

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

- `device_id` 必须与 B 的 `OMMEGA_RELAY_DEVICE_ID` 一致；`token` 使用 Server 上为该角色
  分配的 Token 或静态 Token。
- `tls_insecure: true` 会跳过 A 的证书验证。部署可信证书后应按实际环境收紧；B 当前接受
  自签证书，**仅凭 HTTPS 地址不能证明已认证 Server 身份**。
- 在「应用」页按应用打开作用域，并按需选择接管模式（全局默认 / StrongBox / TEE）。
  目标范围是 `target.txt` 与 `injector.toml` 中 `scoop` 的并集；两者都未命中的应用继续
  使用系统原始 Keystore 路径。
- 覆盖安装模块不会清空已保存的 WebUI 信任参数（Hash、Boot Key、SPL）。

### 4. 联调检查

1. B 端 WebUI「概览」应显示 relay 运行、`Server connected` 与最近 poll 时间。
2. Server `/status/` 应出现该 B 的在线记录；管理后台可查看任务与设备状态。
3. A 端「概览」显示最近一次中继结果；首次可先用测试应用建钥、签名，再切到生产应用。
4. A/B 的刷新按钮只重读状态文件，**不会发起硬件证明**；最近结果有时间戳，不能当作
   实时连通性探测。B 进程存在也不等于 Server 已连接。

## 配置一览

| 文件 / 开关 | 作用 | 生效方式 |
|---|---|---|
| A `target.txt` | 选择接管应用 | 注入器实时读取 |
| A `target-security.toml` | 每应用安全级别（全局默认 / StrongBox / TEE） | 注入器实时读取 |
| A `target-compat.toml` | 全局正数 key id 兼容开关 | 热加载；只影响新建密钥 |
| A `config`（`/data/misc/keystore/ommega/`） | 远程连接与全局 StrongBox 策略 | 保存后读取；启动时持久化已启用的远程配置 |
| A `soterta/remote.conf`（`/data/adb/ommega/`） | 独立 SOTER 中继参数 | 与 KeyMint 配置分别维护 |
| A `webui-props.sh` / `config.toml [trust]` | Verified Boot Hash、Key、SPL | 实时更新；必要时只重启 Ommega keymint |
| A `config.toml [main].use_native_strongbox` | StrongBox 后端选择 | `touch /data/adb/ommega/restart.keymint` |
| B `relay.conf` 连接参数 | Server、设备 ID、机器 ID、Token | WebUI 保存后由 relay 热加载 |
| B `relay.conf` 日志参数 | 文件日志与 logcat 开关、级别 | relay 下次启动时生效 |
| B `spl.conf` | System/Boot/Vendor SPL、OS 版本 | 保存即应用；`service.sh` 再应用 |
| B `spl-applied.conf` | 已生效目标与降级下限依据 | 脚本维护，升级/回滚时保留 |

## WebUI

A/B WebUI 只提供简体中文与 English，可跟随系统语言，并切换浅色 / 深色主题；设备端使用
包内本地资源，无需在线生成 UI。`ui/` 中的 OpenUI 预览仅用于开发。

| 端 | 页面 | 主要内容 |
|---|---|---|
| A | 概览 | 最近中继结果、KeyMint 进程、SOTER 软件 TA 与账本状态 |
| A | 应用 | 系统已安装应用列表；单应用作用域、接管模式、认证签名、CPU/设备属性 |
| A | 设置 | 远程配置、SOTER、自动化、证明参数、系统能力掩码、语言、主题、帮助与关于 |
| B | 概览 | relay 进程、连接证据、当前系统属性与电量 |
| B | 连接 | Server、设备 ID、Token 与日志配置 |
| B | 设置 | 设备属性（SPL / OS）、语言与主题 |

顶部两个 TXT 使用说明保留较早的操作示例；配置路径、界面入口与 SPL 规则以本 README、
[CHANGELOG.md](CHANGELOG.md) 和当前源码为准。

## 能力与边界

- **KeyMint 身份对齐**：B 的 `profile` 任务读取真实默认 HAL 的 Stable AIDL 版本、接口
  哈希、`getHardwareInfo()`、安全级别和 StrongBox 可用性；A 在软件 TA 启动前取得并冻结
  该身份，后续证明必须携带同一 profile。冷启动网络不可用时等待重试，不提前冻结错误身份。
  A 校验叶证书的 challenge、应用 ID、版本和安全级别；**完整证书链的可信根、逐级签名与
  撤销校验尚未完成**。
- **原生 StrongBox 与 RKP**：`use_native_strongbox = true` 时，A 复用本机
  StrongBox HAL，证明密钥通过 `IRemotelyProvisionedComponent/strongbox` 获取；实际返回
  的安全级别会被校验。修改该后端只需重启 Ommega keymint，不重启整台设备。
- **每应用安全级别**：`全局默认`遵循设备能力与“禁用原生 StrongBox”总开关；
  `StrongBox` 覆盖全局禁用；`TEE` 把该应用的请求改写为 TEE。策略保存在
  `target-security.toml`，注入器实时读取。
- **冷启动与生物认证**：Binder hook 先于远程 RPC 安装；`ommega-rpc-warmup` 后台初始化
  RPC；`OnDeviceUnlocked`、auth token 和用户维护事件进入内存 mirror 队列，RPC 恢复后
  顺序重放。
- **B 端 SPL 与服务级运维**：SPL 与 OS 版本来自系统属性，不是硬件值；属性变化只重载
  原生 KeyMint 和 `keystore2`。日期只升不降；较高值下升级过的 keyblob 在较低值下可能
  无法使用。B 端只提供 `service.sh`，不提供 `post-fs-data.sh`。
- **relay 与 Server 稳定性**：短轮询与 TEE 工作线程分离，轮询下限 80 ms，单次 HTTP
  预算 3 秒；2.1.3 起 `b/poll` 使用独立的
  `RELAY_POLL_RATE_LIMIT_REQUESTS`，不再消耗 A 任务与 `b/result` 的工作配额。
- **电量遥测**：B 只读 `/sys/class/power_supply/battery`，最多每 5 秒采样一次，随现有
  认证 poll 上报；Server 在 `/status/` 暴露 `battery_percent`、`battery_charging`、
  `battery_updated_at_ms`。缺失、非法或过期数据保持未知，不补 0。
- **SOTER**：软件 SOTER TA 与 KeyMint 是两条独立配置路径，远程参数保存在
  `soterta/remote.conf`，不随 KeyMint 远程配置自动改变。远程失败不会静默改用本地密钥。
- **时限与灭屏**：A 端远端签名两次尝试和退避共用 10 秒 HTTP 预算，建钥证明使用独立的
  较长预算。B 端 `OMMEGA_RELAY_WAKELOCK=true` 在灭屏时维持 CPU 可用，每 30 秒续期
  90 秒租约，会增加耗电，且需要内核支持 `CONFIG_PM_WAKELOCKS`。

## 构建

以下命令从仓库根目录进入对应目录后执行；CI 使用固定的 `nightly-2026-09-01` 工具链。

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
# StrongBoxCapabilityMask
cd StrongBoxCapabilityMask
./gradlew :app:testDebugUnitTest :app:assembleRelease
```

完整发行构建建议使用 [Build workflow](https://github.com/Andrea-lyz/ommega/actions/workflows/build.yml)。
CI 只忽略纯 `*.md` 修改，TXT 修改仍会触发构建；默认使用源码版本，不自动递增或提交版本，
手动 `release_version` 仅覆盖本次构建。

## 兼容性与验证范围

以下是 **2026-09-05** 的测试记录，不是兼容设备清单，也不自动继承到后续版本。

| 项目 | A 端 | B 端 |
|---|---|---|
| 机型 | 一加 13，PJZ110 | 一加 11，PHB110 |
| Android / API / ABI | Android 16 / API 36 / arm64-v8a | Android 16 / API 36 / arm64-v8a |
| 固件构建号 | `PJZ110_16.0.10.501(CN01)` | `PHB110_16.0.2.400(CN01)` |
| 验证的硬件路径 | 原生 StrongBox + RKP 曾单独验证；远程验收时关闭 | 默认 TEE KeyMint，native relay |
| StrongBox 状态 | 有原生实现；Mask 后功能查询为 false | 当次固件查询为 false，未验证 B StrongBox |
| `app_attest_key` | true，Mask 保留 | true |

- 历史远程验收使用 A `remote=true`、`local_hw=false`、`disable_native_strongbox=true`、
  `use_native_strongbox=false`、GlobalDefault，并启用 Mask。它验证的是
  **A → Server → B TEE**，不是“双端 StrongBox 均通过”。
- Mask 在 LSPosed 2.2.0 / libxposed API 102 上验证；A/B 模块在 root 环境运行，结果不能
  推广为所有 root 管理器、ROM 或机型可用。
- A 安装器与 Mask 的 `minSdk` 为 29，只是安装门槛，不代表 Android 10 起全部可用；实际
  运行依赖 keystore2、KeyMint AIDL、注入器与固件接口匹配。
- CI 发布 arm64-v8a 模块；其他架构、Android 版本与 B StrongBox 均需独立验证。
- 2.1.4 通过主机 WebUI 检查、Android 全工作区 Clippy 与测试编译，B 端刷新/状态测试
  26/26 通过，客户测试设备状态卡显示 HTTP 204；整体应用流程仍需安装新版后验收。

历史测试中的 Integrity 三绿、Paytm 进入手机号页或间歇 `00000` 等结果属于当时窗口，
不构成对支付、绑卡或任何风控场景的保证。

## 安全与风险

- **B 端数据**：SPL 改动涉及凭据加密。keyblob 在较高 SPL 下升级后，可能要求始终保留该
  较高值；root 丢失后设备回到原生 SPL，已升级的 keyblob 不会回退。请把 B 当作数据可
  丢弃的设备，并保留设备外备份。现有开机自动应用和脚本防护都不保证 root 丢失后仍能
  原生解锁。
- **SPL 只升不降**：每项下限取设备原始值、当前上报值和已生效目标中的最高值。降低上报
  值曾导致 `INVALID_ARGUMENT`（-38），A 降日期也曾使 Wallet 绑卡失败。不要把删除配置
  或回滚模块当作密钥回滚方案。
- **StrongBoxCapabilityMask**：它是全局 `system_server` hook，影响所有读取系统功能表的
  应用；可能引起兼容性、崩溃或启动异常，也不能证明设备已隐藏解锁或通过完整性校验。
  只在 A 确有 StrongBox 且希望应用按“未声明”处理时使用，并阅读
  [Mask 说明](StrongBoxCapabilityMask/README.md)。
- **传输与凭据**：`tls_insecure: true` 跳过 A 的证书验证，B 接受无效证书；公网部署需
  自行保护传输、Token 与 Server。不要把 HTTPS 地址当作已认证身份的证明。
- **发布签名**：两个 APK 使用 Android 默认 debug 签名，CI 会缓存该调试身份；它不是正式
  发布签名，缓存失效可能导致签名身份变化，升级前务必核对证书。
- **无担保**：本项目按现状提供，不保证适用性、安全性或任何检测结果。使用者自行承担
  数据丢失、设备异常、账户限制、业务或财产损失等风险（以适用法律允许的范围为限）。

## 文档

| 文档 | 内容 |
|---|---|
| [A 端说明](a-side/source/README.md) | 注入与路由、远程配置、keybox、认证签名、构建与校验 |
| [B 端说明](b-side/source/README.md) | relay 协议、SPL 规则、电量遥测、灭屏可用性 |
| [Server 说明](server/source/README.md) | 电量字段、速率限制与本地测试 |
| [UI 说明](ui/README.md) | 共享设计系统、预览、浏览器 smoke 与部署回滚 |
| [Mask 说明](StrongBoxCapabilityMask/README.md) | hook 点、验证步骤、兼容前提与回滚 |
| [CHANGELOG.md](CHANGELOG.md) | 2.1.x / 2.0.x 逐版本更新要点 |

## 致谢与许可证

- 上游仓库：[jiyin004-jpg/ommega](https://github.com/jiyin004-jpg/ommega)，原作者 jiyin004
- 本 fork：[Andrea-lyz/ommega](https://github.com/Andrea-lyz/ommega)，维护者 Andrea-lyz

参考的开源实现：

| 项目 | 作者 | GitHub |
|---|---|---|
| Tricky Store | 5ec1cff | [5ec1cff/TrickyStore](https://github.com/5ec1cff/TrickyStore) |
| Tricky Addon | KOWX712 | [KOWX712/Tricky-Addon-Update-Target-List](https://github.com/KOWX712/Tricky-Addon-Update-Target-List) |
| OhMyKeymint | James Clef（qwq233） | [qwq233/OhMyKeymint](https://github.com/qwq233/OhMyKeymint) |
| KeyAttestation | vvb2060 | [vvb2060/KeyAttestation](https://github.com/vvb2060/KeyAttestation) |
| TEESimulator-RS | Enginex0 | [Enginex0/TEESimulator-RS](https://github.com/Enginex0/TEESimulator-RS) |

项目遵循仓库中现有的 AGPL-3.0-or-later、MIT、Apache-2.0 等对应源码许可证；本 README
不改变任何第三方代码的原许可证或版权归属。
