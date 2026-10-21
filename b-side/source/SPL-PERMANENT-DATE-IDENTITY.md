# B 端单向日期身份（Permanent Date Identity）

日期：2026-10-21（Europe/Berlin）
分支：`codex/b-side-permanent-date-identity`（基于 `origin/master`）
状态：本地实现 + 本地测试；**未部署到 B，未触碰任何设备**。

## 1. 目的

B 端把 OS 版本与 SPL 改成 A 的日期，这条链路本身没有疑问；真正的问题���**还原**。
`ro.build.version.security_patch` / `ro.build.version.release` 对硬件 KeyMint 是
单向棘轮：在新日期下铸造或升级过的 keyblob，一旦设备报回旧日期，这些 key 的
签名与证明全部 `INVALID_ARGUMENT` (-38)，模块侧无解。B 端那次“密码失败、指纹
可用、应用全打不开”的事故就是这个机制造成的。

因此本分支不再提供“回到设备原值”这条路径：日期只允许升，不允许降。

## 2. 核心不变量

```
floor(prop) = max(设备原始值(prop), 已应用目标(prop), 设备当前上报值(prop))
```

任何一次写入都必须 >= floor。三个来源分别是：

| 来源 | 记录位置 | 含义 |
|---|---|---|
| 设备原始值 | 只读固件属性文件（`/system/build.prop` 等）→ `/data/adb/ommega/spl-origin.conf` | 出厂值，任何覆盖不得低于它 |
| 已应用目标 | 新增 `/data/adb/ommega/spl-applied.conf` | 本模块已经写入过的值，即 keyblob 铸造时依赖的日期 |
| 设备当前上报值 | `getprop` | 设备此刻报的日期；它可能来自别的工具或上一次会话，棘轮同样适用 |

由此得到三个性质：

1. **只升不降**：清空字段 = 保持当前生效值，不再是“恢复原始值”。
2. **自愈**：设备若在没有模块的情况下开机（属性回到原始值），下一次 `apply` 会
   依据 `spl-applied.conf` 把目标写回，于是依赖该日期的 keyblob 重新可用，
   不需要恢复出厂设置。
3. **不可逆的只有一种**：要回到设备出厂日期，只能恢复出厂设置。

## 3. 语义变化

| 操作 | 旧行为 | 新行为 |
|---|---|---|
| 清空字段 | 恢复设备原始值 | 保持当前生效值（已应用目标 / 当前上报值） |
| 写入 < 设备原始值 | 拒绝 | 拒绝（不变） |
| 写入 < 已应用目标 | 允许，导致 keyblob 失效 | 拒绝 |
| 写入 < 当前上报值 | 允许，导致 keyblob 失效 | 拒绝 |
| 模块缺席后开机 | 属性回原值，keyblob 永久失效 | 下次 apply 写回目标，keyblob 恢复可用 |
| `status` 输出 | 无已应用目标 | 增 `APPLIED_*` 与 `MODE` 相关字段 |

## 4. 改动文件

- `template/spl-control.sh`
  - 新增 `spl-applied.conf` 的读写（`load_applied` / `save_applied`）。
  - `floor_for` 取三个来源的最新值。
  - `keep_value`：空字段优先取已应用目标，其次当前上报值，最后设备原始值。
  - `apply_config` 成功结束后记录设备此刻上报的值。
  - 拒绝信息改为说明新的下限语义与“需要恢复出厂设置”。
- `template/webroot/index.html`、`template/webroot/app.js`：文案改为“只升不降 /
  留空保持当前生效值”，并在 SPL 区域显示实际下限。
- `README.md`：更新棘轮与下限说明。
- `scripts/spl-control.test.mjs`：随语义更新的用例，以及三个新用例
  （已应用目标成为下限、模块缺席后自愈、status 报告下限）。
- `SPL-PIF-TRANSPLANT-ANALYSIS.md`：PIF 机制分析与本方案的来源。

## 5. 与 0x2116 直连 TA 方案的关系

本方案不替代 `0x2116`，它把“B 端硬件链携带目标日期”这件事从**运行期窗口**
改成**开机常态**：

- 不再需要直连 QSEECom 改 TA RAM（去掉 TA 生命周期风险）；
- 不再需要证明“窗口内全部 KeyMint 客户端排他”（`SPL-ISOLATION-FEASIBILITY-AUDIT.md`
  的结论卡在这一步）；
- 不再有窗口延迟（属性写入 + KeyMint 栈重载只在目标变化时发生一次）。

代价：B 永久以目标日期运行，不再支持一 B 对多 A 的按请求自适应；硬断电丢 root
后 B 自身凭据需要重新 apply 目标才能恢复（不是恢复出厂，只是重新 apply）。

## 6. 待验证（需要设备授权）

1. B 端写入目标后，用一次**真实硬件证明**回读证书，确认 tag 705/706/718 携带目标
   日期（属性变了不等于 TEE 接受了）。
2. 目标 OS 版本与 B 型号在 Google 认证/VSR 记录中的兼容性——Strong 通过不等于
   钱包绑卡会接受。
3. B 自身 GMS 在新日期下的 Play Integrity 行为（只影响 B 自身，不影响中继主线）。
4. 软重启（KSU）后 `service.sh` 的 apply 不重启 KeyMint 栈是否成立（属性在开机
   早期已是目标值时）。

## 7. 回滚

本分支只改本地源码，未部署。回滚 = 放弃分支。若已部署，唯一能回到出厂日期的
手段是恢复出厂设置。
