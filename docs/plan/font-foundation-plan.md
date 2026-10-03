# 字体基础建设 + 逐消费点对接 · 实施计划

> **用户口径（2026-10-02）**：「先做基础性建设，然后依次对接和校验每个使用字体的功能」
>
> 这份计划取代前一版"按 P0-P5 排"的顺序。核心改变：
> **地基（单一入口 + 可校验性）先立起来，之后每接一个消费点都有独立判据可验，不再"一把改完对像素"。**
>
> 三条硬事实（实读证据，见 §五）决定了"能验什么、验不了什么"，所以地基必须把它们**编码成判据**，
> 而不是靠注释提醒。本文档所有"已验证"都标了 file:line。

---

## 一、为什么必须先做地基（而不是直接改 P1）

现状是**同一件事最多 4 份实现、靠注释互相提醒"要同序"**：

| 写死/实现 | 位置 | 内容 |
|---|---|---|
| `SYSTEM_FONT_STACK` | `render/server.rs:213` | `'LXGW WenKai','Microsoft YaHei','Noto Sans SC',sans-serif` |
| `EMOJI_FONT` | `cover.rs:578` | `'Segoe UI Emoji','Apple Color Emoji','Noto Color Emoji',sans-serif` |
| 预览字面量兜底 | `PolishPreview.vue:1347` | 又抄一份 SYSTEM_FONT_STACK |
| `ensure_emoji_fallback` | `takumi.rs:196-199` | 又抄一份 emoji 栈（正则注入） |

而**输入口有 7 个、3 种形态**（库 id / 文件绝对路径 / 写死的系统族名），互不校验：

| # | 输入口 | 形态 | 备注 |
|---|---|---|---|
| 1 | `[style.subtitle] font` | 库 id | 字幕 |
| 2 | `[cover] font` | 库 id | 封面标题 |
| 3 | `POST /api/polish/project` `subtitle_font` | 库 id | 写回 #1 |
| 4 | `POST /api/polish/cover-config` `cover.font` | 库 id | 写回 #2 |
| 5 | `VTEDIT_DHAMPIR_FONT` | **文件绝对路径** | **优先于一切**（`dhampir.rs:436`） |
| 6 | `cover.rs::EMOJI_FONT` | 写死族名 | 封面 emoji 特效层 |
| 7 | `takumi.rs::ensure_emoji_fallback` | 写死族名 | 封面 CSS 兜底 |

⇒ 不先收敛，每接一个消费点都要重复回答"这个口用的是哪份栈、emoji 谁来补"，
**而答案在 4 个地方各有一份、还会漂**。这就是"先做基础建设"的理由。

---

## 二、基础建设（B 系列）—— 不含任何出图行为改动

### B1 单一字体解析入口（目标①）

在 `yekitrim-core::fonts` 增加**角色化解析**，成为"用哪支字体"的唯一答案：

```rust
/// 字体解析的唯一入口：给定用户选择，返回角色化的字体组合。
/// 所有消费点（预览 / 成片 / 封面）都必须经它取字体，不许自己拼栈。
pub struct FontPlan {
    /// 正文族名（用户选的那支的内部族名；空/未装 ⇒ 走 preview 的兜底）
    pub text_family: Option<String>,
    /// **emoji 补充槽位**：库里 emoji==true 且已装，id 稳定序取第一支；没有则 None
    pub emoji_family: Option<String>,
    /// 兜底链（空库回落态，逐字保留今天的 SYSTEM_FONT_STACK 语义）
    pub fallback: Vec<String>,
}
pub fn resolve_font_plan(root: &Path, selected_id: Option<&str>) -> FontPlan;
```

**关键约束**：`emoji_family` 是**独立字段**，不是拼进 `fallback` 的一段 —— 由**消费点自己决定**
要不要用、以及插在哪（因为三个消费点的语义不同，见 §五 事实 1/3）。这是职责分离在类型上的落地。

### B2 emoji 槽位的唯一权威

