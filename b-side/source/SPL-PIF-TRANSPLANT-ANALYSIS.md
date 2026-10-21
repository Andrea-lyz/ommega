# PlayIntegrityFix 机制分析与 B 端移植评估

日期：2026-10-21（Europe/Berlin）
范围：本地只读。仓库 clone 到临时目录后按 `inject_s` 分支 HEAD 读源码；
本轮**未**在 A/B 任何设备上执行操作，未写属性、未启停服务、未接触 TA。

## 0. 结论

PIF 的原理**不能**解决“让 Google 钱包接受 A 借来的 B 证书链”，但它的两条工程
原则对我们的 B 端方案有实质帮助，而且我们自己的 B 端模块已经有一半：

1. **改源头，不改结果**：PIF 从不改写 Play Integrity 判决，也不生成软件证明，
   它只让 GMS 看到一个自洽的“已认证设备身份”，证书链仍然是真硬件签发的。
   对应到我们：让 stock HAL 在启动时把目标日期正常 configure 进 TA，而不是用
   `0x2116` 直连 TA 去改 TA RAM。
2. **单向写入，不做还原**：PIF 把 SPL 写进模块的 `system.prop`（开机早期生效），
   之后不主动回退。这正好避开我们踩过的最大坑——keyblob 棘轮：blob 在新日期下
   铸造/升级后，设备一旦报回旧日期，这些 key 的签名与证明全部 `INVALID_ARGUMENT`
   (-38)，模块侧无解。

可执行的下一步是 **P1：B 端“开机即生效的单向日期身份”**（见 §4），
并且**先核对“型号 + OS 版本”的认证一致性**（见 §5），否则 Strong 能过、
钱包仍可能拒绑。

## 1. PIF 实际做了什么（静态确认）

取证对象：<https://github.com/KOWX712/PlayIntegrityFix>，clone 后 HEAD 位于
`inject_s` 分支（该分支即仓库自称的 `[INJECT]` 变体）。

### 1.1 注入面

`zygisk/src/main/cpp/zygisk.cpp` 的 `preAppSpecialize` 只在
`com.google.android.gms` 与 `com.android.vending` 两个进程继续，其它进程一律
`DLCLOSE_MODULE_LIBRARY`。`post-fs-data`/`service` 阶段不加载 payload 到业务进程。

### 1.2 三层机制

| 层 | 文件 | 作用 |
|---|---|---|
| 原生属性虚拟化 | `zygisk/src/main/cpp/property_virtualizer.cpp` | 把本进程 `/dev/__properties__/` 各属性区用 `MAP_FIXED` 重映射为**私有副本**，只在私有页上改值。GMS 进程内看到假值，系统其它进程仍看到真值；磁盘上的属性文件与其它进程的 `getprop` 都不受影响。 |
| dex 注入 + 反射 | `zygisk/src/main/cpp/zygisk.cpp`（`InMemoryDexClassLoader`）+ `zygisk/src/main/java/.../EntryPoint.java` | 向 GMS 的 classloader 注入 dex，用 `HiddenApiBypass` 反射改 `Build.*` 字段；把 `Security.getProvider("AndroidKeyStore")` 换成 `CustomProvider`。 |
| 模块脚本 | `module/security_patch.sh`、`module/autopif.sh`、`module/action.sh` | `action.sh` → `autopif.sh` → `security_patch.sh`：写 `ro.build.version.security_patch` 与 `ro.vendor.build.security_patch`（`resetprop -n`），并落一份 `system.prop` 到模块目录（开机早期由 Magisk/KSU 加载），另写 TrickyStore `security_patch.txt`、qwq233 `devconfig.toml`。 |

属性虚拟化覆盖的键（`overrideFor`）：`ro.build.fingerprint`、`ro.build.id`、
`ro.product.device|name|brand|model`、`ro.build.version.security_patch`、
`ro.board.first_api_level`=32、`ro.product.first_api_level`、
`init.svc.adbd`=stopped、`sys.usb.state`=mtp。

### 1.3 PIF 明确**没有**做的事

- `CustomKeyStoreSpi` 是**透传包装器**：所有 `engine*` 方法都转调真实
  `AndroidKeyStore` SPI；它只在 `engineGetCertificateChain` 命中 DroidGuard 调用栈时
  抛 `UnsupportedOperationException`。**它不生成任何软件证明**。
- 不改写证书链内容，不改 attestation extension，不接触 TA / KeyMint。
- README 自述：“It only serves to pass Device verdict in the Play Integrity tests
  and certify your device.”——目标是让 GMS 认为“这是一台已认证设备”。

## 2. 与我们项目的三条对照结论

### 2.1 判决层伪造对“钱包绑卡”无效（不要再投入）

