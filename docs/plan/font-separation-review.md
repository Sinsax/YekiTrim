# 字体职责分离 + 输入层收敛 · 方案评估与交接

> **目标（用户 2026-10-02 澄清，本文档按此重排）**
> 1. **输入层收敛**：所有用字体的功能，字体输入走**同一个标准接入**，以保证字体库正确性。
> 2. **职责分离**：字体不一定含 emoji，所以**字体（管字）与 emoji（管表情）分开**。
>
> 评估方式：逐条到真实链路取证。所有"已验证"都标了 file:line。
> ⚠️ **v3 更新**：按用户澄清重排目标；§0 的三条事实（含 v2 反转的 P0）全部保留。

---

## 一、输入层现状：字体到底从哪几个口进来

按"谁提供一个字体"盘点，今天有 **7 个输入口，3 种不同形态**：

| # | 输入口 | 形态 | 取值规则 | 落点 |
|---|---|---|---|---|
| 1 | `[style.subtitle] font`（polish.toml） | **库 id 字符串** | 空 = 系统默认 | 字幕（预览+成片） |
| 2 | `[cover] font`（polish.toml） | **库 id 字符串** | 空 = 系统栈 | 封面标题 |
| 3 | `POST /api/polish/project` 的 `subtitle_font` | 库 id | 前端下拉 | 写回 #1 |
| 4 | `POST /api/polish/cover-config` 的 `cover.font` | 库 id | 前端下拉 | 写回 #2 |
| 5 | `VTEDIT_DHAMPIR_FONT`（env） | **文件绝对路径** | 无 | 字幕成片，**最高优先** |
| 6 | `cover.rs::EMOJI_FONT`（常量） | **写死的系统族名栈** | 无 | 封面 emoji 特效层 |
| 7 | `takumi.rs::ensure_emoji_fallback`（常量） | **写死的系统族名栈** | 无 | 封面 CSS 兜底 |

**问题正是你指出的两点，而且比预想的更明确：**

- **形态不统一**：#1–#4 是"库 id"（库内相对标识），#5 是"文件绝对路径"，#6/#7 是"系统族名"。
  三者语义完全不同、互不校验。选了一个 id，成片却可能被 `VTEDIT_DHAMPIR_FONT` 整体顶掉（#5 优先于一切）。
- **兜底链各写各的**：`server.rs::SYSTEM_FONT_STACK`（`'LXGW WenKai','Microsoft YaHei','Noto Sans SC',sans-serif`）、
  `cover.rs::EMOJI_FONT`（`'Segoe UI Emoji','Apple Color Emoji','Noto Color Emoji',sans-serif`）、
  `PolishPreview.vue:1347`（又抄一份字面量）、`takumi.rs:199`（再抄一份）。
  ⇒ **同一件事 4 份实现**，且靠注释互相提醒"要同序"（`server.rs:211-213`、`PolishPreview.vue:1416-1417`）。
- **无人持有"emoji 该由谁补"的知识**：`emoji: bool` 元数据在 `fonts.rs:149` 躺着，
  `/api/polish/fonts` 也回了（`polish.rs:1287`），**但没有一个消费点读它**（全仓 grep 实读确认）。

⇒ **收敛的正确落点 = 在 `vtrim-core::fonts` 造一个"字体解析唯一入口"**，
把上面 7 个口全部改成"问它要"：给定 `id | 空`，返回**该用哪些字体 + 各自角色（正文 / emoji 补充 / 兜底）**。
这样"选了库里没有的字体""emoji 由谁补""成片和预览是不是同一套"这三个问题，都只有一处答案。

---

## 二、职责分离：emoji 不是"一支可选字体"，是一个独立槽位

按你的第 2 点，设计应明确成：

- **文字槽位**：用户选（或默认）的那支，管"字"。
- **emoji 补充槽位**：由库自动选（`emoji == true` 且已装），管"表情"，**用户不选、不参与默认**。
- 二者**拼成一个栈**才交给各消费点 —— 而不是让用户在一个下拉里从混合候选里挑。

这个抽象今天是**部分存在、但没人执行**的：
- `preferred_fallback_family`（`server.rs:233-242`）**已经显式跳过 emoji**（注释"自带字体里只有它不是 emoji"）
  —— 说明代码已经"知道"emoji 不该当正文；
