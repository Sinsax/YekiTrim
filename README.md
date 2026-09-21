# V-Trim

直播录像自动切片工具。输入录播视频 + 弹幕文件，输出按事件分割的片段 + 字幕 + `publish_config.json`。

## 前置条件

- **ffmpeg** + **ffprobe** — 需在 PATH 中，用于音频提取和视频剪切
- **Rust** 工具链 (构建用)

## 项目结构

```
.
├── crates/
│   ├── vtrim-core/        # 管线编排 + ASR / 事件检测 / 剪切
│   ├── vtrim-polish/      # GSAP 精修引擎（时间轴 / 贴纸 / 音效 / 封面 / 出片）
│   ├── vtrim-render-core/ # GPU 绘制原语（被 polish / wasm 使用）
│   ├── vtrim-service/     # CLI 与 WebUI 共用的领域逻辑（账号 / 项目 / 录播姬 / 用量）
│   ├── vtrim-server/      # axum HTTP API + WebUI 宿主 + 任务队列（api/ 按领域分模块）
│   ├── vtrim-cli/         # 入口（clap 命令 + tracing 装配），二进制名 vtrim
│   ├── vtrim-wasm/        # 浏览器内真渲染预览（独立 target，不在 workspace members 里）
│   └── xtask/             # 构建 / 打包 / 产物门禁（cargo xtask scan|package）
├── webui/                 # WebUI 源码（Vue 3 + Vite）；webui/dist 是服务端唯一界面来源，已入库
├── docs/                  # 设计稿 / 计划 / 发行说明；docs/archived 是历史设计文档
├── AGENTS.md              # 工程约定（**路由页**：全局不变式 + 模块索引 + 改代码协议）
├── docs/module-map.json   # 模块结构注册表：模块 → 目录 → 模块卡 → 验证命令
├── ROADMAP.md             # 待排期功能
├── Cargo.toml / Cargo.lock
└── vtrim.toml             # 项目配置 (gitignored)
```

> **改代码前先读对应模块目录的 `AGENTS.md`**：全局规矩在根 `AGENTS.md`（路由页），
> 每个模块的"职责 / 入口 / 边界 / 验证"在它自己目录的卡里（清单见
> `docs/module-map.json` 与根 `AGENTS.md` 的模块索引表）。改完跑两条门禁：
> `python3 scripts/check_module_map.py`（结构/覆盖率/预算）与
> `python3 scripts/check_module_boundaries.py`（分层与前端边界）。

## 快速开始

```bash
# 构建
cargo build

# 创建项目 (--danmaku 可选，自动搜索同目录 .ass)
vtrim init input.mp4

# 下载 ASR 模型 (本地转写需要)
vtrim download

# 全自动管线
vtrim process
```

## CLI 命令

全局选项：

| 选项 | 作用 |
|------|------|
| `-c, --config <path>` | 指定配置文件路径 (默认 `./vtrim.toml`)，对所有子命令生效 |

### `vtrim init <video>`

创建 `vtrim.toml` 项目配置文件。

| 参数 | 作用 |
|------|------|
| `<video>` | 输入视频文件路径 **(必填)** |
| `--danmaku <file>` | 弹幕文件 (ASS 格式)。不传则自动寻找 `<video>.ass` |
| `-o, --output <dir>` | 默认 `./output` |
| `--config <path>` | `vtrim.toml` 输出路径 (默认 `./vtrim.toml`) |

自动发现：`--danmaku` 不传时，自动检查视频同名 `.ass` 文件，存在则写入配置。

### `vtrim process`

全自动管线处理。无视 `current_stage` 状态机，从头跑到尾，完成后设 `current_stage=4`。

| 参数 | 作用 |
|------|------|
| `-i, --input <file>` | 输入视频 (默认 `vtrim.toml` 中 `input_file`) |
| `-o, --output <dir>` | 输出目录 (默认 `vtrim.toml` 中 `output_dir`) |
| `--dry-run` | 仅分析不剪切，跑到 EventDetector 为止 |
| `--stages <stages>` | 逗号分隔指定阶段，默认 `audio,transcribe,detect,cut` |

