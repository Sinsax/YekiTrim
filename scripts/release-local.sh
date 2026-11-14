#!/usr/bin/env bash
#
# scripts/release-local.sh —— 单机双端发版（替代 GitHub Actions）
#
#   Linux  产物：docker 容器（docker/Dockerfile.linux-x64，glibc 2.39 基线）内编译。
#                滚动发行版（Arch 等）直编的 ELF 带 GLIBC_2.43 这类符号，在目标发行版上
#                起不来 —— xtask 的门禁会直接拒收（crates/xtask/src/main.rs 的 MAX_GLIBC）。
#   Windows 产物：cargo-xwin 交叉编译（lld-link + 微软官方 SDK/CRT，由 xwin 拉取）。
#                三个已知的坑（都已在脚本里处理，改前先读）：
#                ① libwebp-sys 要对 target 编 C ⇒ 必须有 clang-cl（Arch 的 llvm 包不含，
#                   要 clang22 + llvm-libs22 两个包配套，clang-cl 才能加载）；
#                ② clang-cl 不认 libwebp-sys 的 -msse4.1 探针 ⇒ 用 TARGET_CPU=haswell
#                   走 -march=haswell（产物要求 AVX2，Haswell 2013+）；不想限制就在
#                   Windows 本机用真 MSVC 编；
#                ③ xwin 解包出的 SDK 库是 pathcch.lib/PATHCCH.lib，而代码请求混合大小写
#                   PathCch.lib ⇒ 首次链接前建一批规范大小写软链（幂等）。
#
# 用法：
#   scripts/release-local.sh                # 两端都出
#   scripts/release-local.sh linux          # 只出 Linux
#   scripts/release-local.sh windows        # 只出 Windows
#   SKIP_DOCKER_BUILD=1 scripts/release-local.sh linux   # 复用容器里已编好的产物
#
# 产物落在 release/（gitignored）。上传：gh release upload <tag> release/*.{zip,tar.gz}
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$HERE")"
cd "$ROOT"

TARGETS="${*:-linux windows}"
echo "==> 发版目标: $TARGETS"

want() { case " $TARGETS " in *" $1 "*) return 0;; *) return 1;; esac; }

# ---------- Linux（容器基线） ----------
if want linux; then
  echo "==> [linux] 构建镜像（已存在则跳过）"
  docker image inspect yekitrim-linux-x64-builder >/dev/null 2>&1 \
    || docker build -f docker/Dockerfile.linux-x64 -t yekitrim-linux-x64-builder .

  echo "==> [linux] 容器内编译"
  # SHERPA_ONNX_LIB_DIR 挂本机已有的 TTS-off 静态库（与 build.rs 自动下载的归档同源）
  docker run --rm --network host \
    -v "$ROOT:/src" \
    -v "${SHERPA_LIB:-/home/toki/project/sherpa-onnx-prebuilt-no-tts/build/cmake-build/install/lib}:/sherpa:ro" \
    -e SHERPA_ONNX_LIB_DIR=/sherpa \
    -w /src yekitrim-linux-x64-builder \
    cargo build --release -p yekitrim-cli --locked

  echo "==> [linux] 打包"
  cargo xtask package --target linux --skip-build --no-dhampir
fi

# ---------- Windows（cargo-xwin 交叉编译，宿主机直跑） ----------
if want windows; then
  LLVM22="$ROOT/.workbuddy/llvm22/usr/lib/llvm22"
  LLIBS22="$ROOT/.workbuddy/llvm-libs22/usr/lib"
  if ! [ -x "$LLVM22/bin/clang-cl" ] || ! [ -e "$LLIBS22/libLLVM.so.22.1" ]; then
    echo "==> [windows] 引导 llvm22 + llvm-libs22 工具链（一次性，~100MB）"
    mkdir -p "$ROOT/.workbuddy/llvm-pkg" "$ROOT/.workbuddy/llvm22" "$ROOT/.workbuddy/llvm-libs22"
    curl -fsSL -o "$ROOT/.workbuddy/llvm-pkg/clang22.pkg.tar.zst" \
      https://geo.mirror.pkgbuild.com/extra/os/x86_64/clang22-22.1.8-1-x86_64.pkg.tar.zst
    curl -fsSL -o "$ROOT/.workbuddy/llvm-pkg/llvm-libs22.pkg.tar.zst" \
      https://archive.archlinux.org/packages/l/llvm-libs/llvm-libs-22.1.8-1-x86_64.pkg.tar.zst
    tar --zstd -xf "$ROOT/.workbuddy/llvm-pkg/clang22.pkg.tar.zst" -C "$ROOT/.workbuddy/llvm22"
    tar --zstd -xf "$ROOT/.workbuddy/llvm-pkg/llvm-libs22.pkg.tar.zst" -C "$ROOT/.workbuddy/llvm-libs22"
  fi
  command -v lld-link >/dev/null || { echo "缺 lld-link（Arch: pacman -S lld）"; exit 1; }

  echo "==> [windows] 交叉编译（crt-static + x86-64-v3，匹配 /MT 静态库）"
  export PATH="$LLVM22/bin:$ROOT/.workbuddy/llvm-bin/usr/bin:$PATH"
  export LD_LIBRARY_PATH="$LLVM22/lib:$LLIBS22:${LD_LIBRARY_PATH:-}"
  export RUSTFLAGS="-C target-feature=+crt-static -C target-cpu=x86-64-v3"
  export TARGET_CPU=haswell
  export XWIN_CACHE_DIR="$ROOT/.workbuddy/xwin-cache"
  # 首次跑会由 vendored build.rs 下载 win 静态库归档；链接可能因 SDK 库大小写失败 ⇒
  # 先编一遍 → 补规范大小写软链 → 再编一遍（第二遍只做链接，很快）。
  cargo xwin build --release --target x86_64-pc-windows-msvc -p yekitrim-cli --locked || true
  UM="$ROOT/.workbuddy/xwin-cache/xwin/sdk/lib/um/x86_64"
  if [ -d "$UM" ]; then
    for n in PathCch Shlwapi Shell32 Ole32 OleAut32 Crypt32 Dwmapi Gdi32 User32 UxTheme Winmm Ws2_32 Shcore Comctl32 Comdlg32 Secur32 Webservices WindowsCodecs Dbghelp Setupapi Wintrust Iphlpapi Dnsapi Cfgmgr32 Powrprof Userenv WtsApi32 Netapi32 Version Wininet Urlmon Msi RuntimeObject Bcrypt Ntdll Kernel32 Advapi32 Rpcrt4; do
      [ -e "$UM/$n.lib" ] || { src=$(ls "$UM" | grep -iE "^$n\.lib$" | head -1); [ -n "$src" ] && ln -sf "$UM/$src" "$UM/$n.lib"; }
    done
  fi
  cargo xwin build --release --target x86_64-pc-windows-msvc -p yekitrim-cli --locked

  # xtask 的门禁与组装都从 target/release/ 取 exe；交叉产物在 triple 子目录里，拷过去
  mkdir -p target/release
  for exe in yekitrim.exe yekitrim-tray.exe; do
    cp -f "target/x86_64-pc-windows-msvc/release/$exe" "target/release/$exe"
  done

  echo "==> [windows] 打包"
  cargo xtask package --target windows --skip-build --no-dhampir
fi

echo "==> 完成。产物："
ls -lh release/*.zip release/*.tar.gz 2>/dev/null || true

