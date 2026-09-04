# Ommega — Andrea-lyz Fork

> [!IMPORTANT]
> 本仓库是 [jiyin004-jpg/ommega](https://github.com/jiyin004-jpg/ommega) 的社区维护分支，
> 不是上游官方发行版或官方在线服务。当前源码版本为 **1.4.0**，模块作者信息为
> `jiyin004, Andrea-lyz`。

Ommega 是一套 A/B/Server 三端远程 KeyMint 系统：A 端拦截指定应用的 Android
Keystore 请求，经 Server 调度到 B 端真实硬件 TEE 执行，并把证明、签名和解密结果
返回 A 端。本 fork 在上游架构上重点补齐了远程身份一致性、原生 StrongBox、冷启动
生命周期、实时 WebUI 配置和 B 端软重启运维。

## 与上游的主要区别

### 1. A/B KeyMint 身份严格对齐

- B 端新增 `profile` 任务，读取真实默认 KeyMint HAL 的 Stable AIDL 版本、接口哈希、
  `getHardwareInfo()`、安全级别和 StrongBox 可用性。
- A 端在软件 TA 启动前取得并冻结该 profile；后续证明结果必须携带同一身份，防止
  AIDL 版本、硬件版本或安全级别混用。
- 冷启动网络暂不可用时会等待并重试远程 profile，不再提前冻结错误的本地身份。
- Server 对 B 端 profile 做结构校验；允许真实厂商 HAL 返回空
  `keymint_author`，但拒绝字段缺失、类型错误和仅含空白字符的值。
- 远程 profile、证明证书链或安全级别不一致时默认失败关闭；只有明确启用本地回退
  时才允许切换到本地路径。

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
| TEE | 强制使用 TEE；该应用的 StrongBox 请求返回不可用以触发标准降级 |

显式策略保存在：

```text
/data/misc/keystore/ommega/target-security.toml
```

远程配置页另有默认关闭的“禁用原生 StrongBox”开关。开启后，它只影响
`target.txt` 中处于“全局默认”的应用；显式 StrongBox/TEE 选择优先。

旧 `strongbox_unavailable_packages` 仅保留一次性迁移兼容：当
`target-security.toml` 尚不存在时可转成 TEE 策略，之后不再作为正式配置写回。
`target.txt` 的 `!`、`?` 后缀也不再是正式配置格式。

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

### 6. B 端 SPL WebUI 与纯软重启运维

- B 端 WebUI 可分别设置 System、Boot、Vendor SPL，保存后立即应用。
- 属性变化时只重载原生 KeyMint 服务和 `keystore2`，模块本身不请求硬重启。
- 配置保存在 `/data/adb/ommega/spl.conf`；留空可恢复首次记录的基线值。
- B 端故意不提供 `post-fs-data.sh`，只以 `service.sh` 作为生命周期入口；后续
  KernelSU 加载或软重启时由 `service.sh` 重新应用 SPL，再启动 relay。
- `service.sh` 会按精确模块路径清理旧 relay，避免旧进程继续占用 `relay.lock`。

> 修改属性不等于硬件 TEE 一定接受了新 Boot SPL。最终值必须以新生成的硬件证明
> 证书为准。

### 7. B 端 relay 与 Server 稳定性

- B 端 `relay.conf` 支持运行时热加载，修改连接和日志配置无需重启进程。
- 长轮询与 TEE 工作线程分离；任务执行期间仍可继续领取任务，降低批量检测延迟。
- 修复任务完成早于 waiter 注册、长轮询超时、重复 request ID 和掉线任务回收等队列
  竞态。
- Server 会校验证明证书链、profile、安全级别和任务归属，避免无效结果静默进入 A 端。
- Server 管理后台保留独立的“StrongBox 强健模式”：默认关闭；开启后，仅在 B 端
  StrongBox 返回能力类错误时，才在同一 B 设备上重试 TEE。

Server 强健模式与 A 端控制互不等价：

| 控制项 | 生效位置 | 用途 |
|---|---|---|
| `use_native_strongbox` | A 端 KeyMint 后端 | 使用 A 设备真实 StrongBox/RKP |
| 禁用原生 StrongBox | A 端应用路由 | 让全局默认目标应用按无 StrongBox 设备处理 |
| 每应用 StrongBox/TEE | A 端应用路由 | 覆盖全局默认行为 |
| Server StrongBox 强健模式 | B 端任务失败之后 | 能力类错误时重试 TEE；默认关闭，Server 重启后复位 |

### 8. 自动构建与版本管理

GitHub Actions 会执行 A/B Android Rust workspace 检查、Server 测试、A/B 模块构建、
B 端 ELF 动态链接检查、Linux musl Server 构建和 B-app APK 构建，并同步产品版本后
上传统一 artifact。工作流支持手动指定 `release_version`；本 fork 当前保持
**1.4.0**。

## 系统组成

| 端 | 角色 | 形态 |
|---|---|---|
| A 端（`a-side`） | 拦截目标应用 Keystore；远程 TEE；可选 A 端原生 StrongBox | KernelSU/APatch/Magisk 模块，arm64-v8a |
| B 端（`b-side`） | 调用真实硬件 KeyMint，处理 profile/attest/sign/decrypt | KernelSU/APatch/Magisk 模块，arm64-v8a |
| B 端 App（`b-app`） | 可选的独立 B 端客户端与连接配置界面 | Android APK |
| Server（`server`） | 鉴权、任务队列、A/B 调度、设备与后台管理 | Linux x86_64 musl 二进制 |

## 快速部署

本 fork 不在 README 中分发公共 Server Token。请自行部署 Server 并设置自定义凭据；
启用 MySQL 后可从管理后台为 A/B 分配独立的角色 Token。

### B 端

安装 B 模块，在 `/data/adb/ommega/relay.conf` 中填写：

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

配置由 relay 热加载；需要主动触发重新读取时执行：

```sh
touch /data/adb/ommega/restart.all
```

该标记不会硬重启设备。

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
| B `relay.conf` | Server、设备 ID、Token、日志 | relay 热加载 |
| B `spl.conf` | System/Boot/Vendor SPL | 保存即应用；`service.sh` 再应用 |

## 构建

```sh
# A 端模块
cd a-side/source
python build.py

# B 端模块
cd b-side/source
python build.py

# Server
cd server/source
cargo build --release

# B 端 App
cd b-app/source
./gradlew assembleRelease
```

完整发行构建建议使用仓库的
[Build workflow](https://github.com/Andrea-lyz/ommega/actions/workflows/build.yml)。

## 验证范围与限制

1.4.0 已在实际 A/B 设备上验证远程 TEE profile、证明/签名链路、原生
StrongBox + Android RKP、用户认证绑定密钥生命周期、B 端 SPL 即时应用和 Server
热更新。检测器结果会受到设备固件、应用版本和策略配置影响，本仓库不承诺所有应用或
所有检测器表现一致。

## 上游、署名与许可证

- 上游仓库：[jiyin004-jpg/ommega](https://github.com/jiyin004-jpg/ommega)
- 本 fork：[Andrea-lyz/ommega](https://github.com/Andrea-lyz/ommega)
- 原作者：jiyin004
- Fork 维护与 1.4.0 改动：Andrea-lyz

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
