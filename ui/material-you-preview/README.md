# Ommega Material You preview

This standalone prototype explores a shared A/B WebUI design. It uses mock
data and does not call a device bridge, change module configuration, or control
services.

在仓库根目录运行 `node ui/material-you-preview/serve.mjs`，然后打开
`http://127.0.0.1:4184/`。A/B 切换、导航、主题、搜索、开关和弹窗仅供设计评审；
所有数据均为示例，不会读取或修改设备。
在 A 端概览的“预览状态”中可以切换最近请求结果，查看图标和状态文案。
B 端概览中的设备信息和连接状态为示例，电量位置显示“待接入”。
最近延迟展示 A 发出任务到收到 B 回传证书链的示例耗时；请求失败时两项状态转为红色。
应用页包含全局作用域、列表操作和每应用的独立策略页；正数密钥 ID 兼容是设置页的全局开关。证书调用次数为示例数据，零次不显示标签。所有开关只修改预览内的示例状态。
设置页的二级弹窗提供示例表单、保存与取消；语言选择仅演示布局，暂不切换整页文案。

颜色跟随浏览器的深浅色设置，右上角可手动切换；这版原型不读取 Android 壁纸颜色。

The visual direction takes inspiration from the public FlClash interface:
tonal surfaces, rounded cards, compact status blocks, and adaptive navigation.
The prototype is implemented independently using local HTML, CSS, and JavaScript.
