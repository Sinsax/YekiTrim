# scripts — 门禁与工具脚本（结构/边界检查、路由对拍、出片报告、贴纸音效目录、ASR 原型资产）

> 范围：`scripts`/** ｜ 上游：改代码的人与 CI（两条 `check_*.py` 是改动的必跑项）｜ 验证：`python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py`

## 职责

仓库的**机器判据与大件生成工具**。规矩写成散文会腐坏（一开始没人违反，后来某次"就这一次"就破防了，而且不会报错、
测试也不会红），所以这里把两类硬边界变成能跑的检查；另外放了贴纸/音效目录的抓取打标脚本与 ASR 对齐原型的资产。
本目录**不是产品代码**，但 `check_module_map.py` / `check_module_boundaries.py` 是**每次改动都要跑的**。

## 入口

### 门禁（改动必跑）

- **`check_module_boundaries.py`** —— 边界门禁：15 条规则（14 条行级 + 1 条**成对判据**；P4 隔离 + 分层方向 + 前端边界），违规**非零退出**。
  覆盖：`downloader` 不许开子进程 / 不许知道 B站；`bilibili` 不许知道下载器；`net/` 不许知道 B站；
  `download_worker` 不许引用 `ingest_worker` / 不许有录播姬语义；`core` / `render-core` / `polish` 不许认识上层；
  `service` 不许认识 `server`；`server` 不许认识 `cli`；`cli` 只用 server 的入口；
  `components` 不许 import `views`；作用域查询串只在 `webui/src/api/` 里拼。
  **加规则的方法：往 `RULES` 表加一条 `(规则名, 相对路径列表, 正则, 为什么)`**（`why` 会在报错时打出来）。
  ⚠️ **"用了 X 就必须同时有 Y"这类文件级约束加进 `PAIRED_RULES`**（`(规则名, 路径, trigger, need, why)`）：
  行级正则表达不了跨行成对关系（`Agent::new(` 与 `with_cancel(` 本来就在不同行）。现有那条是
  「批量 LLM 循环必须交出取消探针」。判据刻意做粗（文件级）—— 目的是别漏掉新的批量循环，不是精确到语句；
  自证方法：临时放一个只含 `Agent::new(` 的文件进 `crates/vtrim-polish/src/llm/`，门禁应当 exit 1。
  用法：`python3 scripts/check_module_boundaries.py [-v]`。
- **`check_module_map.py`** —— 模块卡门禁：读 `docs/module-map.json`（模块 id → 目录 → 卡 → verify），
  校验**覆盖率 / 预算 / 四节骨架 / 索引表一致 / ⚠️ 行数下限**，违规非零退出。
  `coverage_roots`（`crates/*/src`、`webui/src`、`scripts` …）下**每个源文件都必须落在某张卡的目录内**，
  否则报"无主文件"；**任何没登记进注册表的 `AGENTS.md` 也算失败**（卡片会漂移）。
  用法：`python3 scripts/check_module_map.py [-v|--json]`。
- **`route_manifest.py`** —— 把服务端的 axum 路由表 dump 成稳定、可 diff 的清单。
  用途是**重构对拍**：`server/` 拆 crate 时的验收线是"`(method, path)` 集合前后一致"，
  而 `cargo test` 只能证明 handler 还过、证明不了**没有路由搬走**。它是**源代码级抽取器**（`axum::Router`
  运行时不暴露路由表，另一个办法是写一个重建 router 的测试，那得住在被搬动的 crate 里）。
  **exit code 始终 0** —— 比较是调用方的事（`diff` 两次输出）。**它抓不到** handler 体、中间件顺序与
  `.route_layer(...)` 的挂载语义（那些靠 router 单测）。用法：`python3 scripts/route_manifest.py [--root .] [--out manifest.txt]`。
- **`render_phase_report.py`** —— 出片日志的**阶段报告**（只读）：把 `render-*.log` 拆成
  「墙钟 / 分块渲染 / 抽帧 / 编码 / 固定开销」与「发帧 / 启动 / 页面循环 / 回读」两张表
  （配套文档 `docs/plan/polish-render-speed.md`）。默认目录 `<仓库>/target/debug/.vtrim/cloud/logs`。零依赖、只读。
  用法：`python3 scripts/render_phase_report.py [日志目录]`。

### 贴纸 / 音效目录生成（一次性工程）

- `build_komori_catalog.py` —— 爬小森平（効果音ラボ）全站音效目录 → `experiments/komori_full_catalog.json`
  （`--download` 会顺带下全部音频，**谨慎，~200MB**）。
- `build_komori_cn.py` —— 用中文版页面重建 catalog（中文标题 + `sound_os2` URL）→ `experiments/komori_cn_full.json`；
  缺失中文版的页面（`knocking01`/`pc01`）用日文版条目兜底。
- `fetch_komori_cn.py` —— 爬中文版页面补中文释义 → `experiments/komori_labeled_catalog.json`。
- `tag_komori_catalog.py` → `experiments/komori_tagged_catalog.json`（策展 tag）；`subtag_komori_catalog.py` →
  `experiments/komori_subtagged_catalog.json`（7 大类 → 细分 tag）；`subtag_komori_cn.py` →
  `experiments/komori_cn_tagged.json`。
- `translate_sfx_zh.py` —— 给无官方中文的条目**用 LLM 批量生成中文释义**（输入 `experiments/komori_raw.json`，
  输出 `experiments/komori_raw_labeled.json`）；它读的 LLM 配置是 **`target/debug/.vtrim/llm.toml`**。
- `rename_emotelab_stickers.py` —— EmoteLab 贴纸**批量改名**：① 先把匹配的原件复制到 `<src>/backup/`
  （已存在则跳过、不覆盖）② 原地改名（去 `ysyk_` 前缀与时间戳、内容名空格转 `_`）③ 目标名已存在则追加 `_2`/`_3`。
  用法：`python3 scripts/rename_emotelab_stickers.py [--src <目录>] [--dry-run] [--no-backup]`。

### `asrtest/`（原型资产，不是产品代码）

- `align_srt.py` —— "qwen3 文本 + sensevoice 时间轴"双模型对齐的 Python 原型（LCS 锚点 + 锚点间局部 DP + SRT 断句）。
  跑它需要本地模型 `~/.vtrim/models/{qwen3-asr,sensevoice}/`。
- `results.json` —— 5 模型 × 7 音频的 CER/RTF 回归基准；`rust_validation.md` —— Rust 实施后的**历史验证记录**
  （68.8s 片段，锚点覆盖率 74.8%）；`README.md` 描述资产来源与使用方式。
  ⚠️ `rust_validation.md` 里那份 `[asr]` 配置是**当时**的写法（例如 `timestamp_alignment`），
  **现在配置里已经没有这个键了** —— 只当历史参考，别照抄。

## 边界

- ⚠️ **跑法用 `python3`**：本机（以及不少 Windows 环境）`python` **不在 PATH**（文档里的旧写法
  `python scripts/...` 跑不起来）。
- ⚠️ **为什么不把它们放进 `crates/xtask`**：`xtask` **刻意不依赖 python / PowerShell**（整个 crate 只用
  std + `sha2` + `zip`，为的就是把"先跑个包装脚本"那一层去掉）。所以仓库任务**不许往 xtask 里塞脚本调用**，
  门禁脚本留在本目录、用 `python3` 直接跑。
- ⚠️ **门禁不可绕过**：违规了就改代码，或改规则并写清理由（规则变了要在同一个改动里说明）。
  "这次先跳过"正是这两个脚本存在的理由。
- ⚠️ **`check_module_boundaries.py` 必须保留两条处理**：① **先剥注释再匹配**
  （规则说明自己就会写下"本目录不许出现 `yt-dlp`"，不剥注释这些文档行会全部命中 —— 上一版手写的 grep 判据
  就因此产出 **8 条假阳性**，等于没用；`(?<!:)//` 是为了不把 `https://…` 里的 `//` 当注释起点）；
  ② **`std::process::id()` 不算"开子进程"**（它只取 PID 给临时目录命名，`downloader/ytdlp.rs`、
  `bilibili/danmaku.rs` 都用了，没有任何进程被创建）—— 规则认的是 `Command` / `.spawn(`。
  分层规则用 `(?<!["\w])vtrim_server::`（`logbus.rs` 有一段测试专门断言"三个 crate 前缀都要剥掉"，
  里面写着字符串字面量 `"vtrim_server::"` —— 那是数据不是依赖，裸匹配会误报）。
