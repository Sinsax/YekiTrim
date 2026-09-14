# V-Trim

直播录像自动切片工具。输入录播视频 + 弹幕文件，输出按事件分割的片段 + 字幕 + `publish_config.json`。

## 架构

Cargo workspace, 三个 crate:
- `crates/vtrim-core` — 管线编排 + 全部业务逻辑
- `crates/vtrim-cli` — CLI 入口 (clap)
- `crates/vtrim-polish` — GSAP 精修引擎（从 vtedit 合并）

**Stage 编号**: AudioExtractor(1) → Transcriber(2) → EventDetector(3) → Cutter(4)，连续无跳过。

所有 stage 用 `#[async_trait]` 实现 `Stage` trait (`crates/vtrim-core/src/pipeline/stage.rs`)。

`PipelineRunner` 提供三种运行模式 (`runner.rs`):
- `run()` — 受文件存在性约束（而非 `current_stage`），单 stage 命令用
- `run_all()` — 无视文件检查从头跑到底，`process` 命令用
- `run_until(max)` — 跑到指定 stage 为止，`process --dry-run` 用

`current_stage` 不再持久化到 `vtrim.toml`，改为从输出文件存在性自动推导：

| 文件存在 | 推导阶段 |
|---------|---------|
| `output/video.wav` 或 `audio_file` 非空 | Stage 1 |
| `output/transcript.json` 或 `video.srt` 或 `transcript_file` 非空 | Stage 2 |
| `output/events.json` 或 `events_file` 非空 | Stage 3 |
| `output/clips/clips_order.txt` 存在 | Stage 4 |

## CLI

```
vtrim init <video> [--danmaku <file>]    # 创建 vtrim.toml
vtrim extract [-i] [-o]                  # 提取音频 (Stage 1)
vtrim process [-i] [-o] [--dry-run]      # 全自动管线
vtrim transcribe [-i]                    # Stage 2 (local)
vtrim detect [-t <transcript>]           # Stage 3 (自动发现: 参数 > config > output_dir/*.json > *.srt)
vtrim cut [-i] [-e <events>] [-o]        # Stage 4 (自动发现: 参数 > config/output_dir/events.json)
vtrim silence [files...]                 # 「去静音」: 长停顿映射为屏蔽轨道 (写 polish.toml, 不改视频文件)
       [--duration 2.0] [--threshold]    # 默认扫描 output/clips/, 未初始化则先 polish init
vtrim download                           # 下载全部 ASR 模型 (sensevoice/firered-zh/qwen3-asr) → .vtrim/models/
vtrim status                             # 查看当前进度
vtrim doctor [--url <base>]              # 安全与运行状况摘要（账号/初始密码/锁/磁盘/服务探测）
vtrim ingest token [--disable]           # 录播姬: 生成 webhook token / 关闭 webhook
vtrim ingest status                      # 录播姬: 采集配置概览
vtrim config init [--path]               # 生成全局配置文件
vtrim config show                        # 显示当前配置 (含 LLM)
vtrim correct [-t <transcript>]          # LLM 订正转录文本
vtrim polish init [<dir>]                # 精修: 初始化项目 + 解析字幕弹幕
vtrim polish llm [<dir>]                 # 精修: LLM 生成 segments（使用 .vtrim/llm.toml）
vtrim polish process [<dir>]             # 精修: 后处理 + 下载资源 + 计算位置
vtrim polish render [<dir>]              # 精修: 渲染视频（自动先跑 process）
vtrim polish cover [<dir>] [--template]  # 精修: 生成封面
vtrim polish preview [<dir>] [mode]      # 精修: 浏览器预览（video/cover/双端口）
vtrim polish doctor [<dir>]              # 精修: 系统检查
vtrim polish test [<dir>]                # 精修: 测试模式
vtrim polish stickers list               # 本地贴纸: 列出索引（组名+成员数+tags）
vtrim polish stickers test <kw...>       # 本地贴纸: 测试 keyword→贴纸匹配（诊断用）
vtrim polish stickers scan [--llm]       # 本地贴纸: 生成/更新 stickers.json 清单（--llm 批量生成英文 tags）
```

所有 stage 命令的 `-i` 可选，默认从 `vtrim.toml` 的 `input_file` 读取。

## 状态机

`current_stage` 不再持久化到 `vtrim.toml`，改为从输出文件存在性自动推导。

