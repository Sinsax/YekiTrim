# vtrim-core/src/danmaku — 弹幕解析（ASS / 录播姬 XML / SRT）与密度

> 范围：`crates/vtrim-core/src/danmaku`/** ｜ 上游：`detection`（热度）、`cutting`（`clip.dm.srt`）、`vtrim-polish`（`cmd_parse`）｜ 验证：`cargo test -p vtrim-core --lib danmaku`

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
- 密度：`danmaku::analyze(path, window_secs, total_duration) -> Vec<DensityWindow>`（按
  `danmaku_window_secs`，默认 10s 归一化）、`average_density(heatmap, start, end, window_secs)`、
  `peak_density(...)`。
- 检测侧入口在 `detection::EventDetector`；切片侧由 `cutting::Cutter` 调 `load_danmaku_auto` 写
  `<clip_dir>/clip.dm.srt`。

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
- Web 侧拿弹幕路径的**统一入口是 `Workspace::danmaku_path`**（`danmaku_file` 是相对工程目录存的）；
  管线侧是 `pipeline::runner::resolve_danmaku`。别在别处 `PathBuf::from` 之后直接用。

> 改动只允许落在本目录；跨模块改动先读对方卡，并跑两条门禁。

## 验证

```bash
cargo test -p vtrim-core --lib danmaku
```

`danmaku::tests` + `parser::tests` 覆盖 ASS 解析、XML 解析与密度计算；换解析器或改判据必须全绿。