流程：
1. 音频提取 (如未完成)
2. 语音转写 (如未完成，自动恢复已有 `transcript.json` / `video.srt`)
3. LLM 订正 (如 `asr.llm_correct=true` 且配置了 LLM key，且事件尚未检测)
4. 事件检测 (如未完成)
5. 视频剪切 (如未完成，`--dry-run` 跳过)

### `vtrim extract`

Stage 1 — 从视频提取 16kHz mono WAV。

| 参数 | 作用 |
|------|------|
| `-i, --input <file>` | 输入视频 (默认 `vtrim.toml` 中 `input_file`) |
| `-o, --output <dir>` | 输出目录 (默认 `vtrim.toml` 中 `output_dir`) |

输出：`<output_dir>/video.wav`，自动写入 `vtrim.toml` 的 `audio_file`。

**状态机约束**：`current_stage >= 0` 即可运行。

### `vtrim transcribe`

Stage 2 — 语音转文字 (local sherpa-onnx SenseVoice)。

| 参数 | 作用 |
|------|------|
| `-i, --input <file>` | 输入音视频 (默认 `vtrim.toml` 中 `input_file`，自动回退 `audio_file`) |
| `-o, --output <dir>` | 输出目录 (默认 `vtrim.toml` 中 `output_dir`) |

输出：
- `<output_dir>/video.srt` — 字幕 (UTF-8 BOM)
- `<output_dir>/transcript.json` — 结构化 JSON

自动写入 `vtrim.toml` 的 `transcript_file`。

并行策略：CPU 核数 / 4 (上限 4)，每 chunk 的 ONNX intra-op threads=2。

配置参数 (`.vtrim/config.toml`)：

| 参数 | 默认 | 说明 |
|------|------|------|
| `num_workers` | `0` | 0=auto (CPU/4, 上限4)，手动值不超过 CPU 核数 |
| `chunk_duration` | `60.0` | 每块秒数 |
| `chunk_overlap` | `10.0` | 块间重叠秒数 |
| `segment_gap` | `1.0` | 段间静音阈值(秒)，超过才切为两条字幕 |
| `language` | `"zh"` | ASR 语言 |

**状态机约束**：需先完成 `extract` (current >= 1)。

### `vtrim correct`

LLM 订正转录文本。

| 参数 | 作用 |
|------|------|
| `-t, --transcript <file>` | 转录 JSON/SRT 文件 |

自动发现优先级：`--transcript` 参数 > `vtrim.toml` 中 `transcript_file` > `<output_dir>/transcript.json`。

输出：`<output_dir>/video_llm.srt`，自动更新 `transcript_file` 指向此文件。

需要 `.vtrim/llm.toml` 已配置 API key。无状态机约束 (可独立运行)。

### `vtrim detect`

Stage 3 — LLM 分析转写结果，检测高价值事件。

| 参数 | 作用 |
|------|------|
| `-t, --transcript <file>` | 转录 JSON/SRT 文件 |

自动发现优先级：`--transcript` 参数 > `vtrim.toml` 中 `transcript_file` > `<output_dir>/transcript.json` > `<output_dir>/video.srt`。

输出：`<output_dir>/events.json` (EventStore 格式)。

自动写入 `vtrim.toml` 的 `events_file`。

检测逻辑：
- LLM API key 非空 → 调用 LLM 分析全部转录段 (单次调用，64k+ 上下文)
- LLM API key 为空 → 降级为单个默认事件 (覆盖整段视频)
- 检测到事件后 → 仅凭 `标题+概括` 让 LLM 按预测点击率/有趣程度/爆点强度给传播潜力分；
  再与弹幕/字幕热度按 `detection.llm_score_weight` 加权，输出 `popularity` (0~1) + `rank`
  （无弹幕时基础热度只用字幕密度；LLM 评分失败/关闭时回退基础热度）
- 主播名称 (`detection.vtuber_name`) 非空 → 自动追加到标题末尾

**状态机约束**：需先完成 `transcribe` (current >= 2)。

### `vtrim cut`

Stage 4 — 按事件片段剪切视频 + 生成字幕。

| 参数 | 作用 |
|------|------|
| `-i, --input <file>` | 输入视频 (默认 `vtrim.toml` 中 `input_file`) |
| `-e, --events <file>` | 事件 JSON 文件 |
| `-o, --output <dir>` | 输出目录 (默认 `vtrim.toml` 中 `output_dir`) |

