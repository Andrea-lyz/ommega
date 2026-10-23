# Build workflow

`build.yml` resolves the product version first. UI validation, the two Android
Rust builds, the server build and the APK builds then run independently. The
final artifact collection waits for every check and component to pass.

The Android toolchain reuses an installed LLVM 18 libclang when available and
installs the host packages only when discovery fails. The pinned NDK r28c uses
the setup action's job cache; the first run fills it and later runs can restore
it instead of downloading the NDK again. Protoc, Rust and Android API versions
remain pinned in the workflow/action.

Rust caches remain separated by component, target toolchain and dependency
inputs. Rust and Gradle caches can be written by master, manual runs and PRs
from this repository; external PRs restore them without writing. GitHub cache
scope still applies: PR caches cannot become master caches. Gradle's build
cache and daemon reuse are enabled for both APK projects.

Rust cache entries are immutable and keyed primarily by toolchain and
dependencies. A cache hit avoids dependency work; it does not guarantee that
changed workspace crates or final linking can be skipped. A new cache key or
an expired entry is filled by a successful run. Cache restore/save messages
and per-step durations provide the evidence for subsequent warm runs.

All existing Android Clippy/test compilation gates, UI tests, APK unit tests,
ELF linkage checks, signing checks and final bundle checks remain required.
Release compiler profiles and artifact formats are defined by the component
build scripts.
