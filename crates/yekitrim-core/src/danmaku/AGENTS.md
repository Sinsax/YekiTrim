# yekitrim-core/src/danmaku — 弹幕解析（ASS / 录播姬 XML / SRT）与密度

> 范围：`crates/yekitrim-core/src/danmaku`/** ｜ 上游：`detection`（热度）、`cutting`（`clip.dm.srt`）、`yekitrim-polish`（`cmd_parse`）｜ 验证：`cargo test -p yekitrim-core --lib danmaku`

## 职责

把三种弹幕文件解析成同一种 `DanmakuEntry` 列表，再按时间窗归一出密度曲线（热力图），
给事件检测算 `popularity`、给 Cutter 生成 `clip.dm.srt`。**三种格式都要认** ——
B站导出 ASS/SRT、录播姬落 `danmaku.xml`，用户手上有什么就得吃什么。

## 入口

- `danmaku::load_danmaku_auto(path)` —— **唯一该用的入口**：`.xml`（录播姬/B站标准 XML）→
  `parse_bililive_xml`；含 `[Events]` + `Dialogue:` → `parser::parse_ass`；其余按 SRT。
- `danmaku::load_danmaku(path)` —— **只走 ASS**（`parser::parse_ass`），给"订正上下文"这类已知 ASS 的场合。
- `danmaku::parse_srt_file(path)` / `parse_srt_danmaku(content)` / `parse_bililive_xml(content)` /
  `parser::parse_ass(path)`；条目类型 `danmaku::DanmakuEntry`（`start`/`end`/`text`/`color`/`y`/
  `travel_duration`）。
- 在屏窗口：`travel_duration_for(text)`（滚动时长，**唯一一份公式**，按 UTF-8 字节数）、
  `clamp_display_windows(&mut entries) -> usize`（把窗口末尾抬到"至少够它滚完"）、
  常量 `DANMAKU_FADE_IN_SECS`（渲染侧 `fadeIn` 的那 0.3s）。
- 密度：`danmaku::analyze(path, window_secs, total_duration) -> Vec<DensityWindow>`（按
  `danmaku_window_secs`，默认 10s 归一化）、`average_density(heatmap, start, end, window_secs)`、
  `peak_density(...)`。
- 检测侧入口在 `detection::EventDetector`；切片侧由 `cutting::Cutter` 调 `load_danmaku_auto` 写
  `<clip_dir>/clip.dm.srt`。
- **§10 情绪轨道**（`emotion` 子模块，**公开**）：`emotion::track(&[DanmakuEntry], window_secs,
  total_duration) -> Vec<EmotionWindow>`（**L1 词表 + L2 颜色**，零成本、可解释）；
  `Emotion{joy,shock,laugh,awe}` / `EmotionWindow{start,emotion,count,hits}` /
  `score_subtitle(text) -> Emotion`（**无弹幕时的判据**）。消费方是 `yekitrim-polish` 的
  `process`（Step 1c2，写 `data/emotion.json`）。

## 边界

- **别再用 `parse_ass` 直读用户给的 `danmaku_file`**：真实 B站/录播姬弹幕是 XML，`parse_ass` 对它
  **吐 0 条且不报错**。踩过的两处：Cutter 曾单独用 `parse_ass`（XML 弹幕只有热力图、没有
  `clip.dm.srt`）、精修 `cmd_parse` 曾走 SRT 分支（B站 XML 被解析成 0 条）。统一走 `load_danmaku_auto`。
- ⚠️ **`parse_srt_file` 只认 SRT**（内容看着像弹幕 XML 会直接 `bail!`，把"静默 0 条"变成一眼可见的失败）：
  `danmaku_file` 指向 `.xml`/`.ass` 时正确入口是 `load_danmaku_auto`。
- **别再各自抄一份格式判断**：扩展名 + `<?xml` / `<d p=` / `[Events]`+`Dialogue:` 的判据只在
  `load_danmaku_auto` 里有一份，新增调用点直接用它。
- **别把密度算成绝对条数**：`popularity` 要的是**相对量**（弹幕按全片最热的窗口归一），
  归一化口径见 `detection` 与 `cutting` 的卡；改这里的窗口口径会同时改热度排序与 `rank`。
- 同一份弹幕可能被检测与切片各读一次（`analyze` / `load_danmaku_auto`）：解析要能重复调用、
  不许有跨调用的可变状态。
- ⚠️ **在屏窗口必须够它滚完**（`clamp_display_windows`）：渲染侧的渐隐钉在窗口末尾
  （`d.end - 0.2` 起渐隐、`t >= d.end` 整条丢掉），而滚动由 `travel` 决定 —— 两口钟不耦合时，
  一条 8 秒窗口、10.4 秒 travel 的弹幕会在**还没到左边缘**时消失（实测距左边缘 337px 处就在渐隐、
  39 字节那条在屏幕中段 728px 处没了）。录播姬 XML 给的窗口是固定 8 秒，中文弹幕几乎全部命中。
  调用点：精修 `cmd_parse`（`assign_danmaku_tracks`）与 `process`（`normalize_danmaku_windows`，
  老工程自愈）；底座那条腿是 `tools/polish-to-dhampir.mjs::danmakuExitMs`（同一规则的另一份实现，
  由 `scripts/check-yekitrim-translator.mjs` 对拍常数）。
- ⚠️ **窗口比较带 1 ns 容差，别写回严格相等**：`serde_json` 读 f64 会**差 1 ulp**
  （`"10.700000000000001".parse::<f64>()` 与 `serde_json::from_str::<f64>()` 的位不同），
  严格相等会让每次 `process` 都重写一遍 `data/danmaku.json` —— 内容不变、mtime 一直翻新，
  而首页「切片总览」按 mtime 排序，老片段会被一直顶在最前面。见 `WINDOW_EPS_SECS`。
- ⚠️ **情绪轨道的三条纪律**（`emotion.rs`，规格 `docs/plan/emotion-narrative-engine.md` §2）：
  ① **不是热度** —— `popularity` 只知道"这里很热"，情绪要知道"这里是**震惊**还是**爆笑**"，
  两者该配的画面完全不同；② **无弹幕必须退到字幕**，且**在日志里明说用了哪条判据**（不许静默降级）；
  ③ **信号弱时降低演出强度，不是硬演**（`Emotion::is_weak`，阈值 0.15）。
  ⚠️ 窗口合成用 **max 不是平均**：100 条无关弹幕里夹 1 条"？？？"，平均会把它冲淡到看不见，
  而那 1 条恰恰是"震惊"的证据。⚠️ 彩色判定用**饱和度**而不是"!= 白"
  （近白 `#f0f0f0` 是默认色系，只看"非白"会让每条白弹幕都算"花了钱"）。
  ⚠️ `hits` 字段**不是调试便利**：情绪会驱动画面（静音/闪白/大字），
  "为什么这里突然静音了"必须有可查的答案。
- Web 侧拿弹幕路径的**统一入口是 `Workspace::danmaku_path`**（`danmaku_file` 是相对工程目录存的）；
  管线侧是 `pipeline::runner::resolve_danmaku`。别在别处 `PathBuf::from` 之后直接用。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p yekitrim-core --lib danmaku
```

`danmaku::tests` + `parser::tests` + `emotion::tests` 覆盖 ASS 解析、XML 解析、密度计算与情绪判定；
换解析器或改判据必须全绿。⚠️ 写情绪用例时注意**按中点归窗**：`end = start + 4.0` 的条目会落到
窗口 2 而不是窗口 0（踩过：报出来像"词表串了"，其实是读错了窗口）。