```rust
/// 库里至多一支生效的 emoji 字体（emoji==true 且已装，id 稳定序）。
pub fn emoji_font(root: &Path) -> Option<FontEntry>;
```
- 只取**已装**的（`installed` 或 `resolve_all` 解析得到）—— 没下载的不算，与 `cover_font_files_in` 同口径。
- **建议同时做 A4**：`set_default` 拒绝 emoji + `preferred_fallback_family` 扫描时把 emoji 的 `is_default` 视为 false。
  ⇒ 否则手改 FONT.toml 把 emoji 设成 default，会让 `server.rs:228-232` 的 `find(\|e\| e.is_default)`
  把**整个栈首**变成 emoji 族 ⇒ 全库中文豆腐。**这是当前代码里真实存在的一条路径。**

### B3 四份写死 → 全部改为问 B1/B2

- `server.rs::SYSTEM_FONT_STACK`：**保留为常量**（它是"空库回落态"的文档锚点 + 单测钉子），
  但 `fallback_stack()` 改为从 `FontPlan.fallback` 拼；
- `cover.rs::EMOJI_FONT` → 问 B2，没有已装 emoji 时回落系统族名栈（保持今天的观感）；
- `takumi.rs::ensure_emoji_fallback` → 用 B2 的族名；**同时修 R2 的幂等 bug**（见 §六）；
- `PolishPreview.vue:1347` 的字面量兜底：**保留**（旧 server 兼容），但加注"新 server 永远给非空"。

### B4 可校验性地基（这是"依次校验"的前提）

今天**没有**"字体选择链"的独立判据，只有零散单测。要补两件：

1. **Rust 侧：`FontPlan` 的纯函数单测**（不渲染、毫秒级，可覆盖所有分支）：

   | 用例 | 期望 |
   |---|---|
   | 空库 + 未选 | fallback == 今天的 SYSTEM_FONT_STACK（**逐字**） |
   | 装了中文 + 未选 | text_family == 中文内部族名；fallback 同老常量 |
   | 装了 emoji + 装了中文 | **emoji_family == Some(...)，且 text_family 不受影响** |
   | 只有 emoji、没中文 | text_family 为空（**不能**把 emoji 当正文） |
   | 手改 FONT.toml 给 emoji 设 default | is_default 视为 false，text_family 不为 emoji |

   ⚠️ 第 3、5 条是**这次新增能力的判据**；第 1 条是"零回归"的钉子 —— **它保留 `server.rs:544-562` 的语义**。

2. **一条门禁脚本 `scripts/check-font-inputs.mjs`**（抄 `check-effect-registry.mjs` 的形状）：
   把"字体输入面"登记成一份清单，**两向判据**：
   - **登记的口必须真的存在**（在源码里找到该字段/调用）；
   - **那些文件里不许有没人认领的字体栈字面量**（扫 `sans-serif` 前缀的 `font-family` 字面量，
     每个都要么来自 B1/B2、要么显式登记为"有意保留"）。
   ⇒ 这条**正是防"第 5 份写死又长出来"**的机制 —— 也是"每接一个消费点"时**自动检查它有没有走统一入口**的手段。
   **它让"依次对接"变成可逐条勾选、且漏了会红的事。**

⚠️ B 系列**不改任何出图行为**（除 A4 的拒绝路径），所以这两条单测 + 门禁能先绿，
之后每接一个消费点，跑它就知道有没有接对。

---

## 三、依次对接每个消费点（C 系列，每个独立验收）

按"依赖最少、最容易验"排序。**每步结束后 B4 的门禁必须仍绿**（证明没长出新写死）。

