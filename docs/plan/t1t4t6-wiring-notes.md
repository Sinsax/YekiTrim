# T1/T4/T6 接线结果与**两条必须知道的坑**（task-17）

> 面向：下一个动「文字元素动效」的人。
> 本仓侧（`F:\para\Code\YekiTrim`）接线已完成并提交；底座侧另有任务（task-18）。

---

## 一、接了什么、没接什么

| 项 | 底座通道 | 本仓接线 | 状态 |
|---|---|---|---|
| **T1 弹入** | `scale_in_from` / `scale_in_peak` / `scale_in_ms` | `[elements]` 三个键 → 字幕轨 `subtitle` | ✅ 通 |
| **T4 抖动** | `shake_px` / `shake_hz` / `shake_phase` | `[elements]` 两个键 + 相位按 style 档错开 | ✅ 通 |
| **T6 底衬** | `plate_color` / `plate_pad_px` | `[elements]` 两个键 → 字幕轨 `subtitle` | ⚠️ **写对了，但 HEAD 上不生效**（见第二节） |
| **T3 强调放大** | `Cue.scale`（**逐条**，底座已就绪） | **不接** | ⛔ **本仓无触发源**（见第三节） |

字段名**逐字照底座接口文档**，口径也照抄：
* T1/T4 → Dhampir 仓 `docs/text-motion-integration.md`
* T6 → 本仓 `docs/plan/t6-plate-interface.md`

---

## 二、⚠️⚠️ T6 在 HEAD 上**不生效** —— 底座既有 bug，已被另一个任务修

### 现象
「文字元素轨 + SRT 字幕轨并存」的工程（**= 本仓所有正常工程**）里，
`plate_color` / `plate_pad_px` **配了等于没配**：一个像素的底衬都不铺，**而报告里一行差异都没有**。

### 根因（底座）
`crates/dhampir-core/src/overlay.rs` 的 `subtitle_style` 曾经**是一个共享槽**，
在 `for track in &timeline.tracks` 循环里**每条字幕轨整份重写**，循环结束后只返回**最后一份**
⇒ **这一帧活着的最后一条字幕轨说了算**，前面轨的轨道级样式被整份覆盖。

本仓转译器产出**两条**字幕轨：
```
text_hook  ← 文字元素轨（配了底衬）
sub        ← SRT 字幕轨（plate_color = None，排在后面 ⇒ 把底衬冲掉）
```

### 隔离证明（同一条二进制、同一份工程，**只删 `sub` 轨**）
| 工程 | 红底衬像素（`plate_color=[255,0,0,255]`、白背景） |
|---|---|
| 原样（含 SRT 轨） | **0** |
| 删掉 SRT 轨 | **7676** |

### ⚠️ 影响面不止 T6
被覆盖的是**轨道级**样式（`color` / `outline` / `stroke_*` / `shadow_*` / `fade_*` /
`rise_*` / `scale_in_*` / `shake_*` / `plate_*` / 字体族与字重）。

**不受影响**：**逐条**字段（`TextItem` 的 `color` / `font_ratio` / `dx_px`）——
它们在覆盖发生**之前**就 push 进 `items` 了。
⇒ **这正好解释了实测现象：T1（逐条 `font_ratio`）/ T4（逐条 `dx_px`）活着，T6（轨道级）死了。**

> 实测（同一工程、只删 `sub` 轨）：两条都保留 T1 起始缩（宽 164/134 ≪ 稳态 272）、
> 都过冲（303/254）、都有 T4 质心抖动 —— **唯一差别是外接框宽度（272 vs 196）= 底衬那一圈**。

### 状态
**底座 task-18 正在修**（`assets`），修法是给 `TextItem` 挂 `track_style`（每条自带样式，结构上写不出"被覆盖"）。
⚠️ **我量到的是他改到一半的工作区**（`TextItem::track_style` 在 working tree、不在 HEAD）——
所以本节的结论**对 HEAD 成立**；修好之后 T6 就通了。**别去重复查一个正在被修的 bug，先看 task-18 的状态。**

★ 既有测试结构上抓不到它：`grep -c "TrackKind::Subtitle" overlay.rs` **只有 2**
（既有单测**全都只放一条字幕轨**，而覆盖需要 **≥2 条轨**才显形）。
**不是测试写得差，是测试的形状恰好绕过了它。**

---

## 三、⚠️ T3 强调放大：**底座就绪、本仓无触发源 —— 故意不接**

**不是没做，是没得接。**

* 底座通道是好的：`Cue.scale`（逐条，`None` = 1.0），乘法合成 `line.scale × enter_scale × cue.scale`。
* 但它是**逐条**的，而本仓这一档**不存在"哪一条/哪几个字要放大"这个信息**：
  ```
  tools/polish-to-dhampir.mjs:3371  const textBody = String(event.text ?? '');  ← 原样透传
  process/mod.rs 里 'class="hl"'                                    → 0 命中
  .hl 的生产者只有 cover.rs / takumi.rs（**封面**）                  → 都不在这条路
  ```