自动发现优先级：`--events` 参数 > `vtrim.toml` 中 `events_file` > `<output_dir>/events.json`。

剪切输出结构：

```
<output_dir>/
└── clips/
    ├── clip-01/
    │   ├── clip.mp4           # 视频片段
    │   ├── clip.srt           # 裁剪后字幕
    │   ├── clip.dm.srt        # 弹幕字幕 (有弹幕时)
    │   └── clip.json          # 元数据 (标题、热度、原始时间范围)
    ├── clip-02/
    │   └── ...
    ├── clips_order.txt        # 按弹幕热度降序排列
    └── publish_config.json    # 发布配置
```

编码器自动检测顺序：`h264_nvenc` → `h264_qsv` → `h264_amf` → `libx264`。
字幕从原始 SRT 裁剪时间范围并偏移到 clip 起始时间。

**状态机约束**：需先完成 `detect` (current >= 3)。

### `vtrim silence`（编辑器里的「去静音」）

把已输出切片中的长停顿映射为**屏蔽轨道**（写入 `polish.toml` 的 `mute_ranges`），
不生成新视频文件，原始 `clip.mp4` / `clip.srt` 保持不变；导出时由 process/render 剪掉。
精修编辑器时间轴顶部的「去静音」快捷按钮等价于此命令。

| 参数 | 作用 |
|------|------|
| `[files...]` | 要处理的视频或目录。不传则扫描 `<cwd>/output/clips/*/` |
| `--duration <sec>` | 静音阈值(秒)，默认 `3.5`。超过此长度的连续静音才会被写入屏蔽轨道（3.5s 以内的字幕间隔一律不处理） |
| `--threshold <dB>` | 音频分析噪声门限，默认 `-30`（仅在缺少字幕文件时使用） |

未初始化的切片会先自动 `polish init`（映射到屏蔽轨道需要有 `polish.toml`）。
检测结果**并入**当前屏蔽轨道（与已有屏蔽相加：重叠/相邻的合成一段），每段静音两侧各留 0.8s
**气口**（字幕时间戳有误差，贴边剪会削掉首尾音节；实际剪掉的是 `间隔 − 1.6s`）。
**字幕间隔 3.5s 以内一律不动**（默认阈值，正常语气停顿就这个量级，剪了反而把对话切碎）。自动加进去的那部分记在 `silence_ranges`，
编辑器里的「撤销去静音」按它精确相减回退，手动添加的屏蔽区间不受影响。

静音检测策略：如有同名 `.srt` 文件，优先分析 SRT 字幕间隔；否则用 ffmpeg silencedetect filter。

### `vtrim download`

下载全部 ASR 模型到 `<程序>/.vtrim/models/<model>/`：`sensevoice`（~165MB，single 模式 + hybrid 打轴）、`qwen3-asr`（~879MB，hybrid 填词）。转写时按 `[asr] model` / `[asr] mode` 自动懒加载，缺失才提示下载。
运行时找模型的顺序是 **配置 `local_model_dir` → 用户目录 `~/.vtrim/models` → 程序目录 `.vtrim/models`**（模型管理页显示的就是实际用到的那一份）。

返回 LLM 完整 API response body。需要 `.vtrim/llm.toml` 已配置。

### `vtrim status`

显示当前处理进度。

```
视频工程: input.mp4
当前进度: Stage #2

  [✓] AudioExtractor
  [→] Transcriber
  [·] EventDetector
  [·] Cutter
```

### `vtrim config init`

生成全局配置文件（覆盖写，已有配置会被替换）。

| 参数 | 作用 |
|------|------|
| `--path <path>` | 输出路径 (默认 `.vtrim/config.toml`) |

生成文件：
- `.vtrim/config.toml` — ASR、检测等全局参数，含内建 prompt 模板
- `.vtrim/llm.toml` — LLM 独立配置 (API key、model 等)

> `vtrim server` 首次启动时若发现程序配置缺失，会自动补齐这两个文件（只创建缺失的，
> 已存在的配置不会被覆盖）；若已存在的文件 TOML 语法错误（整份配置都会被忽略），
> 会先备份为 `config.toml.broken` 再重建，并在启动日志里给出原因。

### `vtrim config show`

显示合并后的完整配置 (vtrim.toml + .vtrim/config.toml)。