- ⚠️ **加规则前先确认现状是通过的**：**两条刻意没加的**是「`views/` 里不许出现 `/api/` 字面量或 `fetch(`」与
  「`views/` 里不许用 `scopeQ(`」—— 现状大量不满足，加了就是红门禁，**别盲加**；真要收紧得先改代码
  （那是行为改动，不属于"结构标注"这一批）。
- ⚠️ `check_module_map.py` 的几条判据要点（改它之前先懂为什么不这么写会更松）：
  - **覆盖率排除 root 模块**：root 的 `dir` 是 `.`，它"覆盖一切"；算作所有者的话任何没开卡的目录都会被它兜住，
    覆盖率形同虚设。所以只有 `kind != "root"` 的模块算所有者。
  - **预算按"最坏链"算，不是按单文件**：DSH 一次注入的是"根 + 沿路径每一级卡"，总预算 65536 B；
    超预算时 DSH **先整份丢掉较宽的文件** —— 根文件超标会让全局规矩（连模块索引）全部消失，这正是要拆卡的原因。
    所以既卡单文件，也卡链。
  - **⚠️ 下限**：迁移前的根 `AGENTS.md` 有 **117 行**含 ⚠️ 的规则（踩坑换来的），拆卡时**只许多、不许少** ——
    这条防的是"清理文档时顺手抹掉警告"。
  - **索引表用显式标记**：根卡里只有 `<!-- module-index:begin -->` 与 `<!-- module-index:end -->` 之间的表格
    参与校验，免得把状态机表、防线表误当模块行。
  - `coverage_roots` 里某个模式**匹配不到任何目录**也算失败（多半是路径写错了）；
    反向覆盖也失败（有 `AGENTS.md` 没登记注册表、或注册表登记了卡但文件不存在）。