* ⚠️ **不许拿 `filter.highlight_keywords` 顶替** —— 那是**封面**的字段。
  借来的后果是「**用户改了封面高亮设置、成片里的文字动效跟着变**」，
  一个字段两处语义，而且**症状是静默的**（没人会往"我刚改的是封面"那想）。
* **真要做 = 新功能**（不是接线）：要同时定"关键词从哪来" + 改 `process` + 提示词 + 前端，**单开一批**。

---

## 四、⚠️ 抖动相位是**轨级**的 —— 同轨多条 cue **同拍**

底座 `shake_phase` 住在 `SubtitleStyle` 上（**轨级**），`overlay.rs` 的 cue 循环里读的是那个 `style`
⇒ **同一轨的所有 cue 相位必然相同**。底座自己写着：
> "契约不给'自动错开'，那要靠条目身份，而**契约层没有那个身份**"

⇒ 本仓只能**按 style 档错开**（`SHAKE_PHASE_BY_STYLE`）：
```
hook = 0（**不写键**，0 是底座默认值，写了是 R4 违规）
chapter = 1.9 / comment = 3.8 / count = 5.7
```

★ **局限（记住这条）**：本仓按 style 分 4 轨、文字元素**通常**一档一条，但**不保证**；
**同一档出现两次（两段 hook）就同拍**。这不是 bug，是契约边界。
要真正逐条错开得回底座加 `Cue.shake_phase` ——
⚠️ 那件事与 T3（`Cue.scale`）**是同一个形状**，应该**一起做**，别单独为 T4 做。

---

## 五、🔴 陈旧二进制：症状与"接线没做"**一模一样**

`crates/yekitrim-polish/src/render/dhampir.rs::find_binary()` 的顺序是
**`VTEDIT_DHAMPIR_CLI` → `<程序目录>/dhampir/bin/<exe>`（发布包形状）→ 同层 `Dhampir/target/*` → PATH**。

⇒ `target/debug/dhampir/bin/dhampir.exe` 这个 **gitignored 的本地副本**（本机那份是 10-06 的）
会**盖过** `Dhampir/target/debug/dhampir.exe` 的新构建。

**后果**：T1/T4/T6 **全部不生效、且不报错、报告零差异** ——
我第一次端到端量出「开了全部效果」与「一个都不开」**逐字节相同**，差点报成"接线全错"。

**怎么快速判定用了哪个二进制**（我用这个手法定位的）：
```bash
python3 -c "
d=open(r'<那个 dhampir.exe>','rb').read()
print('scale_in_ms:', b'scale_in_ms' in d, '| plate_color:', b'plate_color' in d)"
```
新版会有这些字符串；旧版没有。

**以后跑本仓端到端前**：确认 `<程序目录>/dhampir/bin/dhampir.exe` 是当前构建
（或设 `VTEDIT_DHAMPIR_CLI` 指向新构建）。

---

## 六、另一条环境坑：`%TEMP%` 写不了

本机 `%TEMP%` 不可写 ⇒ 底座报 `subtitle_raster_failed: os error 5`（拒绝访问）。
**绕法**：`TMPDIR` / `TEMP` / `TMP` 指一个**仓内**目录 + `--font-dir` + `--font-file`。
⚠️ 不设的话出片会**静默失败**（字幕一行都画不出来），报告里只有一条 issue。

---

## 七、判据在哪

| 层 | 判据 |
|---|---|
| 配置结构 | `project::config::tests::只配了动效字段也必须写进_toml`（**逐个字段单独配一次**） |
| JSON 键（R4 第一层） | `check-yekitrim-translator.mjs::checkTextMotion` ⑱：全默认 ⇒ **一个键都不多** |
| 求值（R4 第二层） | 同上 ⑲：默认 ⇒ 单位变换 |
| 逐字段写出 | 同上 ⑳–㉖（七个字段各自独立一档） |
| `is_empty()` 那个坑 | 同上 ㉗（只配 `shake_px`、不开 `typewriter` ⇒ 必须写出去） |
| 源码文本兜底 | 同上 ㉘（照 ⑬–⑰ 的写法） |
| 脏值拒绝 | 同上 ㉙（3 个数 / 5 个数 / alpha 越界 / 毫秒小数 ⇒ **不许写出去**） |

变异：`--mutate` **73/73 全红**（基线 66 + 本次 7）。
⚠️ 首次跑有 **2 条没红**（颜色不校验长度 / `scale_in_ms` 不校验整数）——
那是**判据的洞**，补了 ㉙ 那一组才红。