| 规则 | 示例 |
|------|------|
| `stage > current + 1` → 拒绝 | `detect`(3) 无转写文件 → ❌ |
| `stage <= current` → 可重跑 | `transcribe`(2) 存在转写文件 → ✅ |
| 完成后 stage 自动推进 | `transcribe`(2) 成功后文件已存在 |
| `process` 无视文件检查, 全部重跑 | |
| stage 无跳过 | 缺失前置文件则拒绝 |

实现: `config/types.rs` 的 `current_stage()` 方法通过文件存在性推导。

## 配置文件

两层合并: `.vtrim/config.toml` (程序) → `vtrim.toml` (项目, 后加载覆盖前者)。无环境变量。
实现见 `vtrim-core/src/config/mod.rs::load_config_at`（先 global 后 project）。

`vtrim init` 生成 `vtrim.toml`（项目字段：input/danmaku/转写/剪切参数）。
`vtrim config init` 生成 `.vtrim/config.toml`（`[asr]` + `[detection]` + `[correct]` + `[sfx]`）和 `.vtrim/llm.toml`（`[llm]`，覆盖写）。
`vtrim server` 启动时若程序配置缺失会自动补齐（`service::ensure_global_config`）：只创建缺失文件；
已存在但 TOML 语法错误的会备份为 `*.toml.broken` 后重建（坏文件会让整份配置被忽略）。
设置页表单只覆盖常用项，保存走 `config::patch_config_file`（toml_edit 合并，保留其他 section 与注释），
表单数据源用 `/api/config` 的 `global_config`（global-only），不能用 merged 的 `config`。

**`vtrim.toml`** — 项目配置:
```toml
input_file = ""
danmaku_file = ""
transcript_file = ""
audio_file = ""
events_file = ""

[detection]
danmaku_window_secs = 10

[cutting]
output_dir = "./output"
overwrite = false
silence_noise_db = -30
```

**`.vtrim/config.toml`** — 全局程序配置:
```toml
[asr]
model = "sensevoice"         # ASR 模型: sensevoice(快) | firered-zh(CTC,较准) | qwen3-asr(最准,慢)
num_workers = 0              # 0=auto(CPU/4,上限4), N=指定并行数(不超过CPU核数)
chunk_duration = 60.0        # 每块秒数
chunk_overlap = 10.0         # 块间重叠秒数
segment_gap = 1.0            # 段间静音阈值(秒)，超过此间隔才切为两条字幕
llm_correct = false          # LLM 订正转录文本（需配置 [llm]）

[correct]
system_prompt = "..."       # 订正 prompt（覆盖内建默认）

[detection]
# 主播名称 (非空时自动追加到切片标题末尾)
vtuber_name = ""
# 弹幕密度统计窗口(秒)
danmaku_window_secs = 10
# 检测 system_prompt（修改此处覆盖内建默认）
system_prompt = "..."
# 检测 user prompt_template（{segments} 会被替换；留空 = 内建默认）
prompt_template = "..."

[cutting]
output_dir = "./output"
overwrite = false
silence_noise_db = -30

[sfx]
# 音效屏蔽列表 (vtrim polish 使用): 精确 id 屏蔽单个音效, "tag:xxx" 屏蔽整个标签族
blacklist = []
```

**`.vtrim/llm.toml`** — LLM 配置（独立文件）:
```toml
[llm]
base_url = ""              # OpenAI 兼容 API 地址
api_key = ""
model = ""
temperature = 0
max_tokens = 65536
extra_params = { seed = 42, thinking = { type = "disabled" } }
```

merge 条件: String 字段 `!is_empty()`, f64/u32 字段 != 默认值, Vec 字段非空时追加合并。

**配置默认值注意事项**: 所有配置类型必须手动实现 `Default`（而非 `#[derive(Default)]`）以确保 serde 默认值与 Rust 默认值一致。`Config::default()` 用 `toml::from_str("")` 初始化，依赖各字段的 `#[serde(default)]` 来调用正确的 `Default` impl。新增字段时必须同时加 `Default` impl + `merge()` 条件。

## 各阶段 I/O

### AudioExtractor (Stage 1, `vtrim extract`)
- 输入: 视频 → ffmpeg 16kHz mono WAV + ffprobe VideoInfo
- `ffmpeg -y -vn -acodec pcm_s16le -ar 16000 -ac 1` → `output/video.wav`

