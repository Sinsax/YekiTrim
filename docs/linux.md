# YekiTrim on Linux (x86_64)

> 2026-10-30 ｜ 本文是 Linux 的**当前操作口径**；`docs/linux-port-handover.md` 与
> `docs/plan/linux-adaptation-test-report.md` 是 2026-10-03/05 的移植过程记录（历史）。
> 两者冲突时以本文 + 代码为准。

## 0. 一页状态

| 面 | 状态 |
|---|---|
| 构建（debug/release） | ✅ 需要一份 **TTS-off 的 sherpa-onnx Linux 静态库**，用 `SHERPA_ONNX_LIB_DIR` 指过去 |
| 官方 Linux 发行包 | ✅ 在 `docker/Dockerfile.linux-x64`（ubuntu 24.04 / glibc 2.39）里编，`cargo xtask package --target linux` 出 `tar.gz` |
| WebUI | ✅ release 内嵌在二进制里（debug 读 `webui/dist`），与平台无关 |
| HTTP / 任务队列 / 配置 / 账本 | ✅ 平台无关 |
| ASR（sherpa-onnx） | ✅ 用上面的 TTS-off 库；**绝不能**链 espeak-ng（GPL） |
| 出片 | ⚠️ 需要 **Dhampir 底座的 Linux 产物**（`dhampir/bin/dhampir` + `preview/` + `VERSION`）；本仓只做转译与音轨 |
| 托盘 | ❌ Linux 没有托盘；用 systemd user unit / shell 脚本（见 §3） |
| 数据目录 | ⚠️ 默认仍是 **exe 同级**（`YEKITRIM_DATA_DIR` 可覆盖）。系统级安装到只读目录时**必须**设它 |

## 1. 构建

```bash
# 1) 一份关掉 TTS 的 sherpa-onnx（不能带 espeak-ng）。已有的话直接指：
export SHERPA_ONNX_LIB_DIR=/path/to/sherpa-onnx/build/cmake-build/install/lib
cargo build -p yekitrim-cli --bin yekitrim          # debug
cargo build --release -p yekitrim-cli --bin yekitrim # release（本机调试用，发布见 §2）
```

没有 Linux 库时：在 sherpa-onnx 源码里用 `-DSHERPA_ONNX_ENABLE_TTS=OFF` 编一份静态库，
把 `install/lib` 指给 `SHERPA_ONNX_LIB_DIR`。**不要把 `[patch.crates-io] sherpa-onnx-sys` 删掉**
（那会链上 GPL 的 espeak-ng，整个发行物变成 GPL 组合作品）。

## 2. 官方发行包（固定 glibc 基线）

在 ubuntu:24.04 容器里编，避免滚动发行版把 `GLIBC_2.43` 写进产物：

```bash
docker build --network host -f docker/Dockerfile.linux-x64 -t yekitrim-linux-x64-builder .
docker run --rm --network host \
  -v "$PWD:/src" \
  -v ../sherpa-onnx-prebuilt-no-tts/build/cmake-build/install/lib:/sherpa:ro \
  -e SHERPA_ONNX_LIB_DIR=/sherpa \
  -w /src yekitrim-linux-x64-builder \
  cargo build --release -p yekitrim-cli --locked

cargo xtask package --target linux --skip-build --smoke-test \
  --dhampir-exe ../Dhampir/target/release/dhampir
```

产物：`release/yekitrim-v<版本>-linux-x64.tar.gz`（**tar.gz**，保留 0755）+ `.sha256.txt`。
打包器会做两件强制门禁：`cargo xtask scan`（ELF 里的 espeak-ng/GPL 标记）与
`GLIBC_* <= 2.39` 检查。缺 Dhampir 产物时用 `--no-dhampir` 只打本体（出片不可用）。

**2026-10-30 实测**：`yekitrim-v0.4.2-linux-x64.tar.gz` 42.3 MB，sha256 `92ac30d8e…f2e375`；
scan 5 个标记全 0、glibc 2.39、冒烟 `/api/health` + WebUI 通过。
⚠️ 烟测会在包目录里生成 `global/`、`users/`、`resources/fonts/*` 等运行态 ——
打包器现在会把它们清掉（以前 Linux 包因此多出约 15 MB 内置字体，且 `global/` 里带着烟测解包路径）。

## 3. 运行

```bash
# 本机桌面/单机：只监听 loopback；--local-no-auth 仅限本机
YEKITRIM_DATA_DIR="$HOME/.local/share/yekitrim" \
  ./yekitrim server --local-no-auth --port 26320
```