### `vtrim version`

显示版本号。

---

### `vtrim server` — WebUI 服务（需要登录）

启动浏览器可视化服务，服务 `webui/dist` 构建产物。

> **认证不可关闭**：除 `/api/health` 与 `/api/auth/login` 外，**所有**接口都要求会话，
> 包括从 `127.0.0.1` 访问。没有"免登录/本机模式"分支——单人使用＝只有一个 `admin`
> 账号的部署，同样要登录（设计：`docs/superpowers/specs/2026-09-12-multi-user-server-design.md` §3.5）。

| 参数 | 作用 |
|------|------|
| `--host <addr>` | 监听地址，默认 `127.0.0.1` |
| `--port <n>` | 监听端口，默认 `26320` |
| `--open` | 启动后自动打开浏览器 |
| `--config <file>` | 打开指定工程（默认恢复上次工程） |
| `--allow-remote` | 允许监听非 loopback 地址（默认拒绝） |
| `--trust-proxy` | 采信反代转发的 `X-Forwarded-For` / `X-Forwarded-Proto`（默认**关**，见下） |
| `--data-dir <dir>` | 账号账本目录，默认 `<exe_dir>/.vtrim/cloud`（也可用 `VTRIM_DATA_DIR`） |
| `--admin-password-file <file>` | 无人值守部署：用该文件里的密码作为初始管理员密码 |
| `--force` | 接管残留的 `vtrim.lock`（确认上一个实例已崩溃时使用） |

> **端口为什么是 26320**：老默认 7788 在部分 Windows 上落进了 Hyper-V/WSL 的**保留端口段**
> （实测 7635–7834），表现是 `10013 WSAEACCES` 绑不上，而 `netstat` 里一个监听者都没有
> （所以不是"被占用"）。26320 落在跨平台安全带 **15001–32767** 内：既躲开 Windows 的动态段
> （本机从 1024 起）与上述保留段，也在 Linux 动态段（32768–60999）之外。
> 真绑不上时会打印实测可绑的替代端口，按提示 `--port` 即可。

**首次启动**会生成管理员账号并把初始密码打印一次，同时写入
`<data_dir>/initial-admin-password.txt`（首次登录成功后自动删除）：

```
首次启动管理员账号: admin ｜ 初始密码: ABCD-EFGH-JKMN-PQRS ｜ 已同时写入 …\initial-admin-password.txt
```

- 重启**不会**重新生成密码；会话存在 SQLite 里，7 天滑动续期，重启后仍然有效。
- 仍在使用初始密码时：浏览器顶部常驻横幅提示改密，**且服务端只放行改密/登出/自查**，
  其余接口一律 403；`--host 0.0.0.0 --allow-remote` 在改密前还会**拒绝启动**
  （初始密码会出现在日志与文件里）。
- `data_dir` 不可写 → 直接启动失败（身份数据不可缺省，不会退化成"内存模式"）。
- 同一 `data_dir` 只允许一个实例（`vtrim.lock`），第二个实例明确报错。
- 登录失败 5 次（同用户名 + 同 IP）后限流 15 分钟。
- **`--trust-proxy` 是 `X-Forwarded-*` 的唯一开关，默认关**：这两个头请求方可以随意伪造，
  关着的时候服务只认 socket 对端 —— 否则伪造 `X-Forwarded-For` 就能绕过按 IP 的限流，
  伪造 `X-Forwarded-Proto: https` 还会影响 cookie 的 `Secure` 判定。放在自己的反代后面时请打开。

**对外提供服务（公网/多租户）** 还需要反代 + HTTPS + 强口令，以及一份检查清单：
见 [`docs/deploy.md`](docs/deploy.md)。


### 管理员：从"服务器上的已有文件"新建项目

服务**不接受上传**。管理员（`role=admin`）在**首页的「新增项目 → 服务器文件」**区块里
**可以填任意绝对路径、浏览整台机器**（和以前的单用户 `vtrim init <video>` 一样）：

```
目录：F:/media/2025-09-12      ← 直接填，或从目录树逐级点进去
```

- 可以顺手在配置里放几个常用目录作为**快捷入口**（不是限制，只是下拉按钮）：

