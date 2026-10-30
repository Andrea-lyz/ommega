# 发布流程

适用于 A / B / Server / B-app / StrongBoxCapabilityMask 的统一版本发布。README 只保留
当前版本号；逐版本说明只写进 [CHANGELOG.md](CHANGELOG.md) 和
[CHANGELOG.en.md](CHANGELOG.en.md)。

## 1. 准备

- 所有改动已合并到 `master`，CI 为绿。
- 工作区干净，并与 `origin/master` 同步。
- 先想清楚本次发布的内容：用户可见的修复或变化、受影响组件、升级注意事项。

## 2. 更新版本号

```sh
python3 .github/scripts/bump_version.py --version X.Y.Z
```

不传 `--version` 时默认把 patch 加一。脚本会更新：

- `a-side/source/Cargo.toml`、`a-side/source/ommega-injector/Cargo.toml`
- `b-side/source/Cargo.toml`、`server/source/Cargo.toml`
- 三个 `Cargo.lock` 中对应包的版本
- 两个 APK 的 `versionName`，并把 `versionCode` 加一
- `README.md` / `README.en.md` 中的三处版本引用：badge、正文“当前版本”、版本表

它**不会**改写 README 正文，也**不会**写 CHANGELOG。

## 3. 追加更新日志

在 `CHANGELOG.md` 顶部新增一个条目：

```markdown
## X.Y.Z — YYYY-MM-DD

- 改动与影响。
```

并在 `CHANGELOG.en.md` 追加对应的英文条目。条目至少说明：

- 改了哪些端（A / B / Server / APK）；
- 用户需要做什么，例如是否必须单独更新 Server、是否只更新某个模块、A 安装后要不要重启；
- 验证到什么程度，以及哪些部分仍需实机验收。

**不要再在 README 里新增 `### X.Y.Z 更新要点` 章节。** 那是 2.1.4 及更早版本的旧写法，
从下一个版本起停止使用；README 的版本由脚本维护，发布说明只进 CHANGELOG。

## 4. 校验

```sh
python3 .github/scripts/test_bump_version.py
python3 .github/scripts/bump_version.py --check
```

`--check` 同时核对 README 的当前版本号；不一致时 CI 的 `prepare` 会直接失败。

## 5. 提交

先检查暂存内容：

```sh
git add -u
git diff --cached --stat
git commit -m "chore(release): X.Y.Z"
git push origin master
```

发版提交应只包含版本号、`Cargo.lock` 和两份 CHANGELOG。如果 diff 里出现整篇 README
重写或新的“更新要点”章节，说明用错了流程。

## 6. 构建与发布资产

发布资产由 [Build workflow](https://github.com/Andrea-lyz/ommega/actions/workflows/build.yml)
在 `master` 上构建。发布 GitHub Release 时使用 tag `ommega_fix-X.Y.Z`，附上：

- 两个模块 ZIP、`relay_rs-linux-x86_64-musl`、两个 APK；
- `SHA256SUMS` 与 `build-info.json`；
- 与 CHANGELOG 对应的中英发布说明。

## 2.1.4 及更早版本

这些版本的 README 内嵌 `### X.Y.Z 更新要点`，因此每次发版提交都会整篇改写 README。
从下一个版本起按本文件执行。
