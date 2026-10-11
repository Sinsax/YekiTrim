# 底座那条腿（`dhampir`）的实测规矩

> 从 `crates/yekitrim-polish/src/render/AGENTS.md` **原样外置**（2026-10-11，决策 #161）。
> 原因：`check_module_map.py` 的**最坏指令链预算**（57344 B）在 HEAD 上就被撑爆了 220 B，
> 而 `polish-render` 卡自己 24564 B 已贴着自己的 24576 B 上限 —— **正文一字不删，挪到这里**。
> 卡里留一行指针。**改这些规矩时改这里，别往卡里搬回去。**

- ⚠️ **底座那条腿（`dhampir`，**默认通道**）的三条实测规矩**（`render/dhampir.rs`，2026-09-30 全链真跑过）：
  - **帧数从转译产物里读**（`sequence_frames` = 所有层 `end` 的最大值），**不许拿上层那个
    `duration × 30` 的估算**：转译器写的时间基是**素材自己的**（demo 是 `60/1`），拿 30 算 `--to`
    会让 12 秒的片子**只出 6 秒**，而音轨照旧 12 秒（合流之后更难看）。实测踩到过。
  - **底座的 stdout 落文件 + 按 offset tail，不许用管道**：它打 NDJSON（`start`/`progress`/`done`），
    要翻成 `frames A..B: NN%` 才有人认（不翻的话 WebUI 全程 0%）。它同样会拉起 ffmpeg 孙进程，
    所以照外层那条纪律：写进 `_temp_dhampir_progress.ndjson` 再 tail（带 `_temp_` 前缀，
    失败时被 `cleanup_render_leftovers` 收掉）；stderr 直接继承。
  - **`cmd_render` 开头必须把工程目录绝对化**（`std::path::absolute`）：`resolve_paths` 把
    `project.output` 归一成 `<工程目录>/export.mp4`，工程目录是**相对**的时候，合流前那一步
    "不是绝对路径就再 join 一次工程目录"会把目录名**拼两遍**（`<dir>\<dir>\export.mp4`
    → `No such file or directory`）。CLI 的默认输入就是 `.`，所以这条一定会被踩到。
  - **找二进制时两种失败含义不同**：`VTEDIT_DHAMPIR_CLI` 指错 = 当场报错；`<程序目录>/dhampir/bin/`
    不在 = **继续往下找**（同层 `Dhampir/target/{release,debug}` → PATH）。混成一条 `bail!`
    会让源码树下**永远找不到底座**（`current_exe()` 总能算出那个约定路径）。
  - ⚠️ **屏蔽区间只能剪一次**：出片那条腿转译时带 `--keep-muted`（时间线**整条**保留），
    由 `cmd_render` 尾部那道 `Ffmpeg::cut_ranges` 按**源时间**剪；预览那条腿（`polish_dhampir_project`）
    相反，必须留压缩时间线（客户端要跳）。**两边都剪 = 成片短一大截 + 音轨错位**
    （实测：12s 的片子带 3s 屏蔽，修复前出 6.0s、修复后 9.0s = 与 chrome 逐毫秒一致）。
  - ⚠️ **字体：栈的顺序 + 粗体，两件都不能漏**（2026-10-01 修完"字体不对"）：
    `find_font(style.subtitle.font) -> (常规, Option<粗体>)`，逐支按
    **① 字体库（`fonts::resolve_file`）→ ② 系统字体目录（族名归一化 + 别名表 `Microsoft YaHei`→`msyh.ttc`）** 解析；
    顺序 = 项目选的那支 + **`fallback_stack()`（动态兜底栈，唯一权威）** ——
    首项随**库里"设为默认"那支**走，库里没有可用中文时才回落 `SYSTEM_FONT_STACK`。
    ⚠️ **别改回读常量**：那会让「设为默认」对成片永远无效。栈还必须与 `PolishPreview.vue` 的 `.cap`
    同源（经 `/api/polish/fonts` 透传）——它仨曾各写各的 ⇒ 同一工程**预览雅黑、成片 Noto**。
    ⚠️ **「库里默认哪支」判据只此一处**（2026-10-06）：`fonts::default_text_family`。原先各抄一份
    `.find(|e| e.is_default)`，而**最该问它的 `cmd_init` 一次都没问过** ⇒「每次切出切片，字体都不是
    字体库设置的默认字体」。新增消费点一律走它。⚠️ 这条栈只决定**挑哪一支**、不是"能逐字形回退"
    （底座只收到**一个** `--font-file`），见 `font-library.md` §23.6。
    ⚠️ **`font_family` 必须真族名**（2026-10-08，0.2.0 起）：libass **按族名**找字体，而 `polish.toml`
    存的是**库 id**；两者**真的会不一样**（实测 id `GenJyuuGothic-Bold` vs 族名 `Gen Jyuu Gothic Bold`），
    不一样时 libass **不报错**、静默落 `ArialMT`（`lines_failed: 0`）。所以出片前把 `id→族名`
    经 `--font-families` 交给转译器（`write_font_families`，**库外原样**）。
    整个字体库照旧交给底座（`--font-dir`，它按每条轨的 `font_family` 在目录里找）。
    ⚠️ **`--font-bold-file` 必须传**：转译器给字幕写 `font_weight: 700`、弹幕 `600`（与预览的
    `.cap{font-weight:700}` / `.dm{font-weight:600}` 同口径），而底座**不合成粗体** ——
    它只在 `>= 600` 时去要这个文件，拿不到就退回常规体 ⇒ **成片笔画比预览细一圈**。
    所以 `find_font` 要配出**同一族的粗体**（`msyh.ttc`→`msyhbd.ttc`；可变字体如 `NotoSansSC-VF.ttf`
    没有独立粗体 ⇒ `None`，此时 `describe()` 与就绪位会明说"没有粗体那一支"）。
  - **就绪位与严格模式**（2026-09-30 M3）：`dhampir::readiness(font)` 是**唯一的**"这台机器能不能出片"
    判断（八项各查一次：二进制/转译器/字体/node/**ffmpeg+libass**/预览产物/版本契约；`deep=false`
    那条便宜腿不跑子进程）——`/api/polish/dhampir-status`、`/api/health` 的 `dhampir_ready`、
    `polish doctor`、前端提示都读它。**别各写一份**（症状会是"医生说好、渲染却失败"）。
    `render_video_only(..., strict)` = `polish render --strict`：转译报告里 `DROP > 0` 就**拒绝出片**
    并逐条列出（报告由 `prepare_project_reporting` 带回；`TranslateReport::parse` 只认那几行的形状，
    认不出就当 0，**绝不让报告把出片搞崩**）。