| 步 | 消费点 | 改什么 | 独立判据 |
|---|---|---|---|
| **C1** | 字幕**预览**（`.cap` 栈） | `/api/polish/fonts` 透传 `FontPlan`（含 emoji_family）；`PolishPreview.vue` 拼栈时**把 emoji 追加在末尾** | ① B4 单测；② 前端构建；③ 真跑预览看 emoji（浏览器自带系统 emoji，此项**本就能出**，所以判据是"没退步"） |
| **C2** | 封面**标题** | `cover_font_css` 走 `FontPlan`；`cover_font_files` 注册序改**三段**：默认文字 → 其余文字 → **emoji 收尾** | ① B4 门禁；② `check-cover-takumi.mjs --golden`（**注意：换注册序会动像素，可能需要重冻参照帧**）；③ **真出一张带 emoji 的封面，肉眼确认颜色** |
| **C3** | 封面 **emoji 特效层** | `cover_fx_html` 的 `EMOJI_FONT` → B2；`ensure_emoji_fallback` 动态化 + 幂等 | 同 C2 的 ③（特效层与标题共用颜色问题） |
| **C4** | 前端**职责分离** | C9 两个下拉过滤 `f.emoji`；C10 `FontLibrary` 隐藏 emoji 的「设为默认」 | ① `npm run build`；② `check-cover-stage.mjs`；③ 手工：emoji 不再出现在两个下拉 |
| **C5** | 字幕**成片**（底座） | **不改**。把结论记档 | 出一条带 emoji 的字幕成片，**确认它出不来的原因被准确记录**（见事实 1） |

⚠️ **C5 是"对接并校验"的终点之一，但结论是"本仓不可修"** —— 这条必须显式走一遍并把证据留下，
否则下一个人还会以为"改改栈就好了"。

### 排序理由
- **C1 先做**：预览最容易验、且是浏览器（系统自带 emoji），能先把"数据链路是否通"确认掉。
- **C2/C3 一起验**：两者共用同一个"颜色"问题（事实 2），分开做会**重复踩同一个坑两次**。
- **C4 最后**：它是纯 UI 约束，不依赖前四条，但**必须等 B1/B2 落地**才说得清"emoji 由谁自动补"。
  （否则 UI 写着"自动补充"，而底层还没有那个槽位。）

---

## 四、字体本身的选型（放在 B 与 C 之间，或与 B2 并行）

### S0.5 换 emoji 字体（事实 2 的直接后果）

**自带 emoji 字体是 COLRv1，Takumi 写死只认 COLRv0** ⇒ 即使 C2/C3 接对了，
emoji 也只会是**单色剪影**。所以：

- **首选**：退到 **CBDT 版 Noto Color Emoji v1.x**（Takumi 的位图路径完整支持，且可分发）；
- ⚠️ **必须换新 id**：下载三入口都幂等（`builtin_installed` 为真就跳过），
  **复用同一个 id 老机器永远不会重下** ⇒ 症状是"我改了字体但用户那边没变"；
- 换 id 要**同步改** `BUILTIN_FONTS`（`fonts.rs:699-720`）的 id/file/url/sha256。

⚠️ **这一条应该尽早做**：C2/C3 的验收判据是"**看得见颜色**"，
如果字体还是 COLRv1，C2/C3 做完也**验不出成功**（会误判成"接线没接对"）。

---

## 五、三条硬事实（决定"能验什么"）

### 事实 1：Dhampir 字幕**没有**字体回退 ⇒ C5 不可修 ❌
- `dhampir.rs:445-456` `find_font` 命中即 `return`，只交出**一个**文件；
- CLI 只有 `--font-file` / `--font-bold-file`（`:1003-1034`）；
- 每轨族名由转译器写，**只有一个**（`polish-to-dhampir.mjs:3325` / `:3517`）；
- **底座源码在 `F:\\para\\Code\\Dhampir`**（实读）：`text_overlay.rs:368-387` `pick_font` → `Option<PathBuf>`；
  `text_raster.rs:413-497` 生成 ffmpeg `drawtext=fontfile=<单支>` —— **无回退链、无 fontconfig**；
  `text_raster.rs:58` 明写"不做彩色 emoji 字形"；全仓无多字体选项。
- ⇒ 栈尾插 emoji **零效果**；若成了首个可解析项反而**整轨变豆腐**。