- 但 `fontOptions` 下拉（`PolishView.vue:560-570`、`CoverDesigner.vue:441-453`）**没有过滤 emoji**
  ⇒ 用户仍能把 emoji 字体选成正文默认。**这就是职责没分离的现场。**
- `FontLibrary.vue:94-104` 对 emoji 卡片照样给「设为默认」。

⇒ 方案 C9/C10/A4 三条**方向完全正确**，是本次澄清的直接落实。评估结论不变：**采纳**。

---

## 三、逐条裁定（对方案第二节）

| 条目 | 裁定 | 依据 / 修正 |
|---|---|---|
| **A1** `fonts::emoji_font(root)` 唯一权威 | ✅ 采纳，**且应升级** | 建议直接做成**字体解析唯一入口**（本文档 §四 S1），不止一个 `emoji_font` 访问器。 |
| **A2** 栈尾插 emoji 族 | ⚠️ **部分采纳** | 对 **#2 预览** 成立；对 **#1 字幕成片无效**（事实 1）；对 **#3 封面**基本无效（事实 3，注册序才是决定因素）。**不能宣称"一处改动喂饱三个消费者"。** |
| **A3** `/api/polish/fonts` 加 `emoji_font` | ✅ 采纳 | 纯增量。`polish.rs:1310-1318` 是唯一构造点。 |
| **A4** `is_default` 拒绝 emoji | ✅ 采纳，**两处一起改** | ① `fonts.rs:928 set_default` 加拒绝；② **`preferred_fallback_family`（`server.rs:225-250`）用 `find(\|e\| e.is_default)` 选栈首** —— 手改 FONT.toml 把 emoji 设成 default 会让**整个栈首变 emoji 族** ⇒ 全库中文豆腐。方案说的"扫描时强制视为 false"**必须落在这一处**。 |
| **A5** 字幕页 `font_face_css(&[font, emoji_font])` | ⚠️ **改了也没用** | `server.rs:390` 对应**浏览器预览**页；字幕成片走底座（事实 1），`@font-face` 到不了底座。预览是浏览器、系统装了 emoji 就自带 → 收益≈0，**建议降级/砍掉**。 |
| **B6** `EMOJI_FONT` → 动态 | ✅ **采纳，且属"输入层收敛"正题** | 正是 §一 的 #6/#7 两个写死口。但要认清收益：在 Takumi 上"注册了就能命中"（事实 3），动态化的**主要收益是"单一来源"而非"出图变化"**。 |
| **B7** `ensure_emoji_fallback` 动态化 | ✅ 采纳 + **必修 bug** | 见 R2（正则不幂等）。 |
| **B8** 注册序 emoji 收尾 | ✅ **封面侧的主菜** | 唯一真正决定 Takumi"挑哪支"的旋钮（事实 3）。⚠️ 必须与"颜色"分开验收（事实 2）。 |
| **C9** 两个下拉过滤 emoji | ✅ 采纳 | 职责分离的**直接落实**（§二）。`PolishView.vue:560-570`、`CoverDesigner.vue:441-453`。 |
| **C10** FontLibrary 隐藏「设为默认」 | ✅ 采纳 | `FontLibrary.vue:94-104`。文案行内 ≤20 字，细节进 `title`。 |
| **D11** P0 是否换 COLR | ❌ **结论反转** | **必须换字体**（事实 2）。首选：退到 CBDT 版 Noto Color Emoji v1.x。 |
| **D12** 单测 | ⚠️ 两条会自己被顶翻 | 见 R1。 |

---

## 四、三条必须先看的事实

### 事实 1：Dhampir 字幕**没有**字体回退 —— A2「栈尾插 emoji 喂饱 #1」不成立 ❌

1. `crates/vtrim-polish/src/render/dhampir.rs:445-456` —— `find_font` 是**优先级列表挑一个赢家**，命中即 `return`。
   `font_search_order`（`:398-411`）只是把 `fallback_stack()` 按 `,` 拆成这张优先级表。
2. CLI 只传**一个**字体文件：`--font-file` + `--font-bold-file`（`:1003-1034`）。
3. 每轨族名来自转译器，**只有一个**：`tools/polish-to-dhampir.mjs:3325`（字幕）、`:3517`（弹幕）。
4. **底座源码在 `F:\\para\\Code\\Dhampir`**（实读）：`text_overlay.rs:368-387` `pick_font` → `Option<PathBuf>`，**一个文件**；
   `text_raster.rs:413-497` 生成 ffmpeg `drawtext=fontfile=<单支>` —— **无回退链、无 fontconfig**；
   `text_raster.rs:58` 明写"不做彩色 emoji 字形"；全仓无多字体选项。