Google 钱包绑卡是服务端拿**证书链本身**去验签，并核对认证/VSR 记录；
Play Integrity 的“Strong”判定是另一条检查。这与已实测的“A 端把 SPL 降到
2025-12-01 → Strong 通过但钱包拒绑”是同一类现象：**判定通过 ≠ 链被接受**。
PIF 的价值全在“让判定通过”，因此它解决不了我们的问题。

### 2.2 “改源头”路线对我们有效，而且我们已经有了

`b-side/source/AGENTS.md` 明确记录：原厂 HAL 从**系统属性**读取 OS 版本与
SPL 覆盖值（`ro.build.version.release` 决定 tag 705）。上游
`b-side/source/template/spl-control.sh` 的实现就是 `resetprop -n` 写这四个属性，
然后 `setprop ctl.restart` 重启 KeyMint/keymaster 服务与 `keystore2`，等待
`IKeyMintDevice/default` 重新 `found`。

这与 PIF 的 `security_patch.sh` 是同源做法。**所以“让 B 端硬件链带上目标日期”
不需要 `0x2116` 直连 TA**：直连方案只是把“写入”从属性层挪到了 TA 层，代价是
QSEECom 生命周期风险、需要排他/排空论证（见
`SPL-ISOLATION-FEASIBILITY-AUDIT.md` 的结论）、以及每次请求的额外延迟。

### 2.3 真正的差别在“时机”与“单向性”

PIF 的写入落在**消费者首次读取之前**：

- `system.prop` 由模块系统在开机早期加载，HAL 第一次读到的就是目标值；
- 运行期的 `resetprop -n` 只是让当前这次生效，不依赖任何服务重启。

我们的上游实现是 `service.sh` 启动时 apply → **运行期**改属性 → 再
`ctl.restart` KeyMint 栈。这带来两个我们已经在 B 上付出代价的后果：

1. 改属性时 B 自己的 KeyMint 客户端是活的 → 它们的 keyblob 在新日期下被铸造
   或升级；
2. 只要模块提供了“清空字段 = 恢复设备原值”这条路径（上游确实提供），
   一次还原就会把设备带回旧日期 → 已被棘轮抬高的 blob 全部失效 → 密码解锁失败、
   应用起不来（这正是 B 端那次事故）。

PIF 的做法是：**只写不撤**。它甚至不带“把 SPL 改回去”的日常操作
（`security_patch.sh --disable` 会连 `system.prop` 一起删，属于显式卸载动作）。

## 3. 移植清单：可帮 / 不可帮 / 变体

| PIF 机制 | 能否帮我们 | 说明 |
|---|---|---|
| 属性层改 SPL（`resetprop -n` + `system.prop`） | **能，主线** | 目标日期写进系统属性，stock HAL 自己 configure 进 TA；无直连 TA、无排他论证、无窗口延迟。 |
| 开机早期生效（`system.prop`） | **能，主线** | HAL 首次读取即为目标值 → 不必运行期重启 KeyMint 栈 → 避开“活客户端被改值”的窗口，B 自己的凭据从一开始就在新日期下铸造。 |
| 单向、不还原 | **能，主线** | 把“恢复设备原值”从生产路径移除；一旦需要还原，走恢复出厂设置。 |
| 进程内属性虚拟化（PROP-VIRT） | **弱，变体** | 只让 KeyMint 进程看到新日期，B 的框架/商店仍看到真值。副作用更小，但 TA 仍只有全局一份，棘轮问题与全局方案同源；额外引入“TA 视图 ≠ 系统视图”的不一致。备选 P3，不作主线。 |
| 反射改 `Build.*` / 替换 Provider / 拦截证书链读取 | **不能** | 这些是 GMS 侧的“自证”手段；我们的链必须由真硬件签发，拦截读取只会让 A 端拿不到链。 |
| 判决注入（`[INJECT]` 分支的强改 verdict） | **不能** | 钱包绑卡验的是链本身，判定层伪造过不了（§2.1）。 |
| 冒充已认证型号 | **本项目走不通** | 硬件链上的型号/品牌/attestation id 由 TA 签发、不可写；`0x2116` 只接受 OS/System/Vendor 三个字段。PIF 能改型号是因为它改的是 GMS 看到的属性。 |

## 4. 建议方案 P1：B 端“开机即生效的单向日期身份”

目标：B 永久以目标日期（= A 的日期）运行，硬件链稳定携带该日期；B 自身不再需要
任何“还原”动作。

实现要点（相对当前上游实现的改动）：

1. **写入前移**：目标值写入模块的 `system.prop`（`ro.build.version.security_patch`、
   `ro.vendor.build.security_patch`、`ro.build.version.release`），在开机早期生效，
   使 KeyMint HAL 首次启动即读到目标值。