### 事实 2：自带 emoji 字体 **COLRv1** × Takumi **只认 COLRv0** ⇒ 单色剪影 ❌
**实测字节**（`~/.yekitrim/fonts/NotoColorEmoji/font.ttf`，25,332,736 B，16 表）：
```
HAS CBDT: false   HAS CBLC: false   HAS sbix: false
COLR version = 1        ← COLRv1（另带 20 MB SVG 表、3.4 MB glyf）
```
`takumi-core-0.25.0/src/resources/glyph.rs:440-443`：
```rust
let color_glyph = self.color_glyphs.get_with_format(glyph_id, ColorGlyphFormat::ColrV0)?;  // 写死 V0
```
三级解析（`glyph.rs:417-433`）：①位图 strikes 无表→`None` ②**只问 ColrV0**→`None` ③`glyf` 描边→✅**单色剪影**。
（本机 `seguiemj.ttf` 实测 `COLR version = 0` —— 当年实验台正是用它验的，见 §七 陷阱。）

### 事实 3：Takumi fallback 桶 = **注册序全集**，CSS 族名几乎不起作用
- `takumi-raster-0.5.1/src/render.rs:167,431`：`snapshot_with_fallbacks(font_families.as_ref())`；
- 本仓 `takumi.rs:565-572` **没设 `font_families`**（全仓 grep = 0）⇒ `None`；
- `takumi-core/src/resources/font.rs:468-475`：`None` ⇒ 桶 = `self.order` = **全部已注册字体按注册序**。
- ⇒ **"挑哪支"由 `cover_font_files()` 的注册顺序决定** ⇒ C2 的三段注册序是**唯一有效旋钮**。
- ⚠️ `ensure_emoji_fallback` **只扫 `<style>` 块**（`takumi.rs:60-84`），效果层 emoji 是 **inline style**（`cover.rs:642`）
  ⇒ **特效那层永远扫不到** —— C3 必须用 B2 改 inline 那处的常量，别指望正则。

---

## 六、风险（地基阶段必须先解决）

### R1 ⚠️ A2 与"空库逐字不变"冲突 —— **本计划通过"B 系列不动行为"回避**
`server.rs:529-535` 硬钉 `matches(',').count() == 4`；插 emoji 就变 5 ⇒ 红。
⇒ **本计划不在 `fallback_stack()` 里插 emoji**（emoji 走独立字段、由各消费点自己决定）。
这样 R1 **不触发**，零回归承诺保得住。若后续仍想插，必须单独一批 + 更新那两条单测。

### R2 ⚠️ `ensure_emoji_fallback` 正则**不幂等**（实测）
```
.fx-item.fx-emoji{font-family:'Segoe UI Emoji','Apple Color Emoji','Noto Color Emoji',sans-serif;}
  → …'Noto Color Emoji','Segoe UI Emoji','Noto Color Emoji',sans-serif;   ← 多一段
```
⇒ B3 顺手改成**"已在其中就不插"**（有单测）。

### R3 系统 emoji 族名在 Takumi 上是**死代码**
`Segoe UI Emoji` 等不解析（不读系统字体）⇒ B3 若写"系统栈兜底"，对 Takumi **无兜底作用**，别当保险。

### R4 换注册序会动封面像素
C2 改了 `cover_font_files_in` 的顺序 ⇒ `check-cover-takumi.mjs --golden` 可能超阈值
（注册序第一支 ≈ 真正画标题的那支）。
⇒ **按仓库既有规矩：改出片表现就要重冻参照帧 + 重跑三样本**（`.workbuddy` 有先例）。

---

## 七、陷阱（这次差点又踩，留作判据）

`docs/plan/takumi-cover-capability.md:138` 写着实验台用 **`seguiemj.ttf`（彩色 emoji）**，
`:343`/`:393` 据此宣称 emoji "逐像素级"通过。而实测 `seguiemj.ttf` 是 **COLRv0** —— 正好 Takumi 支持的那档。

⇒ **当年验收对自带那支 COLRv1 字体零证明力。** 本机 `Segoe UI Emoji` 排在栈首且系统装了它，
于是"碰巧是对的"。**换台没有 seguiemj 的机器就退化成剪影。**

**判据结论**：C2/C3 的验收必须**看得见颜色**（肉眼/像素），**不能只看"有没有出图"或"有没有形状"**。