5. ⇒ 栈尾插 emoji **零效果**；若它成了首个可解析项，`pick_font` 会把**整条轨**收敛到 emoji 字体 ⇒ 中文全豆腐。

**⇒ 字幕 emoji 卡在底座能力，本仓改输入层无论如何都修不好。**

### 事实 2：自带 emoji 字体是 **COLRv1**，Takumi 只认 **COLRv0** ⇒ 出**单色剪影** ❌⚠️

**2a. 实测字节** —— 我解析了 `~/.vtrim/fonts/NotoColorEmoji/font.ttf`（25,332,736 B，16 表）：
```
COLR off=3808684 (len 1094045)   CPAL off=4902732   SVG off=4959708 (len 20288281)   glyf off=348532
HAS CBDT: false   HAS CBLC: false   HAS sbix: false
COLR version = 1        ← COLRv1
```
⇒ 方案 D11 说"NotoColorEmoji 是 **CBDT 位图**"**描述过时**：上游 Noto Color Emoji v2.x 已迁 COLRv1。

**2b. Takumi 写死 COLRv0** —— `takumi-core-0.25.0/src/resources/glyph.rs:440-443`：
```rust
let color_glyph = self.color_glyphs.get_with_format(glyph_id, ColorGlyphFormat::ColrV0)?;  // 写死 V0
```
skrifa 0.44 支持 COLRv1，但 Takumi 主动只要 V0。三级解析（`glyph.rs:417-433`）对 COLRv1-only 字体：
① 位图 strikes 无表 → `None` → ② **只问 ColrV0** → `None` → ③ `glyf` 描边 → ✅ **单色剪影**。
NotoColorEmoji **带 3.4 MB `glyf`**，所以出的是**实心剪影**——不是彩色、不是位图、也不是豆腐块。
**最坏的失败形态：看着"有东西"，所以没人会去查。** SVG-in-font 也没实现。

**2c. 为什么仓库自己的实验没抓到（关键陷阱）**
- `takumi.rs:192-200` 把 `'Segoe UI Emoji'` 插在栈**最前**；
- `docs/plan/takumi-cover-capability.md:138` 写着实验台用 **`seguiemj.ttf`**，`:343`/`:393` 据此宣称 emoji "逐像素级"通过；
- **我解析了 `C:/Windows/Fonts/seguiemj.ttf`：`COLR version = 0`** —— 正好是 Takumi 支持的那档。

⇒ 当年验收用的是 **COLRv0 系统字体**，对自带那支 **COLRv1** 字体**零证明力**。
本机上 `Segoe UI Emoji` 排第一且系统装了它，于是"碰巧是对的"。

**2d. 修正后结论**：COLRv0 ✅ / CBDT·sbix ✅ / **COLRv1 ❌** / SVG-in-font ❌
⇒ **"不用换格式"作废**。首选：**退到 CBDT 版 Noto Color Emoji v1.x**（可分发 + Takumi 支持）。
⚠️ 这条同时影响**封面 emoji 特效层**（`cover.rs:634-646` 用同一字体栈），不只是标题。

### 事实 3：Takumi fallback 桶 = **注册序全集**，CSS 族名几乎不起作用 ✅

- `takumi-raster-0.5.1/src/render.rs:167,431`：`snapshot_with_fallbacks(font_families.as_ref())`；
- `crates/vtrim-polish/src/render/takumi.rs:565-572`：本仓**没设 `font_families`**（全仓 grep = 0）⇒ `None`；
- `takumi-core-0.25.0/src/resources/font.rs:468-475`：`None` ⇒ 桶 = `self.order` = **全部已注册字体按注册序**。

⇒ **"挑哪支"由 `cover_font_files()` 的注册顺序决定，不是 CSS 族名。**
（`tests/takumi_font_id.rs:10-27` 已记此结论，与实读一致。）
⚠️ 但"注册了 emoji 字体"≠"出彩"（事实 2）——**B8 解决"字形有没有被找到"，解决不了"是不是彩色的"，必须分开验收。**
⚠️ `ensure_emoji_fallback` **只扫 `<style>` 块**（`takumi.rs:60-84`），而特效层 emoji 是 **inline style**（`cover.rs:642`）
⇒ **特效那层永远扫不到**，B6/B7 必须分清这两条。

