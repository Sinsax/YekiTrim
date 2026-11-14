#!/usr/bin/env bash
#
# scripts/release-local.sh —— 单机双端发版（替代 GitHub Actions）
#
#   Linux  产物：docker 容器（docker/Dockerfile.linux-x64，glibc 2.39 基线）内编译，
#                否则滚动发行版上编出的 ELF 会带 GLIBC_2.43 这类符号、在目标发行版上起不来
#                （xtask 的门禁会直接拒收，见 crates/xtask/src/main.rs 的 MAX_GLIBC）。
#   Windows 产物：cargo-xwin 交叉编译（lld-link + 微软官方 SDK/CRT，由 xwin 拉取）。
#                sherpa-onnx 的 win 静态库由 vendored build.rs 自己按 sha256 下载；
#                RUSTFLAGS 开 crt-static 以匹配归档里的 /MT 库。
#                ⚠️ 本机没有 rc.exe ⇒ exe 不内嵌图标（build.rs 的设计内降级，只是外观）。
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
    -v /home/toki/project/sherpa-onnx-prebuilt-no-tts/build/cmake-build/install/lib:/sherpa:ro \
    -e SHERPA_ONNX_LIB_DIR=/sherpa \
    -w /src yekitrim-linux-x64-builder \
    cargo build --release -p yekitrim-cli --locked

  echo "==> [linux] 打包"
  cargo xtask package --target linux --skip-build --no-dhampir
fi

# ---------- Windows（cargo-xwin 交叉编译） ----------
if want windows; then
  echo "==> [windows] 检查工具链"
  rustup target view x86_64-pc-windows-msvc >/dev/null 2>&1 || rustup target add x86_64-pc-windows-msvc
  command -v cargo-xwin >/dev/null || cargo install cargo-xwin --locked
  command -v lld-link >/dev/null || { echo "缺 lld-link（pacman -S lld）"; exit 1; }

  echo "==> [windows] 交叉编译（crt-static 匹配 /MT 静态库）"
  export RUSTFLAGS="-C target-feature=+crt-static"
  export XWIN_CACHE_DIR="$ROOT/.workbuddy/xwin-cache"
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
ls -lh release/ | tail -8
