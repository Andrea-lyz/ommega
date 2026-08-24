# ommega

Ommega 三端远程 TEE 认证系统：A 端（服务提供端）通过真实硬件 TEE 提供远程认证、签名与解密能力；B 端（客户端）与 B 端 App 通过 server 中转接入；server 负责管理、计费与数据中转。

源码版本：1.3.0（打包日期：2026-08-22）

## 目录结构

```
ommega/
├── a-side/                  # A 端（服务提供端，Magisk 模块）
│   ├── source/              #   完整源码（Rust 工程：keymint 守护进程 + inject 注入器）
│   │                        #   已排除 target/、.git 编译中间产物
│   └── build/
│       └── ommega-a-release-arm64-v8a-1.3.0.zip   # A 端安装包（6.4MB，Magisk 刷入）
├── b-side/                  # B 端（客户端，Magisk 模块）
│   ├── source/              #   完整源码（Rust 工程：relay 守护进程）
│   │                        #   已排除 target/、.git
│   └── build/
│       └── ommegaclient-b-release-arm64-v8a-1.3.0.zip  # B 端安装包（1.8MB，Magisk 刷入）
├── b-app/                   # B 端 Android App（Kotlin 工程）
│   ├── source/              #   完整源码（app/src、Gradle 配置）
│   │                        #   已排除 app/build、.gradle 编译产物
│   └── build/
│       └── client-b-app-release.apk               # B 端 App 安装包（4.6MB）
└── server/                  # 服务端
    ├── source/              #   完整源码（Rust src/ + 运维脚本、测试、证书/诊断文件等）
    └── build/
        └── relay_rs-linux-x86_64-musl    # 服务端二进制（Linux x86_64 musl，约 12.4MB）
```

## 三端说明

| 端 | 角色 | 安装方式 | 关键产物 |
|----|------|----------|----------|
| a-side | A 端（注入方），Magisk 模块，含 keymint 守护与 inject 注入器 | Magisk 刷入 zip | ommega-a-release-arm64-v8a-1.3.0.zip |
| b-side | B 端（被注入方），Magisk 模块，relay 守护进程 | Magisk 刷入 zip | ommegaclient-b-release-arm64-v8a-1.3.0.zip |
| b-app | B 端管理 App，Android 界面 | adb install 安装 | client-b-app-release.apk |
| server | 服务端，提供管理接口与数据 | 部署 server/build 下二进制 | relay_rs-linux-x86_64-musl（12.4MB，05:18 构建） |

## 备注

- 源码均为当时打包状态，未做任何修改。
- 复制的源码已排除 Rust target/ 编译缓存、Android .gradle/build 中间产物，重新构建时正常执行
  `python build.py`（a-side / b-side）或 Gradle（b-app）即可。
