# V-Trim

直播录像自动切片工具。输入录播视频 + 弹幕文件，输出按事件分割的片段 + 字幕 + `publish_config.json`。

## 前置条件

- **ffmpeg** + **ffprobe** — 需在 PATH 中，用于音频提取和视频剪切
- **Rust** 工具链 (构建用)

## 项目结构

```
.
├── crates/               # 源码
│   ├── vtrim-core/       #   管线编排 + 业务逻辑
│   ├── vtrim-cli/        #   CLI 入口 (clap)
│   └── vtrim-polish/     #   GSAP 精修引擎
├── docs/archived/        #   历史设计文档
├── AGENTS.md
├── Cargo.toml / Cargo.lock
└── vtrim.toml            # 项目配置 (gitignored)
```

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
- 弹幕文件存在 → 计算 10s 窗口密度，输出 `popularity` (0~1) + `rank`
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
| `--duration <sec>` | 静音阈值(秒)，默认 `2.0`。超过此长度的连续静音将被写入屏蔽轨道 |
| `--threshold <dB>` | 音频分析噪声门限，默认 `-30`（仅在缺少字幕文件时使用） |

未初始化的切片会先自动 `polish init`（映射到屏蔽轨道需要有 `polish.toml`）。
记录写入 `silence_ranges`，编辑器里的「撤销去静音」按它精确回退，手动添加的屏蔽区间不受影响。

静音检测策略：如有同名 `.srt` 文件，优先分析 SRT 字幕间隔；否则用 ffmpeg silencedetect filter。

### `vtrim download`

下载全部 ASR 模型到 `.vtrim/models/<model>/`：`sensevoice`（默认）、`firered-zh`、`qwen3-asr`（各 ~165MB / ~520MB / ~879MB）。转写时按 `[asr] model` 自动懒加载，缺失才提示下载。

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
| `--port <n>` | 监听端口，默认 `7788` |
| `--open` | 启动后自动打开浏览器 |
| `--config <file>` | 打开指定工程（默认恢复上次工程） |
| `--allow-remote` | 允许监听非 loopback 地址（默认拒绝） |
| `--data-dir <dir>` | 账号账本目录，默认 `<exe_dir>/.vtrim/cloud`（也可用 `VTRIM_DATA_DIR`） |
| `--admin-password-file <file>` | 无人值守部署：用该文件里的密码作为初始管理员密码 |
| `--force` | 接管残留的 `vtrim.lock`（确认上一个实例已崩溃时使用） |

**首次启动**会生成管理员账号并把初始密码打印一次，同时写入
`<data_dir>/initial-admin-password.txt`（首次登录成功后自动删除）：

```
首次启动管理员账号: admin ｜ 初始密码: ABCD-EFGH-JKMN-PQRS ｜ 已同时写入 …\initial-admin-password.txt
```

- 重启**不会**重新生成密码；会话存在 SQLite 里，7 天滑动续期，重启后仍然有效。
- 仍在使用初始密码时，浏览器顶部常驻横幅提示改密；`--host 0.0.0.0 --allow-remote`
  在改密前会**拒绝启动**（初始密码会出现在日志与文件里）。
- `data_dir` 不可写 → 直接启动失败（身份数据不可缺省，不会退化成"内存模式"）。
- 同一 `data_dir` 只允许一个实例（`vtrim.lock`），第二个实例明确报错。
- 登录失败 5 次（同用户名 + 同 IP）后限流 15 分钟。

### 管理员：从"服务器上的已有文件"新建项目

服务**不接受上传**。管理员（`role=admin`）在网页侧栏点「＋新建项目 → 从服务器文件」，
**可以填任意绝对路径、浏览整台机器**（和以前的单用户 `vtrim init <video>` 一样）：

```
目录：F:/media/2025-09-12      ← 直接填，或从目录树逐级点进去
```

- 可以顺手在配置里放几个常用目录作为**快捷入口**（不是限制，只是下拉按钮）：

```toml
[storage]
projects_root = "D:/vtrim-projects"        # 托管项目目录（默认 <exe_dir>/.vtrim/projects）
local_roots = ["F:/media", "D:/BililiveRecorder"]   # 可选：快捷入口
```

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
http://<本机IP>:7788/api/ingest/bililive/<token>
```

- 该端点**自带 token 鉴权、不要求登录**（录播姬只能填 URL，无法加自定义头）；
  收到事件只做"验 token → EventId 去重 → 落库"然后立即 2xx，重活交给后台入库 worker。
- 未映射的房间进「待分配池」，由管理员在 `#/ingest` 指派；指派后自动继续入库。
- 入库前做磁盘水位/配额预检，不足会明确拒绝（文案含"预计需要/可用"），**绝不自动删数据**。
- 弹幕：录播姬写的 `.xml` 会被直接解析（无需 DanmakuFactory），随片搬进项目并写进 `vtrim.toml`。

---
只读诊断（不会创建账号库、不改配置）：

```
$ vtrim doctor [--url http://127.0.0.1:7788]
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
  http://127.0.0.1:7788/api/health → 200（… listening=loopback，accounts=1 …）
建议 …
```

- **单实例锁**：`pid=0` = 上次正常退出；pid 已不存在 = 崩溃残留（下次启动自动接管）；
  读不到内容 = 有实例正独占持有（服务在运行）。
- **服务探测**：直接读 `/api/health`，能看出是否对外监听、是否走了 TLS 终止；
  探测不到就说明服务没在跑（默认端口 7788，可用 `--url` 改）。

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
model = "sensevoice"         # sensevoice(快) | firered-zh(较准) | qwen3-asr(最准,慢)
num_workers = 0              # 0=auto(CPU/4,上限4)
chunk_duration = 60.0        # 每块秒数
chunk_overlap = 10.0         # 块间重叠
segment_gap = 1.0            # 段间静音阈值(秒)
language = "zh"              # ASR 语言

[detection]
vtuber_name = ""             # 主播名称 (追加到标题末尾)
danmaku_window_secs = 10

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
cargo build                    # 0 warnings
cargo test --lib danmaku       # ASS 解析 + 密度计算测试
cargo run -- status            # 查看当前进度
```

## 许可

MIT