2. **取消运行期重启**：默认值路径下不再执行 `setprop ctl.restart`（KeyMint/
   keymaster/`keystore2`）。只有在 WebUI 交互式改值时才需要重启，并显式提示该
   操作会按新日期铸造/升级 keyblob。
3. **移除“恢复原值”路径**：WebUI 的“清空字段 = 恢复设备原值”改为“需恢复出厂
   设置”。保留 `spl-origin.conf` 的原值记录，仅用于显示与拒绝降级写入，不再
   作为可回滚目标。
4. **降级护栏保留**：现有 floor 逻辑（不得低于设备原值）继续生效；只是“回到
   floor”不再是常规操作。
5. **目标值来源**：仍从 `/data/adb/ommega/spl.conf` 读取（保持现有 WebUI 形态），
   但配置文件语义从“可回滚覆盖”改为“单向身份”。
6. **验收方式**：`service.sh` 启动后用一次**真实硬件证明**回读证书确认日期，
   不以 `getprop` 变化作为证据（上游 README 已写明这条要求）。

代价与边界（诚实列出）：

- B 永久是“新日期设备”，不再是“一 B 对多 A 的按请求自适应”。当前 AB 一对一
  场景可接受；多 A 场景仍需回到可变窗口方案。
- 硬断电掉 root 后 B 回到原日期，其 blob 已被棘轮抬高 → B 自身凭据失效，需恢复
  出厂重配。但该场景下 root 已丢、中继本来就不工作，**没有新增一类损失**。
- 软重启（KSU）保留 root 与 `system.prop` → 日期与 blob 保持自洽，不触发问题。
- B 自己的 GMS 也会看到新日期；PIF 的做法是让 GMS 侧同步看到同一套身份，
  否则 B 自身的 Play Integrity 会出现“属性新、链旧”的自相矛盾。若 B 只做中继，
  这一点不阻塞主线，但需在 B 侧记录。

## 5. 需要先核对的一件事：型号 + OS 版本的一致性

链上的 OS 版本是我们写进去的（tag 705 ← `ro.build.version.release`），而型号、
品牌、attestation id 来自 B 的真机。Google 的认证/VSR 是按 **(型号, OS 版本)**
查记录的：PI 的 Strong 判定不查这一层，钱包绑卡会查。

因此“OS=170000 + B 的真实型号”这个组合未必在 B 型号的认证记录里存在。
PIF 侧证：它在 GMS 侧把 `Build.VERSION.RELEASE` 与型号一起伪造成同一台已认证设备
的组合——版本与型号必须配套，这正是它的核心教训。

建议（只读、可立即做）：核对 B 型号在 Google 认证清单中是否存在对应 OS 版本的
认证记录，据此决定 B 端应该写 `OS=170000` 还是保持 B 真实 OS 版本、只抬 SPL。
在这点确认前，不要再把“Strong 通过”当成“钱包会接受”。

## 6. 待验证清单（需要设备授权后再执行）

1. stock HAL 首次启动时把属性值 configure 进 TA（仓库文档如此描述；B 端早期
   resetprop 路线曾实际生效，可再以一次真实 attest 回读确认）。
2. `system.prop` 路径在 B 当前 KSU 环境下的加载时机是否早于 KeyMint 服务。
3. 目标 OS 版本与 B 型号的认证/VSR 兼容性（§5）。
4. B 自身 GMS 在新日期下的 Play Integrity 行为（仅影响 B，不影响主线）。

## 7. 附录：PIF 关键文件索引（本地临时 clone，仅供复核）

```
zygisk/src/main/cpp/zygisk.cpp                  进程选择、payload 注入、Build 字段改写
zygisk/src/main/cpp/property_virtualizer.cpp     进程内属性区私有化 + 打补丁
zygisk/src/main/cpp/pif_config.cpp              配置解析（fingerprint → 各字段）
zygisk/src/main/java/.../EntryPoint.java        HiddenApiBypass、Provider 替换、反射改字段
zygisk/src/main/java/.../CustomKeyStoreSpi.java 透传包装 + DroidGuard 拦截
module/action.sh → module/autopif.sh → module/security_patch.sh   SPL 写入链路
module/customize.sh                             system.prop 落盘与开机加载
```

相关内部文档：`SPL-ISOLATION-FEASIBILITY-AUDIT.md`（0x2116 与排他性结论）、
`SPL-HOT-CHANGE-PLAN.md`、`SPL-SINGLE-REQUEST-WINDOW-PLAN.md`、
`template/P0-BASELINE.md`（当前工作树把 `spl-control.sh` 置为只读）。