```toml
[storage]
projects_root = "D:/vtrim-projects"        # 项目托管目录 = 下载/入库的共同落点（默认 <exe_dir>/.vtrim/projects）
local_roots = ["F:/media", "D:/BililiveRecorder"]   # 可选：快捷入口
```

- **B站下载、录播姬入库、本机导入落的是同一个根**：`<projects_root>/users/<uid>/<房间或 BV 号>/<时间戳>-<pid>/`
  （没有单独的"下载目录"）。这一项也能在设置页「B站链接下载 → 项目存放位置」里改，
  改动**只影响新建项目**，已有项目仍在原位置，且改完立即生效、不用重启。

- 目标目录里**已有 `vtrim.toml`** 时只登记/打开，**不会覆盖**（保护手改过的工程设置）；
- 普通用户**只能用**自己的 `<projects_root>/users/<uid>`，`/api/fs/list` 与 `kind="local"`
  对非管理员一律 **403**；
- 仍然只读服务器已有文件：**永不接受上传**。

### `vtrim user` — 账号管理
| 子命令 | 作用 |
|--------|------|
| `list` | 列出账号（用户名/角色/状态/配额/最后登录） |
| `info` | 显示账号账本与初始密码文件位置 |
| `add <name> [--role admin\|user]` | 新建账号（不给密码则生成随机密码并只打印一次） |
| `passwd <name>` | 重置账号密码 |
| `check <name>` | 校验口令是否正确（诊断） |
| `disable <name> [--enable]` | 禁用/启用账号 |
| `init-admin` | 首次部署初始化管理员（幂等：已有账号则不动） |
| `reset-admin-password` | 救援：重置管理员密码（忘记密码时在本机执行） |

密码来源三选一：`--stdin`（管道）、`--password-file <f>`（推荐，不进进程列表）、
都不给则生成随机密码并打印一次。口令用 **scrypt**（`scrypt$log_n$r$p$salt$hash`
自描述哈希串）存储，将来可无损升级到 argon2id。

### `vtrim ingest` — 录播姬采集

录播姬（BililiveRecorder）录完一场直播就推送 webhook，服务端自动把录像变成项目并按策略跑管线。

> **也可以在 WebUI 里配**：设置页「录播姬（采集）」卡有开关 + `root` / `api_base` / `api_user` /
> `api_pass` / `allow_ips` / `auto_pipeline` 表单、一键生成/轮换 webhook token 与「测试连接」
> （保存即生效，不用重启）。命令行入口保留，两者改的是同一份配置。

```
vtrim ingest token            # 生成 webhook token（只显示一次），写入 [ingest] 并启用 webhook
vtrim ingest token --disable  # 只关闭 webhook（保留 token）
vtrim ingest status           # 采集配置概览（不打印秘密）
```

配置（`.vtrim/config.toml`）：

```toml
[ingest]
webhook_enabled = true
token_hash = "scrypt$…"          # 由 `vtrim ingest token` 写入；只存哈希
allow_ips = ["127.0.0.1"]        # 可选来源白名单（同机部署建议只放行本机）

[ingest.recorder]
root = "D:/BililiveRecorder"     # 录播姬输出目录（与 server 同机或同可读盘）
watch_interval_mins = 10         # 兜底扫描间隔（0=关闭）：漏推送/重启/历史录播导入
finalize_idle_mins = 5           # SessionEnded 后多久没有新文件就认为本场结束
move_into_managed = true         # 移入托管项目目录（同盘 rename，跨盘 copy+校验+删源）
concat_files = true              # 多分片合并为 source.mp4（原始分片保留）
extensions = ["flv", "mkv", "mp4", "ts"]
auto_pipeline = "detect"         # none | transcribe | detect | cut（房间映射可覆盖）
```

在录播姬的 Webhook 设置里填：

```
http://<本机IP>:26320/api/ingest/bililive/<token>
```

- 该端点**自带 token 鉴权、不要求登录**（录播姬只能填 URL，无法加自定义头）；
  收到事件只做"验 token → EventId 去重 → 落库"然后立即 2xx，重活交给后台入库 worker。
- 未映射的房间进「待分配池」，由管理员在 `#/ingest` 指派；指派后自动继续入库。
- 入库前做磁盘水位/配额预检，不足会明确拒绝（文案含"本次需要/磁盘可用/需保留底线"），**绝不自动删数据**。
  水位换算出的保留量被钳在 512 MiB ~ 8 GiB，所以"大容量但已用掉大半"的数据盘不会连几 MB 的入库都拒。
