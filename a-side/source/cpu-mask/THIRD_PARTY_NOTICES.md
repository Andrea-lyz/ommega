# Third-party sources

`third_party/zygisk.hpp` is the unmodified official Zygisk API v4 header.
Copyright 2022-2023 John "topjohnwu" Wu. MIT license; the full original notice
is retained in the header. The MIT permission and warranty terms are also
included in the packaged `LICENSE`.

- Source: https://github.com/topjohnwu/zygisk-module-sample/blob/8ce26128f81baaed0b969aaf7f52f886b61af4ab/module/jni/zygisk.hpp
- Upstream SHA256: `f8d55e8b4f89d418c5941afe62ce6a09ddec1f4afd9a1b0a01eb40a93310dd28`

`module/META-INF/com/google/android/update-binary` is the official Magisk
module installer wrapper. It is distributed separately under Magisk's
GPL-3.0 license, included as `third_party/Magisk-GPL-3.0.txt` in the source
and `licenses/Magisk-GPL-3.0.txt` in the install ZIP.

- Source: https://github.com/topjohnwu/Magisk/blob/cf47214ee4912ed1538fbea3d09ba9dd9b5746b0/scripts/module_installer.sh
- Upstream SHA256: `bcf4b1d9913f3af17755569c853e0b5a75b8005f6a18eb3f86dadcc0e968c29d`

The pinned installer wrapper and its complete source are shipped in the ZIP.
MaskCPU's original C++ code and build helpers use the MIT license.