---

## 五、风险与冲突

### R1 ⚠️ A2 与"空库逐字不变"单测/零回归承诺**直接冲突**
- `server.rs:544-562`：**回落态必须与 `SYSTEM_FONT_STACK` 同族名同顺序**；
- `server.rs:529-535` 更直接：`assert_eq!(stack.matches(',').count(), 4, "四段…")` —— **插一段就红**；
- 本机（及任何装过自带字体的机器）库里就有 NotoColorEmoji ⇒ 按 A2 插 emoji 必然多一段。
- ⇒ **必须选一条**：(a) 更新这两条单测并**收窄**"零回归"范围；或 (b) 不动 `fallback_stack()`。
  **这是最大的未决问题，不能留到实现时才发现。**

### R2 ⚠️ `ensure_emoji_fallback` 正则**不幂等**（既有 bug，实测）
```
.fx-item.fx-emoji{font-family:'Segoe UI Emoji','Apple Color Emoji','Noto Color Emoji',sans-serif;}
  → …'Noto Color Emoji','Segoe UI Emoji','Noto Color Emoji',sans-serif;    ← 多一段
```
`takumi.rs:196-199` 无条件插前缀。⇒ 改成**"已在其中就不插"**。

### R3 系统 emoji 族名在 Takumi 上是**死代码**
`Segoe UI Emoji` / `Apple Color Emoji` 在 Takumi 上**不解析**（不读系统字体、只认注册过的），
真正起作用的是注册序（事实 3）。⇒ B6 写"系统栈兜底"对 Takumi **没有兜底作用**，别当保险。

### R4 跨模块面比方案写的大
补：`render/dhampir.rs`（若动 `fallback_stack()`，emoji 族可能进 `font_search_order` **抢占字幕主体**，必须评估）、
`tests/takumi_font_id.rs`（B8 改序后要复跑）、两条单测（R1）、
`docs/plan/takumi-cover-capability.md`（§2c 的"emoji 已验证"要加 COLR 版本限定）、
`docs/plan/font-library.md`（**目前只到 §21**，实读确认）、
`fonts.rs::BUILTIN_FONTS`（换字体则 URL/sha256 随改）。

---

## 六、建议落地顺序（按你的两个目标重排）

```
S0  真机复现 + 颜色实测（半天，最先做）
    ├─ 出一张带 emoji 的封面，**肉眼确认是不是单色剪影**（预期：是）
    ├─ 把栈里 Segoe UI Emoji 拿掉再出一张（隔离"系统字体在兜底"这层假象）
    └─ 出一条带 emoji 的字幕成片（预期：不支持 —— 结掉 #1 预期，写进文档）

S0.5 emoji 字体选型（取决于 S0）
    └─ 首选 CBDT 版 Noto Color Emoji v1.x（可分发 + Takumi 支持）
       ⚠️ 必须**换新 id**：下载三入口幂等、"已装好的跳过"，复用同 id 老机器不会重下

S1  输入层收敛（目标①的核心）—— 在 vtrim-core::fonts 造"字体解析唯一入口"
    ├─ 一个函数吃 (id 或空) → 返回 {正文族, emoji 补充族, 兜底链}（角色化的单一来源）
    ├─ A1 emoji 槽位（至多一支、id 稳定序、只取已装）
    ├─ A3 /api/polish/fonts 暴露它；A4 is_default 双向拦截 emoji
    └─ 现有 4 份写死兜底（SYSTEM_FONT_STACK / EMOJI_FONT / 预览字面量 / ensure_emoji_fallback）
       全部改为问这个入口
    写范围：crates/vtrim-core/src/fonts.rs, crates/vtrim-server/src/api/polish.rs,
            crates/vtrim-polish/src/render/server.rs

S2  前端职责分离（目标②的核心）—— C9 + C10
    ├─ 两个下拉过滤 f.emoji（emoji 退出正文候选）
    ├─ FontLibrary：emoji 卡片隐藏「设为默认」+ 区块文案说明"emoji 自动补充"
    └─ 预览 .cap 零改动（服务端栈已带）
    写范围：webui/src/views/PolishView.vue, webui/src/components/{CoverDesigner,FontLibrary}.vue
    ⚠️ 必须 npm --prefix webui run build 重建 dist（dist 已入库）

S3  封面收口（B8 为主，B6/B7 顺带）—— 注册序 emoji 收尾
    写范围：crates/vtrim-polish/src/cover.rs, src/render/takumi.rs

S4  栈尾 emoji（A2）—— 取决于 R1 的裁定
    若做：改 server.rs + 更新两条单测 + 真跑封面 golden 对拍
    若不收窄零回归承诺，**建议不做**

S5  验收 + 文档（font-library.md 新增 §22；dhampir-parity-register.md 记"字幕 emoji 不支持"；
    takumi-cover-capability.md 给 emoji 结论加 COLR 版本限定）
```