---

## 八、执行顺序总览

```
B0  建地基（无行为改动，先绿）
    ├─ B1 resolve_font_plan（唯一入口）
    ├─ B2 emoji_font 槽位 + A4 双向拦截 is_default
    ├─ B4 Rust 纯函数单测（含零回归钉子）+ scripts/check-font-inputs.mjs
    └─ 出口判据：cargo test + 新门禁绿

S0.5 换 emoji 字体（CBDT 版，**换新 id**）—— 尽早做，否则 C2/C3 验不出成功
    └─ 出口判据：真出一张带 emoji 的封面，**肉眼确认是彩色**

C1  字幕预览对接 + 校验          → 出口：预览 emoji 无退步 + 门禁绿
C2  封面标题对接 + 校验（注册序三段）→ 出口：封面 emoji 出彩 + golden 门禁（必要时重冻）
C3  封面 emoji 特效层对接 + 校验   → 出口：特效层出彩
C4  前端职责分离（C9/C10）        → 出口：emoji 不在正文下拉 + build 过
C5  字幕成片：**确认不可修并记档**  → 出口：证据写进 dhampir-parity-register.md

最后  文档：font-library.md 新增 §22（目前只到 §21，实读确认）；
      takumi-cover-capability.md 给 emoji 结论加 COLR 版本限定
```

---

## 九、验证命令

```bash
# 地基
cargo test -p yekitrim-core --lib fonts
node scripts/check-font-inputs.mjs            # 新增（B4）

# 每个消费点接完
cargo test -p yekitrim-polish --lib render       # server.rs 字体单测
cargo test -p yekitrim-polish                    # takumi_font_id.rs（C2 改序后必跑）
cargo test -p yekitrim-server                    # /api/polish/fonts 响应

# 封面真渲染（C2/C3）
node scripts/check-cover-takumi.mjs --golden scripts/fixtures/cover-golden

# 前端（C1/C4）
npm --prefix webui run build
node webui/scripts/check-cover-stage.mjs

# 全量门禁（.workbuddy/gates.mjs 已有 12 项，B4 应加进去）
node .workbuddy/gates.mjs
python3 scripts/check_module_map.py
python3 scripts/check_module_boundaries.py
```

⚠️ 本机 **bash 坏了**（`couldn't create signal pipe, Win32 error 5`），用 `pwsh` 或直接调工具；
`python3` 别写 `python`（`gates.mjs` 里用的是绝对路径 `YEKITRIM_PYTHON`）。

---

## 十一、执行结果（2026-10-02 落地，全部已验证）

> 本节是**执行后**的记录。上面的 §一~§十 是计划（保留原样，便于对照哪些判断被证实/被推翻）。

### 状态：B 全绿 / S0.5 已换字体 / C1~C5 全部完成

| 步 | 内容 | 结果 |
|---|---|---|
| B1 | `fonts::resolve_font_plan` + `FontPlan`（角色化单一入口） | ✅ |
| B2 | `fonts::emoji_font` 槽位；A4 拦截**三处**（`set_default` / `scan_dirs` 出口 / `resolve_font_plan`） | ✅ |
| B3 | 四份写死栈全部改为问权威（`server.rs` / `cover.rs` / `takumi.rs` / API） | ✅ |
| B4 | Rust 单测 7 条（含零回归钉子）+ `scripts/check-font-inputs.mjs`（**7 项自检**，已进 `gates.mjs`） | ✅ |
| S0.5 | 换 **CBDT 版** Noto Color Emoji（`v2020-09-16-unicode13_1`，新 id `NotoColorEmoji-CBDT`） | ✅ |
| C1 | 字幕预览：`emoji_family` 透传 + 追加在 `.cap` 栈尾 | ✅ |
| C2/C3 | 封面标题 + emoji 特效层：**真出图确认彩色** | ✅ |
| C4 | C9 两个下拉滤 emoji + C10 字体库不给「设为默认」，重建 `dist` | ✅ |
| C5 | 字幕成片：确认**本仓不可修**，加钉子单测 + 记档 | ✅ |

