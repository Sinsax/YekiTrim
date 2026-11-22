# 命令行参考

`yekitrim` 是本仓唯一的可执行文件；本页是它全部子命令的参数、输出与踩坑提示。
构建方式与部署见 [README](../README.md) 与 [deploy.md](deploy.md)。

## 目录

- [全局选项](#全局选项)
- [管线](#管线)：`init` · `extract` · `transcribe` · `correct` · `detect` · `cut` · `process` · `download` · `status`
- [精修](#精修)：`silence` · `polish` · `polish render|cover|preview` · `polish stickers`
- [服务与账号](#服务与账号)：`server` · 从服务器文件新建项目 · `user` · `ingest` · `config`
- [字体](#字体)：`fonts list` · `fonts install`
- [诊断](#诊断)：`doctor` · `usage` · `-V/--version`

## 全局选项

| 选项 | 作用 |
|---|---|
| `-c, --config <path>` | 指定配置文件路径（默认 `./yekitrim.toml`），对所有子命令生效 |
| `-h, --help` | 每个子命令都支持 |

⚠️ **`init` 是 `-c` 的例外**：它自己的 `--config` 是 `yekitrim.toml` 的**写出路径**（缺省 = 输入视频同级目录），不读全局 `cli.config`。但两者在 clap 里**是同一个 id**，所以 `-c` 写在**子命令之前**也会被 init 采用（实测 `yekitrim -c A.toml init v.mp4` ⇒ 写到 `A.toml`；子命令自己的 `--config` 优先）。`init` **之后**的 `-c` 会被 clap 直接拒绝（那个位置只有 `--config`）。其余所有分支一律把全局 `cli.config` 当**读**的配置路径。

## 管线

### `yekitrim init <video>`

创建 `yekitrim.toml` 工程配置。

| 参数 | 作用 |
|---|---|
| `<video>` | 输入视频文件路径 **（必填）**；扩展名只收 `mp4` / `mkv` / `mov` / `avi` / `flv` / `webm` / `mts` / `m2ts`，传目录直接报错 |
| `--danmaku <file>` | 弹幕文件（ASS 格式）。不传则自动找视频同名 `.ass`，存在就写进配置 |
| `-o, --output <dir>` | 输出目录，默认 `./output` |
| `--config <path>` | `yekitrim.toml` 的写出路径，缺省 = **输入视频同级目录**的 `yekitrim.toml` |
| `--force` | 目标配置已存在时**才**允许重建（先备份成 `yekitrim.toml.bak-<yyyymmdd-hhmmss>`） |

⚠️ **目标 `yekitrim.toml` 已存在时默认拒绝覆盖**：`init` 是整份重写，会把素材清单（`[project.materials]`，也就是 ASR 槽位）、`transcript_file`、`[cutting] output_dir` 一起换回模板值 —— `output_dir` 一变，既有产物在工程视角里直接消失。要重建必须显式加 `--force`（会先备份原文件）。

### `yekitrim extract`

Stage 1 — 从视频提取 16 kHz mono WAV。

| 参数 | 作用 |
|---|---|
| `-i, --input <file>` | 输入视频（默认 `yekitrim.toml` 的 `input_file`） |
| `-o, --output <dir>` | 输出目录（默认 `yekitrim.toml` 的 `output_dir`） |

- 输出 `<output_dir>/video.wav`，自动写入 `yekitrim.toml` 的 `audio_file`。
- 状态机：`current_stage >= 0` 即可运行。

### `yekitrim transcribe`

Stage 2 — 语音转文字（本地 sherpa-onnx SenseVoice）。

| 参数 | 作用 |
|---|---|
| `-i, --input <file>` | 输入音视频（默认 `input_file`，自动回退 `audio_file`） |
| `-o, --output <dir>` | 输出目录（默认 `yekitrim.toml` 的 `output_dir`） |

- 输出 `<output_dir>/video.srt`（UTF-8 **BOM**）与 `<output_dir>/transcript.json`（结构化），并把 `transcript_file` 写回配置。
- 并行策略：CPU 核数 / 4（上限 4），每 chunk 的 ONNX intra-op threads = 2。
- 程序配置 `[asr]` 可调：`num_workers`（默认 `0` = auto，手动值不超过 CPU 核数）、`chunk_duration`（`60.0` 秒/块）、`chunk_overlap`（`10.0` 秒重叠）、`segment_gap`（`1.0` 秒，段间静音超过才切成两条字幕）、`language`（`"zh"`）。
- 状态机：需先完成 `extract`（`current >= 1`）。

### `yekitrim correct`

LLM 订正转录文本。

| 参数 | 作用 |
|---|---|
| `-t, --transcript <file>` | 转录 JSON / SRT 文件 |

- 自动发现优先级：`--transcript` > `yekitrim.toml` 的 `transcript_file` > `<output_dir>/transcript.json`。
- 输出 `<output_dir>/video_llm.srt`，并把 `transcript_file` 改指向这份订正稿。
- 需要 `.yekitrim/llm.toml` 已配置 API key。**无状态机约束**，可独立运行。

### `yekitrim detect`

Stage 3 — LLM 分析转写结果，检测高价值事件。

| 参数 | 作用 |
|---|---|
| `-t, --transcript <file>` | 转录 JSON / SRT 文件 |

- 自动发现优先级：`--transcript` > `transcript_file` > `<output_dir>/transcript.json` > `<output_dir>/video.srt`。
- 输出 `<output_dir>/events.json`（EventStore 格式），并写入 `yekitrim.toml` 的 `events_file`。
- 检测逻辑：LLM API key 非空 → 单次调用分析**全部**转录段（64k+ 上下文）；为空 → 降级成单个默认事件（覆盖整段视频）。
- 传播潜力：仅凭「标题 + 概括」让 LLM 按预测点击率 / 有趣程度 / 爆点强度打分，再与弹幕 / 字幕热度按 `detection.llm_score_weight` 加权，输出 `popularity`（0~1）+ `rank`；无弹幕时基础热度只用字幕密度，LLM 评分失败或关闭时回退基础热度。`detection.vtuber_name` 非空则（在**产物标题**里）追加 `【名】` —— 它**不参与切片**、不参与目录命名（2026-11-22 口径：名字填错不该逼着重切整项目）。
- 状态机：需先完成 `transcribe`（`current >= 2`）。

### `yekitrim cut`

Stage 4 — 按事件片段剪切视频 + 生成字幕。

| 参数 | 作用 |
|---|---|
| `-i, --input <file>` | 输入视频（默认 `yekitrim.toml` 的 `input_file`） |
| `-e, --events <file>` | 事件 JSON 文件 |
| `-o, --output <dir>` | 输出目录（默认 `yekitrim.toml` 的 `output_dir`） |

- 事件自动发现优先级：`--events` > `events_file` > `<output_dir>/events.json`。
- 编码器自动检测顺序：`h264_nvenc` → `h264_qsv` → `h264_amf` → `libx264`；字幕从原始 SRT 裁剪时间范围并偏移到 clip 起始时间。

```
<output_dir>/clips/
├── clip-01/{clip.mp4, clip.srt, clip.dm.srt, clip.json}   # 片段/裁剪字幕/弹幕字幕(有弹幕时)/元数据
├── clip-02/...
├── clips_order.txt        # 按弹幕热度降序排列
└── publish_config.json    # 发布配置
```

- 状态机：需先完成 `detect`（`current >= 3`）。

### `yekitrim process`

全自动管线处理。⚠️ **没有 `--stages` 参数**（旧文档写过，实际 clap 里不存在），真实参数只有下面三个：

| 参数 | 作用 |
|---|---|
| `-i, --input <file>` | 输入视频（默认 `yekitrim.toml` 的 `input_file`） |
| `-o, --output <dir>` | 输出目录（默认 `yekitrim.toml` 的 `output_dir`） |
| `--dry-run` | 仅分析不剪切，跑到 EventDetector（Stage 3）为止 |

- **语义是「补齐未完成的阶段」，不是「无视状态机从头跑」**：`process` 没有状态机门槛，但 `current_stage` 已经覆盖到的阶段一律按「已存在，跳过」，已有产物的阶段**不重跑**。改完 prompt / 模型想重跑，用对应的单级命令（`transcribe` / `detect` / `cut`）。
- 流程（每一步都是"如未完成"）：① 音频提取 → ② 语音转写（自动恢复已有 `transcript.json` / `video.srt`）→ ③ LLM 订正（`asr.llm_correct = true` 且配了 LLM key、且事件尚未检测）→ ④ 事件检测 → ⑤ 视频剪切（`--dry-run` 跳过）。
- ⚠️ `-c` 指错文件会**在这一步就失败**（预检），不会静默落到全局默认值上继续跑。

### `yekitrim download`

下载全部 ASR 模型到 `<数据目录>/.yekitrim/models/<model>/`：`sensevoice`（~165 MB，single 模式 + hybrid 打轴）、`qwen3-asr`（~879 MB，hybrid 填词）。转写时按 `[asr] model` / `[asr] mode` 自动懒加载，缺失才提示下载。

运行时找模型只有两档：**配置 `local_model_dir` → `<数据目录>/.yekitrim/models`**（默认数据目录 = exe 同级；模型管理页显示的就是实际用到的那一份）。

### `yekitrim status`

显示当前处理进度：

```
视频工程: input.mp4
当前进度: Stage #2

  [✓] AudioExtractor
  [✓] Transcriber
  [→] EventDetector
  [·] Cutter
```

- `current_stage` = **最后跑完的** stage（不落盘，由输出文件存在性推导）。标记口径：`num <= current` → `✓`，`num == current + 1` → `→`，其余 `·`。`output/transcript.json` 里有 `asr_metadata` 时追加打印模型 / 锚点数 / 锚点覆盖率 / 最大插值跨度 / 降级原因。
- ⚠️ **工程配置不存在时报错并非零退出**（不会伪装成"全新工程"）：找错目录会被如实报出来，脚本里 `status && 下一步` 也跟着正确。

## 精修

### `yekitrim silence [files...]`

把已输出切片中的长停顿映射为**屏蔽轨道**（写入 `polish.toml` 的 `mute_ranges`），不生成新视频文件，原始 `clip.mp4` / `clip.srt` 保持不变；导出时由 process / render 剪掉。精修编辑器时间轴顶部的「去静音」快捷按钮等价于此命令。

| 参数 | 作用 |
|---|---|
| `[files...]` | 要处理的视频或目录。不传则扫描 `<cwd>/output/clips/*/` |
| `--duration <sec>` | 静音阈值（秒），默认 `3.5`。超过此时长的连续静音才写进屏蔽轨道（3.5 s 以内的字幕间隔一律不处理） |
| `--threshold <dB>` | 音频分析噪声门限，默认 `-30`（**仅在缺少字幕文件时**使用） |

- 未初始化的切片会先自动 `polish init`（映射到屏蔽轨道需要有 `polish.toml`）。
- 检测结果**并入**当前屏蔽轨道（与已有屏蔽相加：重叠 / 相邻的合成一段）；每段静音两侧各留 **0.8 s 气口**（字幕时间戳有误差，贴边剪会削掉首尾音节；实际剪掉的是 `间隔 − 1.6 s`）。**字幕间隔 3.5 s 以内一律不动**（默认阈值，正常语气停顿就这个量级，剪了反而把对话切碎）。
- 自动加进去的那部分记在 `silence_ranges`，编辑器的「撤销去静音」按它精确相减回退；手动添加的屏蔽区间不受影响。检测策略：有同名 `.srt` 就优先分析字幕间隔，否则用 ffmpeg `silencedetect`。

### `yekitrim polish <子命令>`

精修引擎（GSAP 特效字幕 / 弹幕 / 封面 / 预览）。所有子命令接受一个可选的 `<dir>`（clip 目录，默认当前目录）；精修工程需要该目录下有 `config.toml` 与 `llm.toml`。

| 子命令 | 作用 |
|---|---|
| `init [<dir>]` | 初始化精修项目：创建 `config.toml` + `llm.toml` + `data/` + `sfx/`，自动解析素材 |
| `llm [<dir>]` | LLM 分析生成 segments 与关键词（用 `.yekitrim/llm.toml`） |
| `process [<dir>]` | 后处理：解析 emoji / irasu keyword、下载资源、计算位置 |
| `render [<dir>]` | 渲染视频，自动先跑 `process`（详见下一节） |
| `cover [<dir>]` | 生成封面（详见下一节） |
| `preview [<dir>] [mode]` | 浏览器预览，自动先跑 `process`（详见下一节） |
| `doctor [<dir>]` | 系统检查：ffmpeg、底座（Dhampir）、LLM 配置 |
| `test [<dir>]` | 测试模式，生成测试素材 + 预览 |
| `stickers <子命令>` | 本地贴纸工具（见下） |

- `polish llm` 在 CLI 走**断点续传**（长片重跑很贵）；编辑器那颗按钮走全量重做。

### `yekitrim polish render` / `cover` / `preview`

| 子命令 | 参数 | 产出 |
|---|---|---|
| `render` | `[<dir>]` `--renderer <name>`（默认 `dhampir`）`--strict` | 成片 |
| `cover` | `[<dir>]` `--template <t>` `--renderer <t>`（`takumi`） | 封面图 |
| `preview` | `[<dir>]` `[mode]`（`video` / `cover` / 留空=双端口） | 浏览器预览 |

- 封面模板可选：`left-text` / `center-text` / `right-text` / `minimal`。
- **出片只剩底座这一条腿**：`render --renderer` 的 `dhampir` 是**唯一一个值**，老写法 `wgpu` / `native` 视作 `dhampir`；`chrome` 是本仓那条**已整条删除**的 headless 兜底腿 —— 传 `chrome` 会**明确报错**，不会静默改走底座。认不出的值由 `pipeline.rs` 一处报错（校验不在 `cli.rs` 里再抄一份）。后端经 `VTEDIT_RENDERER` 传下去。
- `--strict`：转译报告里出现 `DROP`（参照有、底座表达不了）就**拒绝出片**（走 `VTEDIT_STRICT=1`）。没装 Node / 底座不可用时是**明确报错**，没有第二条路可退；缺什么看 `polish doctor` 的 `[dhampir]` 块。
- ⚠️ **封面与视频的出片通道互相独立**：视频走底座出片，封面仍走 Takumi（静态光栅化）。`cover --renderer` 的 `takumi` 也是**唯一一个值**，老脚本里的 `chrome` 同样会**明确报错**；不传时走 `cover.rs` 的默认链（显式参数 > `VTEDIT_COVER_RENDERER` > `takumi`）。
- `preview` 有两条路：先在输入目录向上找 `yekitrim.toml` —— 找到就起 `yekitrim server` 打开同一个 WebUI 精修页；找不到才回退旧的独立预览服务器。目标页是 `/#/p/<项目 id>/polish[/<片段目录名>]`，工程还没登记进项目库时退回 `/#/workbench` 并打一条 info。⚠️ 它**仍然要登录**（免登录只是托盘那条路，见 [server](#yekitrim-server)）。

### `yekitrim polish stickers <子命令>`

| 子命令 | 作用 |
|---|---|
| `list` | 列出本地贴纸索引（组名 + 成员数 + tags） |
| `test <keyword>...` | 测 keyword → 贴纸匹配，可给多个；无命中时列出 3 个候选及分数 |
| `scan [--llm]` | 扫描本地贴纸目录，生成 / 更新 `stickers.json` 清单；`--llm` 用 LLM 批量生成英文 tags |
| `build <package>` | 预处理贴纸包，生成 `<包名>.toml` 候选 tags 清单 |

- 贴纸库按 owner 分：单人 / 托盘形态就在 `<数据目录>/users/<uid>/resources/stickers/`。
- ⚠️ `scan --llm` 没配 LLM key 会**报错**（用户明确要了 `--llm`，静默留空等于"看起来成功、实际没做"）。
- 内建 emoji 分类**没有 CLI 面**（分类是程序资产，不给用户重建 / 更改），别去找对应的子命令。

## 服务与账号

### `yekitrim server`

启动浏览器可视化服务，服务 `webui/dist` 构建产物。

| 参数 | 默认 | 作用 |
|---|---|---|
| `--host <addr>` | `127.0.0.1` | 监听地址 |
| `--port <n>` | `26320` | 监听端口 |
| `--open` | 关 | 启动后自动打开浏览器 |
| `--config <file>` | 恢复上次工程 | 打开指定工程 |
| `--webui-dir <dir>` | 按构建形态 | 覆盖 WebUI 静态资源目录（等价 `YEKITRIM_WEBUI_DIR`，命令行优先） |
| `--allow-remote` | 关 | 允许监听非 loopback 地址 |
| `--trust-proxy` | 关 | 采信反代转发的 `X-Forwarded-For` / `X-Forwarded-Proto` |
| `--data-dir <dir>` | `<exe_dir>/.yekitrim/cloud` | 账号账本 / 会话 / 初始密码文件所在目录（也可用 `YEKITRIM_DATA_DIR`） |
| `--admin-password-file <file>` | 无 | 无人值守部署：用该文件里的密码作为初始管理员密码（不进进程列表） |
| `--local-no-auth` | 关 | 本地免登录，**仅 loopback**（托盘那条路） |
| `--force` | 关 | 接管残留的 `yekitrim.lock`（确认上一个实例已崩溃时用） |

**服务形态下认证不可关闭**：除 `/api/health` 与 `/api/auth/login` 外，**所有**接口都要求会话，包括从 `127.0.0.1` 访问 —— 单人使用 = 只有一个 `admin` 账号的部署，同样要登录。唯一的例外是给桌面托盘用的 `--local-no-auth`（见下），它只允许绑 loopback。

**首次启动**会生成管理员账号并把初始密码打印一次，同时写入 `<data_dir>/initial-admin-password.txt`（首次登录成功后自动删除）：

```
首次启动管理员账号: admin ｜ 初始密码: ABCD-EFGH-JKMN-PQRS ｜ 已同时写入 …\initial-admin-password.txt
```

- 重启**不会**重新生成密码；会话存在 SQLite 里，7 天滑动续期，重启后仍然有效。
- 仍在使用初始密码时：浏览器顶部常驻横幅提示改密，**且服务端只放行改密 / 登出 / 自查**，其余接口一律 403；`--host 0.0.0.0 --allow-remote` 在改密前还会**拒绝启动**（初始密码会出现在日志与文件里）。
- `data_dir` 不可写 → 直接启动失败（身份数据不可缺省，不会退化成"内存模式"）；同一 `data_dir` 只允许一个实例（`yekitrim.lock`），第二个实例明确报错。
- 登录失败 5 次（同用户名 + 同 IP）后限流 15 分钟。
- `--local-no-auth` 是给"双击就用的桌面形态"（`yekitrim-tray`）用的：不校验会话，所有请求都当作本机管理员，代价是**同机上任何进程**都能调这套 API。因此只允许绑 loopback，并另加两道闸：`Host` 必须是 loopback 主机名（防 DNS rebinding）、写请求的 `Origin` / `Sec-Fetch-Site` 必须同源（防跨站表单）。⚠️ 对外服务（`--allow-remote` / 非 loopback）一律拒绝启动，别拿它当"免密后台"。
- **端口为什么是 26320**：老默认 7788 在部分 Windows 上落进了 Hyper-V/WSL 的**保留端口段**（实测 7635–7834），表现是 `10013 WSAEACCES` 绑不上，而 `netstat` 里一个监听者都没有（所以不是"被占用"）。26320 落在跨平台安全带 **15001–32767** 内：既躲开 Windows 的动态段（本机从 1024 起）与上述保留段，也在 Linux 动态段（32768–60999）之外。真绑不上时会打印实测可绑的替代端口，按提示 `--port` 即可。
- `--trust-proxy` 是 `X-Forwarded-*` 的**唯一开关，默认关**：这两个头请求方可以随意伪造，关着的时候服务只认 socket 对端 —— 否则伪造 `X-Forwarded-For` 就能绕过按 IP 的限流，伪造 `X-Forwarded-Proto: https` 还会影响 cookie 的 `Secure` 判定。放在自己的反代后面时请打开。
- WebUI 来源：**release 内嵌**（`webui/dist` 编进 exe，升级只需换 exe）、**debug 读盘**（改前端不必重链接），`--webui-dir` / `YEKITRIM_WEBUI_DIR` 两种形态都能覆盖。

对外提供服务（公网 / 多租户）还需要反代 + HTTPS + 强口令，以及一份检查清单：见 [deploy.md](deploy.md)。

### 从服务器上的已有文件新建项目（WebUI，没有对应子命令）

服务**不接受上传**。管理员（`role=admin`）在**首页的「新增项目 → 服务器文件」**区块里**可以填任意绝对路径、浏览整台机器**（和以前的单用户 `yekitrim init <video>` 一样）：

```
目录：F:/media/2025-09-12      ← 直接填，或从目录树逐级点进去
```

- 可以顺手在配置里放几个常用目录作为**快捷入口**（不是限制，只是下拉按钮）：

```toml
[storage]
projects_root = "D:/yekitrim-projects"        # 项目托管目录 = 下载/入库的共同落点（默认 <exe_dir>/.yekitrim/projects）
local_roots = ["F:/media", "D:/BililiveRecorder"]   # 可选：快捷入口
```

- **B站下载、录播姬入库、本机导入落的是同一个根**：`<projects_root>/users/<uid>/<房间或 BV 号>/<时间戳>-<pid>/`（没有单独的"下载目录"）。这一项也能在设置页「B站链接下载 → 项目存放位置」里改，改动**只影响新建项目**，已有项目仍在原位置，且改完立即生效、不用重启。
- 目标目录里**已有 `yekitrim.toml`** 时只登记 / 打开，**不会覆盖**（保护手改过的工程设置）。
- 普通用户**只能用**自己的 `<projects_root>/users/<uid>`；`/api/fs/list` 与 `kind="local"` 对非管理员一律 **403**。仍然只读服务器已有文件：**永不接受上传**。

### `yekitrim user <子命令>`

| 子命令 | 作用 |
|---|---|
| `list` | 列出账号（用户名 / 角色 / 状态 / 配额 / 最后登录） |
| `info` | 显示账号账本与初始密码文件位置 |
| `add <name> [--role admin\|user]` | 新建账号（不给密码则生成随机密码并只打印一次） |
| `passwd <name>` | 重置账号密码 |
| `check <name>` | 校验口令是否正确（诊断） |
| `disable <name> [--enable]` | 禁用 / 启用账号 |
| `init-admin` | 首次部署初始化管理员（幂等：已有账号则不动） |
| `reset-admin-password` | 救援：重置管理员密码（忘记密码时在本机执行） |
| `invite [--note <s>] [--days N] [--count N] [--disable]` | 生成注册邀请码并**开启注册**；`--disable` 只关注册 |
| `invites` | 列出邀请码（只见脱敏 hint）与注册开关状态 |
| `invite-revoke <id\|selector>` | 撤销一个邀请码 |

- 密码来源三选一：`--stdin`（管道）、`--password-file <f>`（推荐，不进进程列表）、都不给则生成随机密码并打印一次。`add` / `passwd` / `check` / `init-admin` / `reset-admin-password` 都支持这两个参数。口令用 **scrypt** 存储（`scrypt$log_n$r$p$salt$hash` 自描述哈希串），将来可无损升级到 argon2id。
- 邀请码与 `ingest token` 的规矩**相反**：明文**落库、可重复查看**（管理员要能复制再发），所以 `--count` 一次生成 1..=50 个、`--days` 不给就不过期、一个码只能绑一个账号（用过即失效，列表里标「已使用」并显示绑给了谁）。
- `invite` 会**自动打开注册开关**（`[auth] registration_enabled = true`）；`--disable` 关闭注册但**保留已生成的码**。被邀请人访问 `<服务地址>/#/register`，填用户名 / 密码 / 邀请码即可注册（角色固定 `user`）。

### `yekitrim ingest <子命令>`

录播姬（BililiveRecorder）录完一场就推送 webhook，服务端自动把录像变成项目并按策略跑管线。

| 子命令 | 作用 |
|---|---|
| `token` | 生成 webhook token（**只显示一次**），写入 `[ingest]` 并开启 webhook |
| `token --disable` | 只关闭 webhook（保留 token） |
| `status` | 采集配置概览（不打印任何秘密） |

配置在程序配置 `config.toml` 里：

```toml
[ingest]
webhook_enabled = true
token_hash = "scrypt$…"          # 由 `yekitrim ingest token` 写入；只存哈希
allow_ips = ["127.0.0.1"]        # 可选来源白名单（同机部署建议只放行本机）

[ingest.recorder]
kind = "bililive"                # bililive(录播姬) | bililive-tools | none(关闭)；其它值一律按录播姬
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

- 该端点**自带 token 鉴权、不要求登录**（录播姬只能填 URL，无法加自定义头）；收到事件只做"验 token → EventId 去重 → 落库"然后立即 2xx，重活交给后台入库 worker。
- 未映射的房间进「待分配池」，由管理员在 `#/ingest` 指派；指派后自动继续入库。
- 入库前做磁盘水位 / 配额预检，不足会明确拒绝（文案含"本次需要 / 磁盘可用 / 需保留底线"），**绝不自动删数据**。水位换算出的保留量被钳在 **512 MiB ~ 8 GiB**，所以"大容量但已用掉大半"的数据盘不会连几 MB 的入库都拒。
- 弹幕：录播姬写的 `.xml` 会被直接解析（无需 DanmakuFactory），随片搬进项目并写进 `yekitrim.toml`。
- 也可以在 WebUI 里配：设置页「录播姬（采集）」卡有开关 + `root` / `api_base` / `api_user` / `api_pass` / `allow_ips` / `auto_pipeline` 表单、一键生成 / 轮换 token 与「测试连接」（保存即生效，不用重启）。命令行与网页改的是同一份配置。

### `yekitrim config init|show|migrate`

| 子命令 | 作用 |
|---|---|
| `init [--path <path>]` | 生成程序级配置（**覆盖写**，已有配置会被替换） |
| `show` | 显示合并后的完整配置（`yekitrim.toml` + 程序配置） |
| `migrate [路径…]` | 把已下线的键从配置文件里**真清掉并写回** |

- ⚠️ `config init` 不带 `--path` 时写的是**程序级全局配置** `global_config_path()` = `<数据目录>/users/<uid>/.yekitrim/config.toml`，**不是**项目 `./yekitrim.toml`（`--help` 里那句"默认 ./yekitrim.toml"是旧文案，以本条为准；`--path` 可改）。同目录的 `llm.toml` 一起生成，顺带建好贴纸库 / 字体库并装好自带字体。
- `migrate` 不给路径 = 只处理**程序级**那两个（`config.toml` / `llm.toml`）；给目录 → 认该目录的 `yekitrim.toml` / `polish.toml`；给文件 → 只处理该文件。清之前每个文件都留 `.bak-v<旧版本>`（原样回退的唯一手段）；文件不存在打印"跳过"；**有任一文件失败就非零退出**。
- ⚠️ 为什么需要 `migrate`：工程级配置（`yekitrim.toml` / `polish.toml`）按设计**只在内存里升、不写回磁盘**（工程可能几百个、还可能在网络盘上），那几个已下线的键会永远留在文件里、每次读都重新发现它；这条命令是"按一下就真的清掉"的出口。
- `yekitrim server` 首次启动时若发现程序配置缺失，会自动补齐这两个文件（**只创建缺失的**，已存在的不会被覆盖）；已存在的文件 TOML 语法错误（整份配置都会被忽略）时会先备份为 `config.toml.broken` 再重建，并在启动日志里给出原因。

## 字体

### `yekitrim fonts list|install`

| 子命令 | 作用 |
|---|---|
| `list` | 列出字体库：id / **内部族名** / 显示名 / 体积 / 标签（自带 · 默认 · emoji · 未安装） |
| `install [--id <id>]` | 装上**自带字体**（把内嵌字节写进字体库；幂等、不联网；`--id` 只装某一支） |

- 字体库是**程序级资产**（不属于某个精修工程），所以是顶层命令。库分两层：用户库 `<数据目录>/users/<uid>/resources/fonts/`（`Local` = 当前管理员），自带字体装在**公用库** `<数据目录>/global/resources/fonts/`；`list` 两个库都看，所以自带字体不会被误报成"未安装"（它会把两条路径都打印出来）。
- 两支自带字体都是 **SIL OFL-1.1**（可商用 / **可嵌入** / 可再分发），**字节直接内嵌在 `yekitrim.exe` 里**，首次启动自动落盘 —— 不需要联网、也不需要手动下载：**Noto Sans SC**（中文正文，10.0 MB）与 **Noto Color Emoji**（**CBDT** 位图，非 COLRv1，9.9 MB）。
- ⚠️ **正常不需要跑 `install`**：它是**修复入口**（手删了库里那支、或落盘失败之后补回来）。`--id` 不在自带清单里会直接报错。
- 运行时的兜底栈 = 库里那支 → `Microsoft YaHei` → `Noto Sans SC` → `sans-serif`。
- ⚠️ 中文正文那支 **2026-10-02 由「霞鹜文楷 Lite」换成「Noto Sans SC」**（体积 13.2 → 10.0 MB，观感更中性）；老工程不受影响：配置里显式写着的旧字体名照旧按原名解析。
- ⚠️ `.ttc` / `.otc`（字体集合）**用不了**：浏览器的 `@font-face` 不认集合，收进来会出现"预览回退系统字体、出片却用了它"；导入时会被直接拒绝，请先拆包成单个 `.ttf` / `.otf`。
- 想用别的字体：在精修页「字体库」导入，或把 `.ttf` / `.otf` 放进字体库目录（`fonts list` 第一行会打印它的绝对路径）。字体许可与第三方清单见 [THIRD-PARTY-NOTICES](../THIRD-PARTY-NOTICES.md)。

## 诊断

### `yekitrim doctor [--url <base>]`

只读诊断（**不会创建账号库、不改配置**）。`--url` 默认 `http://127.0.0.1:26320`，与 `server` 默认监听一致。

```
$ yekitrim doctor
账号数据
  数据目录: …\cloud（剩余 24.3 GB / 共 251.2 GB · 可写 ✓）
  账号账本: …\yekitrim.db（1 个账号）
    - admin（初始密码 · 管理员，从未登录）
  初始密码: 仍在使用 ⚠（对外监听会被拒绝启动）
  会话: 0 条未过期
  初始密码文件: 存在 ⚠ …
单实例锁
  …\yekitrim.lock → 读不到内容（独占持有 = 有实例正在运行）
服务探测
  http://127.0.0.1:26320/api/health → 200（… listening=loopback，accounts=1 …）
建议 …
```

- **单实例锁**：`pid=0` = 上次正常退出；pid 已不存在 = 崩溃残留（下次启动自动接管）；读不到内容 = 有实例正独占持有（服务在运行）。
- **服务探测**：直接读 `/api/health`，能看出是否对外监听、是否走了 TLS 终止；探测不到就说明服务没在跑（默认端口 26320，可用 `--url` 改）。建议区里"仍在用初始密码"这类判断直接读本机账本，不走未认证的 `/api/health`。

### `yekitrim usage [--days N]`

LLM 用量（每次调用的 token / 缓存命中 / 耗时）。

| 参数 | 作用 |
|---|---|
| `--days <N>` | 统计最近多少天，默认 `30`（内部夹到 1..=3650） |

- 数据来自账本 `llm_usage` 表（`<data_dir>/yekitrim.db`，账本 v7 起）。
- 输出一行总量（调用 / 失败 / 输入 tok / 输出 tok / 缓存命中 / 缓存率 / 平均耗时）+ 四张表：按链路、按项目、按天（最近 14 行）、按模型。
- **缓存率 = Σ缓存命中 ÷ Σ输入**（只算输入侧、先求和再相除，不是每次调用比率的平均）；`prompt_tokens == 0` 时显示 `—`（老中转没报 usage / 失败行 token 全 0，不是"没命中"）。
- 一条记录都没有时只打印一句提示 —— 先跑一次事件检测 / 精修 LLM。CLI 侧会挂用量记账的分支只有 `process` / `detect` / `correct` / `polish *`（服务端另有自己的安装点）。

### `-V` / `--version`

打印版本号并退出。

- ⚠️ 它是**顶层选项**，不是子命令：`yekitrim version` 会得到 `error: unrecognized subcommand 'version'`（旧文档里的 `yekitrim version` 并不存在）。`yekitrim --version` 与 `yekitrim -V` 等价。