### Transcriber
- sherpa-onnx 库 (CPU only)，模型按 `[asr] model` 选择: `sensevoice` (默认) | `firered-zh` (FireRedASR v2 CTC int8) | `qwen3-asr` (Qwen3-ASR 0.6B int8, 最准最慢)
- firered-zh / qwen3-asr 无 token 级时间戳: 兜底按标点+字数占比分配块内时间 (split_text_by_ratio)
- 可配置定长分块 (默认 60s + 10s 重叠)
- 并行转写: 默认 CPU/4 路并行 (上限 4)，每 chunk intra-op threads=2
- SRT 使用 UTF-8 BOM 写入 (Windows 字幕兼容)
- 输出: `video.srt` + `transcript.json`，自动写 `transcript_file` 到 vtrim.toml

### EventDetector
- 一次 LLM API 调用分析全部 segments (利用 64k+ 上下文窗口), 非分批
- 全中文 prompt: ≥1分钟片段、B 站风格标题(含 emoji)、详尽摘要
- 弹幕: ASS 解析 → 10s 窗口归一化密度 → 每个 event 的 `popularity` 0~1
- 按 popularity 降序赋 `rank`，最终按时间排序输出
- 降级: API key 为空时使用单个默认事件（调用失败则报错）
- 输出: `events.json`

### Cutter
- GPU 加速编码 (NVENC/QSV/AMF 自动检测, 降级 libx264)
- `ffmpeg -ss {start} -i {input} -t {duration} -c:v {auto_encoder} -preset {auto_preset} {qparam} 23 -pix_fmt yuv420p -c:a aac -movflags +faststart {output}`
- 命名: `{label}.mp4` → `output/clips/` (无编号前缀)
- 字幕: 从原始 SRT 裁剪时间范围 + 偏移 → `{label}.srt`
- 输出: `output/publish_config.json` + `output/clips/clips_order.txt` (按弹幕热度降序)

## WebUI / Server

- `vtrim server` 只服务 `webui/dist`（Vue3 构建产物），**不再内嵌单文件页面**：
  找不到 `webui/dist/index.html` 时 `/` 返回 **503 + 构建指引**，启动日志也会告警。
  构建：`cd webui && npm install && npm run build`。`webui/dist` **已入库**
  （`webui/.gitignore` 不再忽略它）：改了 `webui/src` 必须重建并一起提交，否则端出旧界面。
- **样式纪律（前端改样式前必读）**：唯一设计标准是
  `docs/web-ui-mockups/sakura-cream.html`（配色/圆角/阴影/字体，v0.2 §4 全量取自它），
  v5 壳与数据型组件的布局数值取自交互基准 `docs/web-ui-mockups/ia-demos/multiuser.html`。
  - `webui/src/styles/tokens.css` = 色板/圆角/阴影/字体变量（**不得另取色**）；
    `webui/src/styles/app.css` = 两份样稿组件类的**并集**（`.shell/.side/.proj/.snode/.foot-item/
    .user-chip/.card/.card-blk/.btn/.mini-btn/.chip/.state-badge/.src-badge/.tb/.tabs/.wiz-src/
    .player/.lane/.grid9/.meter/.login-*/.empty/...`）。
  - **组件内不再重复定义这些基准类**（scoped 样式只写样稿里没有的东西，如行内删除按钮、
    页面专属布局）；新增组件先在样稿里找同名类，找到就照抄数值。
  - 关键数值：侧栏 268px、卡片 `22px 22px 18px`（`.card-lg`）/`16px`（`.card-blk`）、
    页标题 26px、chip `2px 10px`、进度条 6px、贴纸 52px/`rotate(-5deg)`、
    `.cols-main` = `1fr 300px`、主区 padding `18px 22px 60px`。