- 弹幕：录播姬写的 `.xml` 会被直接解析（无需 DanmakuFactory），随片搬进项目并写进 `vtrim.toml`。

---
只读诊断（不会创建账号库、不改配置）：

```
$ vtrim doctor [--url http://127.0.0.1:26320]
账号数据
  数据目录: …\cloud（剩余 24.3 GB / 共 251.2 GB · 可写 ✓）
  账号账本: …\vtrim.db（1 个账号）
    - admin（初始密码 · 管理员，从未登录）
  初始密码: 仍在使用 ⚠（对外监听会被拒绝启动）
  会话: 0 条未过期
  初始密码文件: 存在 ⚠ …
单实例锁
  …\vtrim.lock → 读不到内容（独占持有 = 有实例正在运行）
服务探测
  http://127.0.0.1:26320/api/health → 200（… listening=loopback，accounts=1 …）
建议 …
```

- **单实例锁**：`pid=0` = 上次正常退出；pid 已不存在 = 崩溃残留（下次启动自动接管）；
  读不到内容 = 有实例正独占持有（服务在运行）。
- **服务探测**：直接读 `/api/health`，能看出是否对外监听、是否走了 TLS 终止；
  探测不到就说明服务没在跑（默认端口 26320，可用 `--url` 改）。

---

### 视频精修 (`vtrim polish`)

精修引擎，基于 GSAP 为已剪切 clip 添加特效字幕、弹幕渲染、封面生成、预览。

所有子命令接受一个可选的 `<dir>` 参数，为 clip 目录 (默认当前目录)。精修项目需要 `config.toml` 和 `llm.toml` 位于该目录下。

| 子命令 | 作用 |
|--------|------|
| `init [<dir>]` | 初始化精修项目，创建 `config.toml` + `llm.toml` + `data/` + `sfx/`，自动解析素材 |
| `llm [<dir>]` | LLM 分析生成 segments 和关键词 (使用 `.vtrim/llm.toml`) |
| `process [<dir>]` | 后处理：解析 emoji/irasu keyword、下载资源、计算位置 |
| `render [<dir>]` | 渲染视频，自动先跑 process |
| `cover [<dir>] [--template <t>]` | 生成封面，可选模板: `left-text` / `center-text` / `right-text` / `minimal` |
| `preview [<dir>] [mode]` | 浏览器预览，自动先跑 process。mode: `video` / `cover` / 留空(双端口) |
| `doctor [<dir>]` | 系统检查：ffmpeg、Chrome、LLM 配置 |
| `test [<dir>]` | 测试模式，生成测试素材 + 预览 |

## 管线流程

| 阶段 | Stage # | 输入 | 输出 |
|------|---------|------|------|
| AudioExtractor | 1 | 视频 (mp4) | WAV (16kHz mono) |
| Transcriber | 2 | WAV | SRT + JSON |
| EventDetector | 3 | 转录 JSON + 弹幕 ASS | events.json |
| Cutter | 4 | 视频 + events.json | clips + 字幕 + publish_config |

Stage 编号连续无跳过。所有 stage 通过 `#[async_trait]` 实现 `Stage` trait。

### 状态机规则

`vtrim.toml` 中 `current_stage` 控制单 stage 命令：

| 规则 | 示例 (current=2) |
|------|-----------------|
| `stage > current + 1` → 拒绝 | `detect`(3=2+1) ✅ / `cut`(4>3) ❌ |
| `stage <= current` → 可重跑 | `transcribe`(2≤2) ✅ |
| 完成后 stage 自动推进 | `transcribe`(2) → `current_stage=2` |
| `process` 无视 state, 跑完设 `current_stage=4` | |

### 单个 Stage 命令运行模式

- `run()` — 受 `current_stage` 状态机约束，不能跳过
- `run_all()` — 无视状态机从头跑到底 (`process` 命令)
- `run_until(max)` — 跑到指定 stage 为止 (`process --dry-run`)

## 配置

双层配置合并：`vtrim.toml` (项目) → `.vtrim/config.toml` (全局，后加载覆盖前者)。无环境变量。

合并规则：String 字段 `!is_empty()` 时覆盖，数值字段 != 默认值时覆盖。