### 验证证据

```
cargo check --workspace --all-targets     → 0 warnings
cargo test -p yekitrim-core --lib fonts      → 22 passed（含 6 条新增）
cargo test -p yekitrim-polish --lib render   → 29 passed（含 5 条新增）
cargo test -p yekitrim-server                → 151 passed
node .workbuddy/gates.mjs                 → 全部门禁通过（14 项，含新增的字体输入面 + 自检）
node webui/scripts/check-*.mjs            → 7 条全过
```

### 出图对照（`.workbuddy/emoji-cover-test/`，三组都有产物留证）

| 库里有什么 | 产物 | 结果 |
|---|---|---|
| 只有 CBDT | `s2.jpg` | ✅ **emoji 全彩**（标题 + 特效层都是彩色） |
| 只有旧 COLRv1 | `s1.jpg` | ❌ 与「一个 emoji 字体都没有」**逐字节相同**（sha `f1af84a1`）⇒ 旧那支对 Takumi **等于不存在** |
| 两支都在（**修复后**） | `FIXED-with-both-fonts.jpg` | ✅ 全彩（与「只有 CBDT」一致） |

⇒ 这三组把 §七 那个陷阱**从推断变成了实测**：COLRv1 不是"渲染成剪影"，而是**整片消失**
（因为上游 v2.x 那支的 `glyf` 在 Takumi 的路径里也没被用上）。

### 执行中发现的三个 bug（原计划没预见）

**① `sans-serif` 被加了引号** —— `authority_tail()` 第一版对所有项统一 `quote_css`，拼出 `'sans-serif'`。
CSS 里带引号的通用族是"一个叫这名字的**族名**"，浏览器不认识就丢掉 ⇒ **兜底链最后一道保险失效**。
被新加的对拍单测 `tail_stack_matches_core_authority` 抓住。

**② `ensure_emoji_fallback` 不幂等** —— 即原计划 R2，已修 + 2 条单测。

**③ `emoji_font()` 被旧字体抢占（最隐蔽，且是我自己引入的）** ——
库里同时有新旧两支 emoji 时，纯 id 字典序让**旧那支赢**：

```text
"NotoColorEmoji" < "NotoColorEmoji-CBDT"   ⇒ 选到旧 COLRv1 ⇒ 封面 emoji 整片消失
```

而界面照样显示"已装"、`/api/polish/fonts` 也照样回 `emoji_family` —— **没有任何一处会报错**。
这正是本次换字体（换 id）必然触发的场景。修法：**自带（清单内、验证过格式）优先**，
其余按 id 稳定序；回归钉子 `builtin_emoji_wins_over_a_stale_leftover`。

> **教训**："换 id"这个决定本身就制造了"新旧并存"的窗口，而当时只想到了"老机器不会重下"，
> 没想到"老机器上旧那份还在、会被选中"。**换资源的 id 时，要同时想清楚旧资源怎么失效。**

### 对原计划的修正（哪些判断被证实/推翻）

| 原计划说法 | 执行结论 |
|---|---|
| A2「栈尾插 emoji 一处喂三个消费者」 | **证伪**。只对预览有效；底座无逐字形回退（C5 钉子钉住）、封面靠注册序。**最终没做 A2** ⇒ R1 未触发，零回归承诺保住 |
| A5「字幕页 `font_face_css(&[font, emoji])`」 | **砍掉**。那是浏览器预览页，系统自带 emoji 已覆盖，收益≈0 |
| B8「注册序 emoji 收尾」 | **证实是主菜**。C2/C3 的出彩全靠它 |
| D11「P0 是否换 COLR」 | **证实必须换**（且换的是 CBDT 版；上游 `main` 分支早已迁 COLRv1） |
| 「emoji 渲染成单色剪影」 | **修正**：实测是**整片消失**（更隐蔽 —— 剪影至少看得见） |
| 字体输入口「7 个」 | 证实；且写死栈是 **7 处**（3 Rust + 4 前端），不是 4 处 |