- **前端信息架构（v5 工作台，`docs/plan/webui-workbench-redesign.md`）**：
  - 壳：左侧栏常驻（品牌 / 项目过滤 / 项目树：名称+来源徽标+进度徽标+可展开的
    预处理·切片·精修节点+占用 / 底部全局入口：首页·入库·设置(admin) + 用户 chip）；
    面包屑 + 内容区；任务抽屉、改密弹窗、Toast/Dialog 挂在壳上；登录页不套壳。
    **侧栏不再有「＋新建项目」与「当前项目」入口**（建项目在首页，导航靠项目树/面包屑）。
    **侧栏可向左折叠**（用户要求）：品牌行右侧的折叠按钮 → `.side.mini`（app.css，
    `width:64px` + `transition:width .18s`）。折叠态 = 品牌 V + 折叠按钮（竖排）+
    项目首字圆头像（`title` 名称·当前阶段，点击直达该项目当前阶段，运行中红点）+
    底部三图标入口 + 用户头像；展开态保留过滤框与完整项目树。状态在 `stores/ui.js`
    的 `ui.sideCollapsed` / `toggleSide()`，localStorage `vtrim.ui.sideCollapsed`，
    **模块初始化时同步读取**（放 computed 里读会在首帧闪一下展开态）。
    折叠轨的类（`.rail-*`/`.side-toggle`/`.rdot`）是样稿没有的新态，写在 SideNav 的 scoped 样式里。
  - 首页 `#/workbench` = **「新增项目」区块**（先选来源，再切换成对应的导入方式）+
    「录制导入（最近）」；**没有项目列表**（原先的项目表已移除）：
    - 服务器文件（仅 admin）：首页内嵌目录浏览器（任意绝对路径 + 快捷入口 + 上一级），
      选一个视频 → 新建项目；非 admin 显示"仅管理员"；
    - 录播姬录制：列出待处理会话（`unassigned`/`failed`）供 admin 指派并入库 + 「扫描录制目录」；
    - B 站链接：P4 未实现，如实标注不可用。
  - 项目级危险操作（**删除源录播 / 删除项目**）在 `#/p/:pid/settings`（项目设置页也显示
    名称/来源/阶段/占用/源大小/产物，以及三个阶段的打开入口）。
  - 路由：`#/workbench`（首页：新增项目 + 录制导入最近）、`#/p/:pid/{prepare,slices,polish,polish/:clip,settings}`、
    `#/ingest`、`#/settings`（仅 admin）、`#/login`；`/` 按 localStorage 回上次项目+阶段；
    **旧路径 `/project/:cfg/**`、`/polish`、`/jobs`、`/models`、`/projects` 全部 redirect**。
  - 作用域单点：`api/scope.js` 的 `scopeQ(pid)`；业务调用只在 `api/projectData.js` /
    `api/polish.js` / `api/jobs.js` 三处拼查询串，视图/组件不自己拼。
  - `stores/workbench.js`：项目列表 + 侧栏展开/过滤/上次位置 + per-pid 数据缓存与
    `invalidate(...)`；`composables/useJobs.js` 的 `onJobDone` 驱动失效矩阵
    （任务结束 → 缓存失效 + 徽标刷新）。任务带 `project` 字段，侧栏显示 "预处理 42%"。
  - 预处理页 = 播放器 + 字幕轨道（块内行内编辑/拖动平移）+ 切片标记行 + 预处理动作卡；
    精修页 = **片段 chip 条**（`FragmentList chips`：编号 + 标题 + 状态 chip 的一行，
    比卡片墙省 ~100px 竖高；卡片墙仍用于列表态/未初始化态）→
    `grid cols-main edit-grid` 左列 = **预览 + 时间轴上下相邻**（用户要求：最大化窗口下两者同屏）、
    右栏 = 「生成与导出」卡（LLM 剪辑参数块〔区域编辑 + 贴纸来源〕→ 渲染导出）→
    封面设计器 → 底部产物整行；切片页 = 操作条 + 卡片墙 + 未剪切事件（侧收细条 / 卡片式放大）。
    编辑区网格用 `align-items:start`（右栏不跟着左列拉长）；有片段时页头不再输出 `.sub`
    （片段条 + 面包屑已说明是哪个片段），竖高全留给预览。
  - **时间轴编辑器必须保持紧凑**（用户要求精简；它是竖高大户）：泳道 **26px（原 34）×7 条
    + 标尺 20px = 202px**；**卡片头取消** —— 标题/提示经 `<slot name="head">` 进
    `TimelineEditor` 的 `.tl-head` 一行，与「去静音 / 撤销去静音」按钮同排；卡片 padding
    收到 12px（`.tl-card`）。改泳道高度时同步三处：`waveBars()`（波形 SVG `height="26"`、
    柱高 ≤22、`y = 26 - h`）、`.block`（`top:3px; height:20px; line-height:20px; font-size:10px`）、
    `.lane-label`/`.lane-track` 的 26px。
    **「去静音」只在时间轴头部**（那里带状态文案与「撤销去静音」）；右侧「生成与导出」卡
    **不要再放一个去静音**（用户指出重复）。
  - **预览尺寸随视口收缩**：`PolishPreview` 的 `.stage` 宽度 =
    `min(100%, calc(clamp(170px, 100vh - 600px, 620px) * 16/9))` —— 高度按 16:9 由「剩下的
    竖高」反推；`600px` = 面包屑 66 + 内容 padding 18 + 页头 ~80 + 片段条 ~50 + 编辑区上边距 16
    + 预览卡头/控制条 ~93 + 左列间距 16 + **时间轴卡 ~259**（改这些布局时同步改这个数）。
    1080p 浏览器最大化（视口 ~950）→ 预览舞台 ~350px 高（约 620×350）；视口 1200 → ~600；
    大屏封顶 620。**别再写死 `66vh`**（那样只有预览自己能进首屏）；也别把常数压到 600 以下
    —— 那会让时间轴掉出首屏（用户先要"同屏"、后嫌"太小"，600 是两者平衡点）。
  - **区域编辑（取代九宫格 3×3 选择器）**：在「生成与导出」卡的 **LLM 剪辑参数块**里，
    只有「人物」「弹幕区域」两个按钮（同一个再点=退出，点另一个=换区域；无区域下拉、
    无贴纸区域）→ 左侧 `PolishPreview` 进入**选框模式**（视频运镜暂停、缩放/滤镜复位；
    盒外压暗；拖框体平移、拖 8 个角/边缩放、空白拖拽重画，坐标为 1920×1080 整帧百分比），
    读数 / 保存选框 / 清除选框都在右侧同一块内，预览卡头显示「选框模式 · 运镜已暂停」chip。
    （**别把这些控件放到页头**：页头在片段列表上方、离预览很远，用户实测反馈"按钮不见了"。
    也别只放在预览卡头而让参数与动作分家 —— 用户要求"区域编辑和贴纸来源作为 LLM 剪辑的参数"。）
    **贴纸落点不在 UI 里框选**（`layout.sticker_area` 由 `process` 计算，要改就手改 polish.toml）。
    保存写 `polish.toml` 的 `layout[区域].top_left/bottom_right`（"55.00%" 百分比字符串），
    清除即删除两个角回到原 `grid`（grid 永不被框选改写 → 旧工程零迁移）。
    **保存不触发任务**：人物框只影响运镜聚焦中心（渲染时读取）、弹幕区域仅作参考，
    都不需要重跑 process（只有贴纸落点才依赖 process，而它已不在 UI 内）。
    读取优先级（`vtrim-render-core/src/types.rs`）：`free_rect()` →
    `region_rects()`/`region_origin()` 一律**自由选框 > grid > origin_grid/origin**；
    JS 侧同一规则在 `webui/src/polish/engine.js` 的 `regionRect/regionOrigin`（GRID_RECTS
    表须与 Rust `grid_rect` 对齐）与 `templates/index.html` 的 `liverOrigin`。
  - **任务完成后的刷新必须静默**（别让页面看起来像浏览器刷新）：`PolishView.load({silent:true})`
    不置 `loading`、不卸载编辑器 —— 非静默时 `v-if="loading"` 会把整个编辑器换成骨架屏，
    编辑器一卸载，预览视频、`selectRegion` 与滚动位置全丢。需要重新从磁盘取数的子组件
    各自用 `:key` 计数重挂载：`TimelineEditor`=`segKey`（LLM 剪辑/初始化/去静音）、
    `CoverDesigner`=`coverKey`（渲染导出）、`PolishPreview`=`previewKey`（任何任务完成）。
  - 设置页：全局配置表单/源码双模式 + LLM 测试 + 模型管理（`ModelList`）+ 项目设置
    （项目配置表单/源码，ASR 模式等走 `PUT /api/project-data/config {patch}`）。