### `vtrim.toml` — 项目配置

```toml
[project]
input_file = ""              # 输入视频
danmaku_file = ""            # 弹幕文件
transcript_file = ""         # 转写文件 (由 transcribe 写入)
audio_file = ""              # 音频文件 (由 extract 写入)
events_file = ""             # 事件文件 (由 detect 写入)

[asr]
backend = "local"            # ASR 后端

[detection]
danmaku_window_secs = 10
# 标题+摘要 LLM 传播潜力评分权重（0 = 关闭；最终热度 = (1-w)*弹幕/字幕热度 + w*LLM评分）
llm_score_weight = 0.3
silence_threshold_secs = 3.0
min_segment_secs = 120
max_segment_secs = 900

[cutting]
output_dir = "./output"
overwrite = false
```

### `.vtrim/config.toml` — 全局配置

```toml
[asr]
model = "sensevoice"         # sensevoice(快) | qwen3-asr(最准,慢)；mode=hybrid 填词固定用 qwen3-asr
num_workers = 0              # 0=auto(CPU/4,上限4)
chunk_duration = 60.0        # 每块秒数
chunk_overlap = 10.0         # 块间重叠
segment_gap = 1.0            # 段间静音阈值(秒)
language = "zh"              # ASR 语言

[detection]
vtuber_name = ""             # 主播名称 (追加到标题末尾)
danmaku_window_secs = 10
llm_score_weight = 0.3       # 标题+摘要 LLM 评分权重；0=关闭

[cutting]
output_dir = "./output"
overwrite = false
```

### `.vtrim/llm.toml` — LLM 独立配置

```toml
base_url = ""                # 留空=OpenAI
api_key = ""
model = ""                   # 留空=使用各模块默认模型
temperature = 0
max_tokens = 65536
extra_params = { seed = 42, thinking = { type = "disabled" } }
```

## 输出

```
<output_dir>/
├── video.wav                # 音频
├── video.srt                # 字幕 (UTF-8 BOM)
├── video_llm.srt            # LLM 订正字幕 (如有)
├── transcript.json          # 结构化转写
├── events.json              # 检测的事件列表
├── publish_config.json      # 发布配置
└── clips/
    ├── clips_order.txt      # 按弹幕热度降序
    ├── clip-01/
    │   ├── clip.mp4
    │   ├── clip.srt         # 裁剪后字幕
    │   ├── clip.dm.srt      # 弹幕字幕
    │   └── clip.json        # 元数据
    └── clip-02/...
```

## 开发

```bash
cargo build                              # 0 warnings
cargo check -p vtrim-cli --all-targets   # 改 vtrim-server 后必跑（0 warnings）
cargo test -p vtrim-cli                  # vtrim-server 单测（JobManager / Range / 路由 404 …）
cargo test -p vtrim-core --lib           # 管线与配置单测
cargo run -- status                      # 查看当前进度
node webui/scripts/check-overlay.mjs     # 覆盖层色档契约：templates/index.html ↔ engine.js 对拍
node webui/scripts/check-caption-wrap.mjs    # 竖屏折行契约（wrapCaptionParts 两份实现）
node webui/scripts/check-sticker-anchor.mjs  # 贴纸锚点/尺寸契约（锚点=盒子左上、position=中心）
node webui/scripts/check-cover-stage.mjs     # 封面舞台契约（设计器黑边覆盖 + 恒横版画布 + 模糊数字同源）
cd webui && npm install && npm run build # 改 webui/src 后必须重建（dist 已入库）
cargo xtask package --smoke-test         # 打 Windows x64 发行包（含 espeak-ng 门禁 + 冒烟）
```

> 依赖方向是**单向**的：`vtrim-service` 绝不 `use vtrim_server::*`；`vtrim-server` 只依赖
> `vtrim-service`。细节与踩坑记录见各模块目录的 `AGENTS.md`（清单见
> [docs/module-map.json](docs/module-map.json) 与 [AGENTS.md](AGENTS.md) 的模块索引表）。

## 许可

**Apache License 2.0**（`Apache-2.0`），全文见 [LICENSE](LICENSE)。

这是一个**宽松许可**：允许商用、允许修改、允许**闭源再分发**。你的义务只有"保留声明"这一类。

