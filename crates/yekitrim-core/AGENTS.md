# yekitrim-core — 管线编排 + ASR / 事件检测 / 剪切

> 范围：`crates/yekitrim-core`/** ｜ 上游：`yekitrim-polish` / `yekitrim-service` / `yekitrim-server` / `yekitrim-cli` 都依赖我 ｜ 验证：`cargo check -p yekitrim-core`

## 职责

整条切片的领域内核：把「录播视频 + 弹幕文件」变成「按事件分割的片段 + 字幕 + 事件清单」。
四个 Stage 的实现在这里（AudioExtractor → Transcriber → EventDetector → Cutter），编排器
`PipelineRunner` 也在这里；`yekitrim-service::core_pipeline` 是 CLI 与 `yekitrim-server` 共用的装配层，
它按 `-i`/`-o`/`-t`/`-e` 覆盖参数后驱动本 crate。此外还兜住工具模块：`types` / `path` / `srt` /
`fonts` / `download` / `progress` / `filelock` / `tenant`（没有专卡的都看这里）。

`lib.rs` 的模块表（**就是全表，新增模块必须在这里登记**）：
`types` / `config` / `fonts` / `path` / `srt` / `pipeline` / `audio` / `asr` / `llm` / `download` /
`detection` / `danmaku` / `cutting` / `progress` / `filelock` / `tenant` / `owner`。

## 入口

- `types::PipelineContext`（`ctx.input_path` / `output_dir` / `config` / `audio_path` /
  `video_info` / `danmaku_path`）、`types::Event`、`types::EventStore`、`types::Transcript`、
  `types::Timestamp`（`to_seconds` / `from_seconds` / `to_srt_format`）、`types::VideoInfo`。
- `pipeline::{Stage, STAGE_AUDIO, STAGE_TRANSCRIBE, STAGE_DETECT, STAGE_CUT}`（`pipeline/stage.rs`）。
- `pipeline::PipelineRunner`（`pipeline/runner.rs`）：`new(config)` / `add_stage(Box<dyn Stage>)` /
  `set_progress(Arc<dyn ProgressReporter>)` / `run()` / `run_all()` / `run_until(max_stage)`。
- 四个 Stage：`audio::AudioExtractor`（1）、`asr::Transcriber`（2）、`detection::EventDetector`（3）、
  `cutting::Cutter`（4）。
- `config::{load_config, load_config_at, load_global_config, load_llm_config, patch_config_file,
  write_project_config, save_transcript_path_in, normalize_project_paths_in}`；
  `config::types::Config` / `ProjectConfig::current_stage(&output_dir)`。
- `process::{cmd, hide_console_window}`：起子进程的唯一入口（**全仓都用它**，理由见「边界」）。
- `path::{yekitrim_home, resources_dir, sticker_dir, sticker_dir_readable, video_dir, video_dir_readable,
  runtime_dir, font_dir, user_home, display}`；`fonts::{dir, ensure_dir, scan, scan_at,
  resolve_file, resolve_file_at, family_of, is_font_file_name, is_safe_id}`。
- `owner`：`Owner::{Local, User(uid), user, is_local, uid, from_env, from_env_value, as_env_value}`、
  `AssetKind::{Fonts, Stickers, Videos, dir_name}`、`owner_root(_at)` / `resources_dir(_at)` /
  `asset_dir(_at)` / `state_dir(_at)` / `config_path(_at)`、
  `sticker_dir_readable(_at)` / `video_dir_readable(_at)`、`migrate_legacy_assets_to`、常量 `OWNER_ENV`。
  ⚠️ **`Owner` 只回答"是谁"、不回答"在哪"**（路径一律由本模块算）—— 这是"将来真要落成用户级目录时
  业务代码一行不动"的全部依据。
- `progress::ProgressReporter`（trait）+ `NoopProgress`；`filelock::{with_path_lock, lock_path,
  with_path_lock_sync}`；`tenant::{jail_join, jail_existing, ensure_within}`。
- `download::{model_names, model_install, download_all, download_vad_model, ensure_models}`。

## 边界

- ⚠️ **全仓规矩：起子进程一律走 `process::cmd()`**（`src/process.rs`），别直接写 `Command::new`。
  原因：`yekitrim-tray` 是 `windows_subsystem = "windows"`（**进程没有控制台**），
  它拉起的每个控制台程序都会被 Windows **分配一个新控制台窗口** —— 用户看到的就是「启动时黑框一闪」。
  实测：托盘启动时 WebUI `GET /api/health` 探 `ffmpeg -version` / `ffprobe -version`，
  冒出**两个** `ConsoleWindowClass` 窗口；走 helper 后归零。
  `process::hide_console_window(&mut cmd)` 给已建好的 std `Command` 补同一条；
  tokio 那份用 `hide_console_window(tcmd.as_std_mut())`。
  ⚠️ 判据是「**当前进程有没有控制台**」而不是无条件加 `CREATE_NO_WINDOW`：
  CLI 有控制台，子进程继续继承它，终端里的输出/进度必须一个字节不变。
  ⚠️ **别加第二个 `creation_flags`**：它覆盖整个 dwCreationFlags（全仓只有这一处设过）。
  机器判据 = `scripts/check_module_boundaries.py` 的「子进程不许绕过 core 的启动助手」（17 条规则之一）。

⚠️ **core 不许认识上层**（门禁：`python3 scripts/check_module_boundaries.py`，扫 `crates/yekitrim-core/src`）：
源码里**不得出现** `yekitrim_polish` / `yekitrim_service` / `yekitrim_server` / `yekitrim_cli`——
分层是单向的 core → polish → service → server → cli，底层一旦反向引用，两层就粘回去。
需要上层能力时由调用方**注入**（sink / 回调 / 参数），不是在 core 里 `use` 上来。

⚠️ core 只 `emit` LLM 用量记录，**写库由 CLI/server 侧装的 sink 负责**（见 `src/llm/AGENTS.md`）。

- **Stage 1（`audio/`，没有专卡，规矩在这里）**：输入视频 → ffmpeg 16kHz mono WAV + ffprobe
  `VideoInfo`；命令是 `ffmpeg -y -vn -acodec pcm_s16le -ar 16000 -ac 1` → `output/video.wav`，
  并把 `video_info` / `audio_path` 写回 `PipelineContext`。改名或改路径会连带炸
  `status.rs` / `current_stage()` 的推导（见下表）。
- **Stage 编号**：AudioExtractor(1) → Transcriber(2) → EventDetector(3) → Cutter(4)，连续无跳过。
- `current_stage` 不再持久化到 `yekitrim.toml`，改为从输出文件存在性自动推导
  （`config/types.rs::current_stage`）：

  | 文件存在 | 推导阶段 |
  |---------|---------|
  | `output/video.wav` 或 `audio_file` 非空 | Stage 1 |
  | `output/transcript.json` 或 `video.srt` 或 `transcript_file` 非空 | Stage 2 |
  | `output/events.json` 或 `events_file` 非空 | Stage 3 |
  | `output/clips/clips_order.txt` 存在 | Stage 4 |

- **状态机**：`stage > current + 1` → 拒绝（`detect`(3) 无转写文件 → ❌）；`stage <= current` → 可重跑；
  完成后 stage 自动推进；`process` 无视文件检查、全部重跑；stage 无跳过（缺失前置文件则拒绝）。
- **弹幕路径解析在 `pipeline/runner.rs::resolve_danmaku`**（`inject_config` 调它）：相对路径按
  **工程根（`output_dir` 的父目录）→ 视频父目录**的顺序试，两个基准都试一遍，否则
  「视频在 `videos/`、弹幕在工程根」的布局会静默丢掉弹幕。
- ⚠️ **2026-10-09 资产布局收拢**（`path.rs`，全仓路径的唯一 choke point —— 20+ 处调用都只认这几个函数）：
  `resources/{fonts,stickers,videos}` 是**用户素材**（用户增删），`runtime/{polish-to-dhampir.mjs,
  effects.json}` 是**随版本发布的只读交付物**。分界是"谁往里放东西"。
  ⚠️ **写往新位置、读两个位置都认**（`sticker_dir_readable()` / `video_dir_readable()`）：
  判据是**目录存在与否**、不是"里面有没有东西"（用内容当判据会让新建的空库被老位置顶掉）。
  **不做自动搬迁**（用户手工移），回落只为"升级后库不会突然变空"。
  ⚠️ 视频库挪出 `stickers/` 的收益：原先它是贴纸根下的普通子目录，而 `scan_packages` 会把
  非 `.` 开头、非 `backup` 的子目录都当包扫 ⇒ 只靠"一张图都没有于是被丢掉"**碰巧**没污染列表。
  完整布局表与老→新对照在 **`docs/file-layout.md`**。
- ⚠️ **素材/配置的路径一律经 `owner`（2026-10-10 起）**：`Owner::Local`（托盘 / CLI /
  `--local-no-auth`）解析到**现状位置**（`<data>/resources/*`、`<data>/.yekitrim/`）⇒ 单人零迁移；
  `Owner::User(uid)` 解析到 `<data>/users/<uid>/…`。
  **不许再在别处拼 `data_dir().join("resources")`** —— 那就是第二条通道（本模块头部三条硬规矩）。
  子进程（出片/封面/预览）经 `OWNER_ENV` 从父进程继承身份。
  ⚠️ **账本（`users`/`sessions`/`invites` 表）永远全局**：它是身份的来源，按用户分是循环依赖。
- **字体库**：`yekitrim_core::fonts` + `path::font_dir()` = **`<数据目录>/resources/fonts/`**
  （单人；多用户走 `fonts::dir_for(owner)`）。
  ⚠️ **2026-10-02 起资产全部收回程序目录**（用户口径：*不要用户目录*）：`path::data_dir()`
  默认 = `exe_dir()`，可用 `YEKITRIM_DATA_DIR` 覆盖（debug/release 共用一份模型的唯一口子）。
  这条路径**曾经**是"用户固定目录 `~/.yekitrim/fonts` 优先 + 老库一次性搬家"，已整条删掉 ——
  代价是同一个库有两个可能的位置，实测 CLI 与 WebUI 各看一份、字体列表完全不同。
  现在读=写同一个目录，`fonts::ensure_dir` 只做 `create_dir_all`。`user_home()` 只剩一个
  用途：`fonts::legacy_user_dir_report`（**只报不删**，让用户知道旧目录还在、可以自己删）。
  `config init` 与 `server` 启动都会 `ensure_dir`。
  **两种布局都支持**：① v2「每支一个目录」`<slug>/font.ttf|otf` + `FONT.toml`
  —— ⚠️ **`id` 是 `FONT.toml` 里的字段，不再等于目录名**（2026-10-02）：目录名与字体文件名一律是
  **ASCII slug**（`ascii_slug` = `<hint>-<sha256(id)[..12]>`），给人看的那串汉字只住 `id` / `display_name`。
  因为底座把字体路径塞进 ffmpeg 的 `drawtext=fontfile=…`：**路径里一个非 ASCII 字符就让 ffmpeg
  每条字幕访问违例一次**（240 帧 4.0s → 37s，且 0 行字幕/0 条弹幕画得出来）。缺 `id` 的老库回落
  "目录名就是 id"（零迁移），`ensure_dir` 顺手跑 `migrate_ascii_layouts`（幂等）把非 ASCII 条目搬成 ASCII。
  元数据含 `id` / `family`（**内部族名，唯一权威**）/ `display_name` / `builtin` / `is_default` / `license` / `scripts`；
  ② 老布局（`.ttf` 直接丢在库里，id = 文件名去扩展名）—— **继续支持**，`scan_all`/`resolve_all` **目录优先**，
  `scan()`/`resolve_file()` 已改走 `*_all`。`family` 仍是内部族名（手写最小 `name` 表解析，按
  `(路径, mtime, 体积)` 缓存，别每次请求重读几十 MB）；**空串 = 系统字体栈**（首项由
  `yekitrim-polish` 的 `preferred_fallback_family()` 按“库里装了哪支”算，见其卡），老工程就是这一档。
  ⚠️ **平铺条目 `flat: true`**（`FontEntry`，界面打「老布局」标签）：散文件直接在库根目录的老布局，
  与导入/下载进来的库字体不是一回事（当年手工拷进来的系统字体常是这种）。
  ⚠️ **`.ttc`/`.otc` 导入一律拒收**（要收得先拆包）：浏览器
  `@font-face` **不认字体集合** ⇒ 收进来会“预览回退系统字体、出片却用了它”。`import_font` 还要求
  “读得出内部族名”；`remove_font` 拒删自带字体（判据 = `meta.builtin` **且** id 仍在
  `BUILTIN_FONTS` 清单里 —— 只看元数据会让**换下来的旧自带字体永远删不掉**）。
  **自带字体（P1-f）**：`BUILTIN_FONTS` 里两支（**Noto Sans SC** + Noto Color Emoji，均 SIL OFL-1.1）——
  ⚠️ **2026-10-02 中文那支由「霞鹜文楷 Lite」换成 Noto Sans SC**（体积 13.2→10.0 MB、观感更中性；
  族名恰好就是 `Noto Sans SC`，与 `fallback_chain()` 里那项**天然对上**，不像旧的英文/中文族名对不上）。
  中文正文**已内嵌**（`include_bytes!`，源在 `assets/fonts/`）——**首次启动自动落盘**，不联网。
  ⚠️ 这条卡原来写的「字节首次使用时下载」**已过时**：`download_builtin_font(s)` 与 `url` 字段仍在，
  但走的是"内嵌优先、只有内嵌字节不可用时才下载"的次序；`sha256` 仍是下载路径的最终防线。
  `scan_all()` 会**把没下载的自带字体也列出来**（`installed: false`、`file` 空）—— 界面据此给「下载」；
  ⚠️ **数「有没有可用字体」时必须滤掉它们**（`resolve_all` 解析得到才算数），否则会告诉用户「有字体」
  然后出片炸在「至少一个字体文件」上。**别把 ttf 塞进仓库**（约 38 MB 且会进 git 历史）。
  方案与理由：`docs/plan/font-library.md`（§5 设计 / §10 契约 / §15 自带字体）。
  ⚠️ **别改成“把字体拷进项目”**：预览页面由渲染服务器（ServeDir 根 = 项目目录）端给浏览器，
  所以库里字体必须有一条自己的路由 `/vt-fonts/:name`（在 `yekitrim-polish`，见其卡）。
  回归钉子：`fonts::tests`（扫描/解析/越权）+ `fonts::tests_v2`（布局/导入/自带/默认/搬家）。
  发行包**不带字体文件**，只建空目录 + `放这里.txt`（文案就写"本目录就是字体库"，见 README「字体」）。
- `filelock` 的两种锁别混用：异步 `with_path_lock` / `lock_path` 给 handler，
  **同步** `with_path_lock_sync` 只给同步上下文的 read-modify-write（core 管线的 `edit_toml_at` 用）
  —— ⚠️ 只在**微秒级**的文件读改写里用，别拿它包 ffmpeg / 网络请求；它**非重入**。
- `tenant` 是多租户路径囚笼：接口只接受项目内**相对路径**，由服务端自己拼绝对路径
  （`jail_join` 写入 / `jail_existing` 读 / `ensure_within` 校验外部传入的完整路径）。
- 前置条件：`ffmpeg` + `ffprobe` 在 PATH 中（Stage 1 与 Cutter 都要）。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo check -p yekitrim-core
```

`cargo check --workspace --all-targets` 改任何 crate 后必跑（0 warnings）；
跨模块边界另跑 `python3 scripts/check_module_boundaries.py`。