- **认证（P1 已落地）**：除 `/api/health` 与 `/api/auth/login` 外**所有接口都要求会话**，
  含 loopback；**没有免登录分支、没有来源地址判定**。单人部署＝只有一个 admin 账号的部署：
  - 首次启动生成 `admin` + 随机初始密码，日志打印一次并写入
    `<data_dir>/initial-admin-password.txt`（首次登录成功后自动删除）；重启不重新生成。
  - 会话在 SQLite `sessions` 表，`vtrim_sid` cookie 为 **HttpOnly**，7 天滑动续期；
    写请求必须带 `X-CSRF-Token`（= `vtrim_csrf` cookie，双提交），缺失/不匹配 → **403**；
    未登录 → **401**（前端 `client.js` 统一跳登录并记回跳）。
  - 登录失败 5 次（同用户名+同 IP，取 `ConnectInfo` 对端地址，**不信任 `X-Forwarded-For`**）
    → **429**，窗口 15 分钟；账号禁用 → 403/401 文案直出。
  - 启动预检：`data_dir` 不可写 → 启动失败（身份不可缺省，绝不退化成内存模式）；
    `vtrim.lock` 单实例保护（独占句柄 + pid 存活判定，`--force` 只在确认残留时用）；
    `--host 0.0.0.0 --allow-remote` 且 `must_change_password=1` → 拒绝启动。
  - `/api/health` 只回能力位与 `accounts / must_change_password / listening / tls_terminated`，
    **不含任何路径**（它无鉴权）。