### 为什么是 Apache-2.0（而不是 GPL）

本项目**曾经**必须以 **GPL-3.0-or-later** 发布：发行二进制经 `sherpa-onnx` 静态链接了
**espeak-ng**（GPL-3.0-or-later，是它预编译静态包内无条件链接的第三方），GPL 的传染性因此
落到整个二进制上。

**那个原因已经消除**。本项目只做 ASR、从不调用 TTS，改用
[TTS-off 预编译归档](docs/plan/sherpa-onnx-no-tts-switch-guide.md) 后，espeak-ng 被替换成
896 字节的**零符号占位库**，不再进入二进制。依赖审计（`cargo metadata --locked`，全树 335 个包）
确认**随二进制分发的 312 个包中没有任何 GPL / LGPL / AGPL**，逐条统计见
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)。

选 Apache-2.0 而非 MIT，是因为 §3 给出**明示的专利授权**（贡献者向你授予免专利费许可，
并附专利报复条款）、§6 明确**不授予商标权**，对企业使用场景更清晰。

> 此前声明的 `GPL-3.0-or-later` **从未产生过实际义务**：GPL 的义务只在**分发**（把副本交给
> 第三方）时触发，而本项目此前未向任何第三方分发过二进制——`v0.1.0` 只是内部测试版本。
> 因此本次是**一次性彻底切换**，不存在需要追溯的旧版本。

### 分发时你要做什么

| 场景 | 你需要做什么 |
|---|---|
| 本机自用 / 自己部署服务器 | 无需任何额外动作——Apache-2.0 的义务只在**分发**时触发。 |
| 把发行包发给别人 | 随包附 `LICENSE` 与 `THIRD-PARTY-NOTICES.md`（`cargo xtask package` 已**强制**打进去），并保留版权声明。 |
| 修改后分发 | 在改过的文件里**显著声明**你做了修改（§4(b)）。 |
| 并入闭源产品 | **允许**，无需公开你的源码。 |

> 上游组件里**没有任何带 `NOTICE` 文件的**，因此 Apache-2.0 §4(d) 的传递义务不触发。
> 但链接进来的 MIT / Apache-2.0 依赖仍要求保留其版权声明——这就是 `THIRD-PARTY-NOTICES.md` 的作用。

### 第三方组件

`ffmpeg` / `ffprobe` 由本项目以**外部命令**方式调用，既不随包分发也不与之链接，请自行安装并遵守其
自身许可（ffmpeg 官方构建的许可取决于其编译选项）。其余组件的授权与出处见
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)。

## 字体

发行包**不内置任何字体文件**，默认走系统字体栈
（`LXGW WenKai` → `Noto Sans SC` → `Microsoft YaHei`，最后兜底 `C:/Windows/Fonts/msyh.ttc`）。

**想换字体：把字体文件丢进程序目录的 `resources/fonts/`**（与 `vtrim.exe`、`.vtrim/`、`stickers/` 同级）：

```
<程序目录>/
  vtrim.exe
  .vtrim/
  stickers/
  resources/fonts/          ← 放 .ttf / .otf / .ttc，丢进去就会出现
```

放好之后到精修页：

- **字幕字体** —— 「生成与导出」卡 → LLM 剪辑参数块 → 「字幕字体」下拉（按片段保存）；
- **封面字体** —— 封面设计器 → 「排版」组 → 「字体」下拉。

下拉里列出的是**字体库里的文件**（显示名从字体自带的字族名读取，例如 `黑体（simhei）`），
另有一项「（系统默认）」回到上面的系统字体栈。选完**保存即生效**，不用重跑任何任务：
预览与"渲染导出"出的成片都会用这个字体。库里没有新字体时，下拉下面会直接写出字体库的完整路径。

> **想要圆润可爱的标题风格？** 可以自行安装或下载 **乐米波波体**（LemiBoBoTi，杭州乐米网络科技）。
> 官方授权地址：<https://font.leminet.cn/>。
> 注意它的授权是"**免费商用**"但**不包括嵌入式用途** —— 商用需按其要求取得免费商用授权书，
> 把字体文件内嵌进软件或 APP 则必须另行授权。**本项目因此不随包分发该字体**，
> 请你自行获取、自己放进 `resources/fonts/` 并遵守其条款。

