import {text} from './ui-kit.js';
const en = {
  '刷新状态':'Refresh status', '正在读取 relay 状态…':'Reading relay status…', '硬件证明，随时就绪。':'Hardware trust. Ready to relay.', '管理专用 B 端的连接与设备信息。':'Manage the connection and device details for your dedicated B.',
  '请求端':'Client', '转发':'Relay', '硬件 TEE':'Hardware TEE', '专用设备':'Dedicated device', '当前系统属性':'Current system property', '当前系统版本':'Current OS version', '设备 ID':'Device ID', '服务器':'Server',
  '进程状态来自本机读取，不代表服务器已连接。':'Process status is read locally. It does not confirm a server connection.',
  '远程连接':'Remote connection', '重新读取':'Reload', '连接参数保存后自动加载，正在处理的任务沿用原连接。':'Connection changes load automatically. Active tasks keep their current connection.',
  '服务器地址':'Server URL', '与 Server 注册的设备 ID 一致':'Match the ID registered on the server', 'B 端连接凭据':'B connection credential', '高级连接选项':'Advanced connection options', '机器 ID · 可选':'Machine ID · Optional', '未分配时留空':'Leave empty if unassigned',
  '日志':'Logging', '日志开关和级别在 relay 下次启动时生效。':'Logging options take effect when the relay next starts.', '文件日志':'File logging', '系统日志（logcat）':'System log (logcat)', '记录级别':'Log level', '保存 Relay 配置':'Save relay configuration',
  '设置与设备信息':'Settings & device details', '设备属性（SPL 与 OS 版本）':'Device properties (SPL & OS)',
  '单独保存和应用。修改 SPL 或 OS 版本会重载 KeyMint 与 keystore2；不能低于设备原始值，留空恢复原始值。在新日期下产生或升级过的密钥，降回旧日期后可能永久不可用。':'Saved and applied separately. Changing SPL or OS reloads KeyMint and keystore2. Values cannot be below the original device values; leave blank to restore them. Keys created or upgraded under a newer date may become permanently unusable after lowering it.',
  'OS 版本':'OS version', '如 17，留空恢复原始值':'e.g. 17; blank restores the original', 'OS 版本只影响 ro.build.version.release 与证明 OS_VERSION；SDK 等级不变。':'OS version changes ro.build.version.release and attestation OS_VERSION; the SDK level stays unchanged.',
  '保存并应用':'Save & apply', '三项使用 System 值':'Use System for all three', '清空全部填写':'Clear fields', 'Boot SPL 是否被硬件接受，以新生成的证明证书为准。':'Only a fresh attestation certificate confirms hardware acceptance of Boot SPL.',
  '主导航':'Main navigation', '概览':'Overview', '连接':'Connection', '设置':'Settings', '显示':'Show', '隐藏':'Hide',
  '正在读取配置…':'Reading configuration…', '读取失败，请检查管理器的 root 权限。':'Read failed. Check the module manager’s root permission.', '尚无配置，请填写连接参数后保存。':'No configuration yet. Enter the connection settings and save.', '已读取保存的配置。':'Saved configuration loaded.',
  '配置未改变，无需保存。':'No changes to save.', '正在保存…':'Saving…', '配置已被其他操作修改，请重新读取后再保存。':'Configuration changed elsewhere. Reload before saving.', '保存失败，请重新读取配置后检查。':'Save failed. Reload and check the configuration.',
  '配置已写入，但复读失败，请重新读取确认。':'Saved, but verification failed. Reload to confirm.', '保存后配置发生变化，请重新读取确认。':'Configuration changed after saving. Reload to confirm.', '配置已保存。':'Configuration saved. ',
  '连接参数将由运行中的 relay 自动加载。':'The running relay will load the new connection settings. ', '日志选项在 relay 下次启动时生效。':'Logging options take effect when the relay next starts. ', '加载通知未写入，将由文件监测触发加载。':'Reload notification failed; file monitoring will trigger the reload.',
  '状态不可用':'Status unavailable', '设备原始值：不可用':'Original device values: unavailable', '未知':'Unknown', '（基线回退）':' (baseline fallback)', '设备原始值：System ':'Original values: System ', '。低于原始值的保存会被拒绝。':'. Values below the originals are rejected.',
  '日期格式必须为 YYYY-MM-DD，或留空恢复设备原始值。':'Use YYYY-MM-DD, or leave blank to restore the original device value.', 'OS 版本必须是 1-2 位数字，或留空恢复设备原始值。':'OS version must be 1–2 digits, or blank to restore the original.', '正在保存并应用…':'Saving and applying…', '已应用。新生成的证明证书才会体现 OS 版本；已存在的密钥不变。':'Applied. New attestations reflect the OS version; existing keys stay unchanged.', '应用失败：':'Apply failed: ',
  '配置值不能包含空格、换行或控制字符。':'Configuration values cannot contain spaces, newlines or control characters.', '请填写服务器地址、设备 ID 和 Token。':'Enter the server URL, device ID and token.', '请输入有效的 HTTP 或 HTTPS 服务器地址。':'Enter a valid HTTP or HTTPS server URL.', '服务器地址应为 HTTP 或 HTTPS 基础地址，不包含账号、查询参数或片段。':'Use an HTTP or HTTPS base URL without credentials, query parameters or fragments.', 'Token 只能包含非空白 ASCII 字符。':'The token must contain non-whitespace ASCII characters only.', '请选择有效的日志开关和级别。':'Select valid logging options and levels.', '无法读取配置校验值，请重新读取。':'Could not read the configuration checksum. Reload.', '配置校验值无效。':'Invalid configuration checksum.',
  '未确认 relay 运行状态':'Relay process not confirmed', 'relay 运行中':'Relay process running', '尚有未保存的修改，继续会丢失。切换语言？':'Unsaved changes will be lost. Change language?', '状态读取于':'Status read at',
};
export const t = value => text(value, en[value] || value);
export function translatePage() {
  document.querySelectorAll('[data-msg]').forEach(el => { el.textContent = t(el.dataset.msg); });
  document.querySelectorAll('[data-placeholder]').forEach(el => { el.placeholder = t(el.dataset.placeholder); });
  document.querySelectorAll('[data-label]').forEach(el => { el.setAttribute('aria-label',t(el.dataset.label)); });
}