- **项目托管与隔离（P2 已落地）**：
  - 归属由 SQLite `projects` 表决定；`/api/projects` 只回本人项目，
    `DELETE /api/projects/:id`（只注销 + 删 vtrim.toml）、
    `DELETE /api/projects/:id/source`（删源录播，保留产物，唯一的手动释放空间手段）。
  - **路径面归零**：所有 `/api/project-data/*` 用 `?project=<id>`；polish 用
    `{project, dir:"output/clips/…"}`（服务端把相对路径归一化成项目内绝对路径）。
    旧的 `?config=<路径>` 只在"该路径已登记且属于本人"时作为过渡兼容（未登记一律 404）。
  - `vtrim-core/src/tenant.rs` 是唯一拼接入口：拒绝绝对路径/UNC/`C:foo`/`..`/符号链接逃逸。
  - `/api/media/<project_id>/<项目内相对路径>`；旧写法（绝对路径）也只在本项目内放行。
  - `/api/admin/*`（config/models/doctor/llm test/**users**）仅 admin；旧路径作为过渡别名挂在同一层。
    `GET /api/admin/users` 只回展示字段（id/username/role/disabled/quota_bytes/created_at），**不含口令哈希**，
    供"待分配池指派给谁"下拉使用。
  - `/api/fs/list` 仅 admin，**可以浏览服务器任意目录**（`{dir}` 绝对路径，与单用户时代一致；
    也兼容 `{root, path}` 相对写法）。这是"服务器上已有文件"这条 admin-only 来源，**永不接受上传**；
    `[storage] local_roots` 只是前端下拉的**快捷入口**，不再是硬限制。
    **进入下一级用 `{dir, enter:"子目录名"}`，路径由服务端拼**：前端自己拼 `dir + '/' + name`
    会把 canonicalize 的 `\\?\F:\a` 拼成 `\\?\F:\a/b`（Windows ERROR_INVALID_NAME 123）。
    因此所有出 JSON 的路径都过 `vtrim_core::path::display()`（剥掉 `\\?\`/`\\?\UNC\`），
    前端拿到的永远是 `F:\a\b` 这种可回传、可拼 URL 的形式。
  - 管理员「首页 →「新增项目」→ 服务器文件」：`POST /api/projects {source:{kind:"local", dir, input, danmaku, output_dir}}`
    （`dir` 绝对路径；非管理员只能用自己的租户根，且 `/api/fs/list` 与 `kind:"local"` 一律 403）；
    **目标目录已有 `vtrim.toml` 时只登记/打开、不覆盖**（保护手改过的工程设置）。
  - 任务带 `owner_id`：`/api/jobs`、`/api/jobs/:id`、`/api/jobs/:id/events`、canc、`/api/stream`
    全部按归属过滤，他人任务 404（SSE 在服务端过滤）。
  - 前端作用域是项目 id：`/project/:pid`，`api/scope.js` 是唯一拼 `project=<id>` 的地方。
- 真渲染（wgpu）暂缓：未启用该 feature 时前端**隐藏**入口（`/api/health.wgpu_render` 判定），
  日后开发时只需恢复 UI 入口，`PolishPreview` 里的逻辑与后端接口都还在。
- **录播姬采集（P3 已落地）**：
  - `vtrim ingest token [--disable]` 生成 webhook token（明文只显示一次，配置里只存 scrypt 哈希）
    并开启 `[ingest] webhook_enabled`；`vtrim ingest status` 看配置概览。
  - `POST /api/ingest/bililive/<token>`：**自带 token 鉴权、不要求会话**（录播姬只能填 URL）；
    只做 验 token → `EventId` 去重 → 落库，然后**立即 2xx**；`allow_ips` 白名单可选；
    `EventData` 用大小写不敏感的多别名取值，未知事件类型忽略但记事件表。
  - `ingest_sessions/ingest_files/ingest_events/channel_map` 四张表；
    `GET /api/ingest/records`（本人；admin `?all=1`/`?unassigned=1`）、`GET/PUT /api/channel-map`、
    `POST /api/ingest/retry`、`POST /api/admin/ingest/assign`、`POST /api/admin/ingest/scan`、
    `GET /api/admin/ingest`。
  - 入库 worker（串行＝入库槽 1）：完结判定（SessionEnded + 全部分片闭环或空闲窗口）→
    水位/配额预检 → 落位（同盘 rename / 跨盘 copy+校验+删源）→ 多分片
    `ffmpeg -f concat -c copy` 合并为 `source.mp4`（失败回退重编码，原始分片保留）→
    弹幕（`.xml`/`.ass`/`.srt`）一并搬入 → 建项目 → 按 `auto_pipeline` 自动跑管线；
    失败清理半成品目录并写 `state=failed` + 错误文案。
  - 兜底扫描：`watch_interval_mins`（0=关闭）或手动 `POST /api/admin/ingest/scan`，
    文件"两轮大小+mtime 不变"才登记，覆盖漏推送/录播姬重启/历史录播导入；
    未映射房间进"待分配池"（`state=unassigned`，仅 admin 指派）。
  - 前端：`#/ingest` 入库页（记录 + 房间映射 + 待分配 + 扫描）与总览页「录制导入（最近）」，
    `/api/health.ingest_ready` 决定是否显示配置引导卡。
- API 都在 `/api/*`：未注册路径返回 **404 + JSON**（`{"ok":false,"error":"未知接口: …"}`），
  不会回退成 SPA HTML（否则前端会把 200 HTML 解析成 `null` 静默失败）。
- 任务模型：**同一时刻只允许一个长任务**（`JobManager::create` 返回 `BusyJob` → HTTP 409）；
  取消经 `ProgressReporter::cancelled()` 在**阶段/批次边界**生效（进行中的单次
  ffmpeg/ASR/LLM 调用不会被打断）；`/api/jobs/:id/events` 先回放历史事件再续传，
  `Lagged` 不会误报成失败。
- `/api/polish/*` 的 `dir` 必须是非空且存在的目录；`polish.toml` / `events.json` /
  `vtrim.toml` / 全局配置的读-改-写走 `vtrim_core::filelock::with_path_lock` 串行化。
- 监听非 loopback 地址必须显式 `--allow-remote`；账号数据在
  `<data_dir>`（`--data-dir` / `VTRIM_DATA_DIR`，默认 `<exe_dir>/.vtrim/cloud`），
  由 `vtrim user *` 与 HTTP 侧共用同一个 SQLite 账本。

### 多用户服务进度（设计：`docs/superpowers/specs/2026-09-12-multi-user-server-design.md`）

| 阶段 | 状态 |
|---|---|
| P1 step-1 账号账本 + `vtrim user` CLI | ✅ 已落地 |
| P1 step-2 HTTP 会话鉴权 + 前端 M1 登录 | ✅ 已落地（`server/auth.rs`、`service/instancelock.rs`） |
| P1 step-3 `vtrim doctor` 安全摘要 | ✅ 已落地（`service/doctor.rs`、`service/disk.rs`） |
| P2 项目托管与隔离（`?project=`、囚笼、任务归属、前端 M2） | ✅ 已落地（`vtrim-core/tenant.rs`、`service/projects.rs`、`server/scope.rs`） |
| P3 录播姬接入（webhook / 入库 worker / 兜底扫描 / 前端 M3） | ✅ 已落地（`server/ingest.rs`、`server/ingest_worker.rs`、`danmaku::parse_bililive_xml`） |
| P4 B站链接通道 / P5 容量与运维 | ⏳ 待做 |

## 开发

```bash
cargo build                    # 0 warnings
cargo check -p vtrim-cli --all-targets   # 改 server 后必跑（0 warnings）
cargo test -p vtrim-cli        # server 单测（JobManager / Range / 路由 404）
cargo test --lib danmaku       # ASS 解析 + 密度计算测试
cargo run -- status            # 查看当前进度
```

前置条件: `ffmpeg` + `ffprobe` 在 PATH 中。

### 新增配置字段步骤

1. `config/types.rs` 加字段 + serde default
2. `config/mod.rs` 的 `merge()` 加对应条件（注意 `.vtrim/config.toml` 在 `vtrim.toml` 之后 merge）
3. 结构体手动 `Default` impl (有字段级 serde default 时必须)
4. `config/default.toml` + 注释

### 核心文件索引

| 路径 | 说明 |
|------|------|
| `vtrim-cli/src/main.rs:27` | `resolve_input()` input 回退 |
| `vtrim-cli/src/main.rs:41` | `build_pipeline_runner()` 注册 stage |
| `vtrim-cli/src/main.rs:62` | `check_stage()` 状态机 |
| `vtrim-core/src/pipeline/stage.rs` | STAGE_* 常量 + Stage trait |
| `vtrim-core/src/pipeline/runner.rs` | PipelineRunner (run/run_all/run_until) |
| `vtrim-core/src/config/mod.rs` | 两源合并 + toml_edit |
| `vtrim-core/src/config/types.rs` | Config 类型定义 + Default impl |
| `vtrim-core/src/types/mod.rs` | Event, PipelineContext, Transcript |
| `vtrim-core/src/download.rs` | 模型/binary 下载 + 断点续传 |
| `vtrim-core/src/asr/local.rs` | sherpa-onnx (CPU only) |
| `vtrim-core/src/detection/llm.rs` | LLM prompt + API |
| `vtrim-core/src/detection/mod.rs` | EventDetector + 弹幕热度 |
| `vtrim-core/src/danmaku/mod.rs` | ASS/SRT 弹幕解析 + 密度 |
| `vtrim-core/src/danmaku/parser.rs` | ASS Dialogue 解析 |
| `vtrim-core/src/llm/mod.rs` | 统一 LLM 客户端（core + polish 共用） |
| `vtrim-core/src/cutting/mod.rs` | Cutter |
| `vtrim-core/src/cutting/silence.rs` | 静音检测（detect_silence_ranges）|
| `vtrim-polish/src/project/silence.rs` | 「去静音」= 静音区间 ↔ 屏蔽轨道（mute_ranges / silence_ranges）|
| `vtrim-polish/src/commands.rs` | 所有 cmd_* 精修命令入口 |
| `vtrim-polish/src/process/mod.rs` | cmd_process 主流程（字幕/段处理/预算） |
| `vtrim-polish/src/process/sfx.rs` | SFX 规则引擎 + 音效解析 |
| `vtrim-polish/src/process/fx.rs` | 瞬时特效 duration 分类与 clamp |
| `vtrim-polish/src/process/download.rs` | 贴纸解析 + 资产下载命令 |
| `vtrim-polish/src/project/config.rs` | vtedit 风格项目配置 + Event/Segment 类型 |
| `vtrim-polish/src/render/chrome.rs` | Chrome headless 渲染 |
| `vtrim-polish/src/render/ffmpeg.rs` | FFmpeg 合成/编码/SFX 混音 |
| `vtrim-polish/src/render/server.rs` | 预览 HTTP server |
| `vtrim-polish/src/llm/mod.rs` | LLM 段分割 + 关键词 |
| `vtrim-polish/src/llm/events.rs` | Step2 逐段事件生成（core Agent + checkpoint） |
| `vtrim-polish/src/assets/downloader.rs` | emoji/irasu/SFX 下载 |
| `vtrim-polish/src/render/pipeline.rs:11` | cmd_render（自动 process → render → cover） |
| `vtrim-polish/src/commands.rs:296` | cmd_preview（自动 process → 双端口预览） |
| `vtrim-polish/src/commands.rs:200` | cmd_llm（使用 .vtrim/llm.toml） |