---

## 七、验证清单

```bash
cargo check --workspace --all-targets                       # 0 warnings
cargo test -p vtrim-polish --lib render                     # server.rs 那两条单测在这
cargo test -p vtrim-polish                                  # takumi_font_id.rs 在 tests/
cargo test -p vtrim-core --lib fonts                        # fonts.rs tests / tests_v2
cargo test -p vtrim-server                                  # /api/polish/fonts 响应形状

node scripts/check-cover-takumi.mjs --golden scripts/fixtures/cover-golden   # 封面真渲染

npm --prefix webui run build
node webui/scripts/check-cover-stage.mjs                    # ⑦ 钉"只有一条替换链"

python3 scripts/check_module_map.py
python3 scripts/check_module_boundaries.py
```

⚠️ 本机 **bash 坏了**（`couldn't create signal pipe, Win32 error 5`），用 `pwsh` 或直接用工具；
`python3` 别写 `python`。

---

## 八、待你拍板

1. **输入层收敛做到哪一档？**（目标①的范围）
   - (a) **只收敛"字体选择"这条链**（推荐）：4 份写死兜底 + 下拉/默认口径统一，不碰底座输入形态。
   - (b) 连 `VTEDIT_DHAMPIR_FONT`（文件路径形态）一起纳入统一入口 —— 更彻底，但要动底座 CLI 契约。
2. **emoji 字体换不换？**（事实 2）
   - (a) **换 CBDT 版 Noto Color Emoji v1.x**（可分发 + Takumi 支持）。**推荐**。
   - (b) 保持现状 + 只靠系统 COLRv0 字体（Linux 常没有）—— 等于放弃自带兜底。
   - (c) vendoring Takumi 开 COLRv1 —— 成本最高。
3. **字幕 emoji 不支持，是否接受？**（事实 1）(a) 接受并记档 **推荐** / (b) 推给底座 / (c) 转贴纸层绕行。
4. **R1 怎么裁？** A2 要不要动 `fallback_stack()` 本身。**我倾向先不动**，只在封面（B8）+ 换字体（S0.5）落地。

---

## 九、一句话交接

**你的两个目标是成立的，方案的大方向也对；但落地时踩了三条硬事实：**
**①字幕 emoji 卡在底座（本仓改输入层无效）；②自带 emoji 字体是 COLRv1 而 Takumi 只认 COLRv0（出单色剪影）；**
**③封面靠注册序选字体、不靠 CSS 族名。**
**所以：输入层收敛（S1）与职责分离（S2）照做、C9/C10/A4 全部采纳；但 A2 要降级、B8 提到主菜，**
**并新增 S0.5 换字体。**

---

## 十、自查记录（我自己错在哪，留作教训）

初版曾写"**P0 已结案：CBDT 能出彩，不用换 COLR**"，**错了**：
- 我验证了 **Takumi 的能力**（`font.rs:210-214` 接受 CBDT+CBLC、`text_drawing.rs:342` 按 RGBA 上色）—— 这部分没错；
- 但**没验证"我们实际发的那支字体是什么格式"**，直接采信了 D11 的描述；
- 实测字节才发现是 **COLRv1**，而 Takumi 写死只问 **ColrV0**（`glyph.rs:443`）。

⇒ **教训与 P0 同构：验证"通道支持 X" ≠ "我们的数据是 X"。**
这正是原方案 P0 想防的坑，我在评估里又踩一次 —— 所以 S0 验收必须**看得见颜色**，
不能只看"有没有出图"或"有没有形状"。