⚠️ `--local-no-auth` 只允许 loopback，且仍要求 `Host` 为 loopback、写请求同源。
对外服务不要用它 —— 用账号登录 + `--allow-remote` + 反代，见 `docs/deploy.md`。

`systemd --user` 示例（`~/.config/systemd/user/yekitrim.service`）：

```ini
[Unit]
Description=YekiTrim
After=network.target

[Service]
ExecStart=%h/YekiTrim/yekitrim server --local-no-auth --port 26320
Environment=YEKITRIM_DATA_DIR=%h/.local/share/yekitrim
WorkingDirectory=%h/YekiTrim
Restart=on-failure
RestartSec=3
# 停 unit 时把 ffmpeg / Dhampir 子进程一起收掉
KillMode=control-group

[Install]
WantedBy=default.target
```

```bash
systemctl --user daemon-reload
systemctl --user enable --now yekitrim
```

优雅关停走跨进程契约（需要设置同一个令牌；不要直接 `kill -9`）：

```bash
export YEKITRIM_SHUTDOWN_TOKEN='与 server 进程一致的令牌'
curl -s -X POST -H "x-yekitrim-shutdown-token: $YEKITRIM_SHUTDOWN_TOKEN" \
  http://127.0.0.1:26320/api/shutdown
```

## 4. 验证（Linux）

```bash
export SHERPA_ONNX_LIB_DIR=/path/to/sherpa-libs
cargo check --workspace --all-targets                 # 0 warnings
cargo test --workspace                                # 全绿；少数 env-only 门禁见下
python3 scripts/check_module_map.py
python3 scripts/check_module_boundaries.py
npm --prefix webui run build
for f in webui/scripts/check-*.mjs scripts/check-preview-audio.mjs; do node "$f"; done
```

### 仍然依赖外部环境的 6 条

| 判据 | 缺什么 | 处理 |
|---|---|---|
| `scripts/check-39-tiers.mjs` | Dhampir v7 的 `per_char_out_frames / scale_curve / scale_cycle_ms` 与 `.evidence/` 真机图 | 换 v7 底座产物（本仓期望 7） |
| `scripts/check-39-tiers-pixels.mjs` | Windows `dhampir.exe`（脚本写死 `F:/para/...`） | 在 Windows 侧跑，或改脚本的候选路径 |
| `scripts/check-perchar-stagger.mjs` | 同上 | 同上 |
| `scripts/check-dhampir-wiring.mjs` | 底座 `HOST_API_VERSION` = 7（本地是 6） | 换 v7 底座 |
| `scripts/check-text-motion-parity.mjs` | 底座仓没有 `text_motion.rs`（v6） | 换 v7 底座 |
| `scripts/check-render-visible.mjs` | 需要 `--project <片段目录>` 或 `--self-test` | 不是自动门禁，按用法跑 |

另有 1 条 `scripts/check-dhampir-drop-list.mjs` 在缺 `docs/wgpu-effect-reference/demo/clip.mp4`
（`.gitignore` 排除的样本）时**明说 SKIP**，不是通过。

## 5. 已知限制 / 排障

- **没有托盘**（Windows 才有）。Linux 用 systemd unit + `curl /api/shutdown`。
- **数据目录默认 exe 同级**：装到 `/usr/bin` / `/opt` 写不进去时首次启动会失败；设
  `YEKITRIM_DATA_DIR`（或 systemd 的 `Environment=`）。这是**有意只保留一个根**（见根 AGENTS）——
  不要再加"用户目录优先"的第二条查找路。
- **glibc ≥ 2.39**（ubuntu 24.04 基线）；更老的发行版用不了官方包。
- **出片必须有 Dhampir 底座**；没有底座只能用 `--no-dhampir` 打本体（预览/切片/字幕可用，出片不可用）。
- `cargo build` 报 `YekiTrim's vendored sherpa-onnx-sys only ships ... win-x64`：就是没设
  `SHERPA_ONNX_LIB_DIR`（或目录不对）。
- `GLIBC_2.43 not found`：产物是在滚动发行版上直编的；用 §2 的容器重编。
- 文件名大小写敏感：Linux 上 `Clip.mp4` ≠ `clip.mp4`；按扩展名匹配的素材发现已经统一走
  `is_video_file`/`find_danmaku` 这类小写比较，自建脚本别自己写 `endsWith('.MP4')`。