- ⚠️ `route_manifest.py` 的 exit code **始终 0**；它只做结构守卫，不证明行为等价。
- ⚠️ `render_phase_report.py` 的进度回写**每 30 帧 / 1.5s 一次**，所以"发帧+启动/页面循环/回读"三段的切点是
  "**最近的一次回写**"、不是精确边界（页面循环那一段误差 ≤ 1.5s，摊到每帧 < 0.1ms）。
  日志是 **GBK/UTF-8 混合**（Windows 中文），解码失败要退 GBK。
- ⚠️ 小森平（効果音ラボ）站方要求：**免费可商用，但禁止直接转载音效 URL** —— 下载到本地缓存使用；
  抓取**节流 1s/页，请勿加速**。素材出处与条款见 `THIRD-PARTY-NOTICES.md` 第五节。
- 本目录多数脚本是**一次性工具**（爬取 / 打标 / 改名 / 原型），不是 CI 链路的一部分；往这里加新脚本不产生
  任何运行时行为，但也**不要**让产品代码去 import 它们。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py
```

两条都要求 **0 违规**：前者看模块结构（覆盖率 / 预算 / 骨架 / 索引一致 / ⚠️ 下限），
后者看边界（15 条：分层方向 + P4 隔离 + 前端 + 成对判据）。加/改规则后**一定重跑两条**——
判据写错的表现是"看起来全过"，比报错更难发现；`-v` 会同时列出通过项。
