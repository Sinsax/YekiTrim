# 字体库管理（字体身份核查 + 自带字体 + 管理页 + 统一来源）

> 起因：用户报「字体预览和实际的对不上」，并要求 ① 找几支 **CC0/可商用+可嵌入+可再分发** 的字体
> ② 查清"很久之前测试用的那个字体"是什么、什么许可 ③ 首页加一个**字体库管理页**（增删 + 设为默认，
> **程序自带的默认字体不可删除**）④ 借此统一字体管理。
>
> 状态：**本文件是方案**；截至写它时**只做过代码取证**（下面每条都给了 file:line），
> 代码**未改**、字体**未入库**。§6 是落地顺序，§7 是待拍板项。

---

## 1. 三个问题的直接答案

### 1.1 "很久之前测试用的那个字体" = **LXGW WenKai（霞鹜文楷）**

它**不是**某次临时下载的测试字体，而是全仓的**默认字体栈第一位**：

| 位置 | 内容 |
|---|---|
| `THIRD-PARTY-NOTICES.md:94` | `LXGW WenKai（霞鹜文楷）｜ SIL OFL-1.1 ｜ 封面/预览模板的字体栈**按名称引用**，不内嵌、不分发字体文件` |
| `crates/yekitrim-polish/src/render/server.rs:205` | `SYSTEM_FONT_STACK = "'LXGW WenKai','Microsoft YaHei','Noto Sans SC',sans-serif"` |
| `crates/yekitrim-render-core/src/types.rs:539` | 缺省字幕字体 = `"LXGW WenKai"` |
| `templates/index.html:19,746,772,1944`、`PolishPreview.vue:1303,1367` | 同一串栈（预览与出片必须同序 —— 顺序反过一次，见 `docs/yekitrim-integration.md:233`） |

**许可结论：是 OFL-1.1，不是你问的 CC0，但你要的三条它都满足**（见 §2）。
⚠️ 它**从未被内嵌**进发行包，只是"按名字引用" —— 本机没装就回退系统字体。这正是"预览与出片对不上"的一个来源。

### 1.2 我本机实验里真正落到磁盘的字体：**都不能随包分发**

`<程序目录>/resources/fonts/` 里当时是：

| 文件 | 来源 | 能否随包分发 |
|---|---|---|
| `msyh.ttc` | Windows 系统字体（微软雅黑） | ❌ 不可（微软 EULA，且 `.ttc` 浏览器 `@font-face` 不认） |
| `seguiemj.ttf` | Windows 系统字体（Segoe UI Emoji） | ❌ 不可 |
| `SimHeiTest.ttf` | 我在验证时用的测试字体 | ❌ 不是发行品（无许可声明） |

⇒ **发行包必须换成自带的自有许可字体**（§4）。

### 1.3 `polish.rs:1313` 那支字体：**明确不能当自带字体**

代码注释里写着：来自 `font.leminet.cn`，授权是「**免费商用，但不包括嵌入式用途**」——
嵌入/随包分发恰是我们最需要的，所以它**只能作为"用户自己装"的字体**，不能进自带集合。
（这条注释本身就该保留 —— 它挡住了一次许可事故。）

---

## 2. 许可事实（带链接，可直接引用）

- **SIL OFL-1.1**：允许使用/研究/修改/**再分发**；字体**可以捆绑、嵌入**、随任何软件一起出售；
  条件：① 不得**单独出售字体本身** ② 保留名（Reserved Font Name）不得被衍生品使用 ③ 衍生品不得换许可证
  ④ 版权与许可声明必须随附。→ 原文：[SIL 中文页](https://software.sil.org/zh-CN/oflt/) ｜
  摘要与条款：[SPDX OFL-1.1](https://spdx.org/licenses/OFL-1.1)
- **CC0-1.0**：最宽松档（等同公共领域奉献），也被许可解释文章列为"100% FREE"：
  [FontsArena 许可证解释](https://fontsarena.com/licenses-explained/)；Apache-2.0 / MIT / CC0 同属 permissive：
  [FontToolbox 许可指南](https://www.fonttoolbox.com/guides/licensing-client-projects)
- ⚠️ **现实**：**字体界真正 CC0 的极少**（多数是 OFL-1.1，或 Apache-2.0/MIT 的少数拉丁字体）。
  ⇒ **建议以 OFL-1.1 为主选**（它已经满足你列的三条），CC0 作为加分项、**逐支到官方仓库核对**后再入库。
  ⚠️ 本文**不列**具体"某字体=CC0"的断言 —— 我没在官方仓库逐支核对过，不猜（想让我核，说一声，一轮能出一张核对表）。

---

## 3. 现有能力（已经有一半，别另起一套）

| 能力 | 位置 | 现状 |
|---|---|---|
| 字体库目录 | `yekitrim_core::path::font_dir()` → `<程序目录>/resources/fonts` | ✅ 有 |
| 扫描/解析/名称校验 | `yekitrim_core::fonts::{scan, resolve_file, is_font_file_name, is_safe_id}`（`crates/yekitrim-core/src/fonts.rs`） | ✅ 有（含单测：`LXGWWenKai-Regular.ttf` ✓、`思源宋体.OTF` ✓、`is_safe_id("LXGW WenKai (v1.0)")` ✓） |
| 只读浏览 API | `GET /api/polish/fonts`（`api/polish.rs:1253` + `routes.rs:134`），**以磁盘为准** | ✅ 有 |
| 字体字节 API | `GET /api/polish/font?name=<id>`（`api/polish.rs:1281`；候选链含 工程 `assets/fonts`、程序字体库；`font_content_type` 按扩展名给 MIME，**已含 `ttc`→`font/collection`**） | ✅ 有 |
| 出片注入 | `font_face_css` / `font_family_with_stack`（`render/server.rs:130,208`） | ✅ 有 |
| 设计器预览注册 | `webui/src/polish/fonts.js` + `CoverDesigner.vue:454` + `PolishPreview.vue:2023` | ⚠️ **前端自己拼名字**（漂移来源） |
| Takumi | `render/takumi.rs:85` 删 `@font-face`，注册字节 + **用 id 当族名** | ⚠️ 第三份来源 |
| **管理类写操作**（导入/删除/设默认） | —— | ❌ **全缺** |

---

## 4. 建议的自带字体（OFL-1.1 为主；入库前逐支核对）

| 用途 | 候选 | 声明许可 | 备注 |
|---|---|---|---|
| 中文正文字体 | **Noto Sans SC** / **Noto Serif SC**、**思源黑体/宋体**（Source Han） | OFL-1.1 | 覆盖全、体积大（10–16MB ⇒ 考虑**子集**或**按需下载**，与 ASR 模型 `yekitrim download` 同套路） |
| 中文手写/标题 | **LXGW WenKai（霞鹜文楷）** | OFL-1.1（仓库已声明，`THIRD-PARTY-NOTICES.md:94`） | 就是现在默认栈第一位；自带它 = "预览与出片终于同源" |
| 中文无衬线（更窄） | **Sarasa Gothic（更纱黑体）** | OFL-1.1 | 需核对"保留名"与子集 |
| Emoji | **Noto Color Emoji** | OFL-1.1 | ⚠️ 彩色 emoji 的**嵌入**是另一层问题（CBDT 表），Takumi 单点里能画 ✓；浏览器 `@font-face` 也能 ✓ |
| 拉丁（可选） | MIT/CC0 的少数几支 | 待核 | 因为中文不能没有，**别只带拉丁** |

⚠️ 体积与入库两条红线：① 会进 `cargo xtask package` 的包 ② 每一支都要在 `THIRD-PARTY-NOTICES.md` 登记，
并被 `cargo xtask scan` 断言过（"字体目录里的每一支都在 notices 里"）。

---

## 5. 字体库管理设计（数据 + API + UI + 统一来源）

### 5.1 数据布局（每支一个目录；**目录名 = ASCII slug，id 住 `FONT.toml`**）

> ⚠️ **2026-10-02 修订**：本节原文是「id = 目录名 = 稳定标识」。那条规则**已被 §24 取代** ——
> 目录名/文件名一律 ASCII slug，`id` 变成 `FONT.toml` 里的字段（原因：ffmpeg 打不开非 ASCII 字体路径）。
> 下面这段保留作历史形状；现行契约看 §24。

```
resources/fonts/
  noto-sans-sc/            ← id（改名不动 id；不再"文件名当 id"）
    font.ttf               ← 只允许 ttf/otf/woff2（ttc/otc 导入时拆包或**明确拒收**）
    FONT.toml              ← 唯一权威元数据
    LICENSE.txt            ← 随字体一起的许可原文
  noto-color-emoji/ …
```

`FONT.toml` 字段建议：`display_name / family / style / weight / scripts(如 ["hans"]) / emoji(bool) /
license(spdx) / source_url / sha256 / builtin(bool) / is_default(bool) / added_at`。

- **`builtin = true` 的那支就是"程序自带的默认字体"，不可删除**（UI 置灰 + API 403）。
- 兼容老库：`scan()` 现在按**平铺文件名**扫；迁移时对"没有 `FONT.toml` 的散文件"按
  "文件名去扩展名 = id / display_name" 兜底（老工程 `font = "SimHeiTest"` 这类必须继续能解析）。

### 5.2 API（都加在 `crates/yekitrim-server/src/api/polish.rs` + `routes.rs`）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/polish/fonts` | 已有；**扩展**返回元数据（id/display/family/builtin/is_default/license/emoji/scripts） |
| POST | `/api/polish/fonts/import` | 导入（上传字节或给定本机路径）。**校验**：能解析、能取到 family、不是 `.ttc/.otc`（或自动拆包）；落盘 + 写 `FONT.toml` + 复制许可 |
| POST | `/api/polish/fonts/:id/default` | 设为默认（同时写回老工程的缺省？→ 只影响**新工程**与"缺省"读取，见 5.4） |
| DELETE | `/api/polish/fonts/:id` | 删除；`builtin=true` → **403**，理由写清"程序自带字体不可删除" |
| GET | `/api/polish/font?name=<id>` | 已有（预览取字节） |

⚠️ **路径安全**：`id` 一律过 `yekitrim_core::fonts::is_safe_id` + 只允许**单段**、只允许库目录下；
**别信前端**。这条与根卡「路径面三防线」同源（历史上出过"普通用户能读服务器任意文件"的事故）。

### 5.3 UI：首页新增「字体库」

- 新页 `webui/src/views/FontsView.vue`（+ 路由 + 侧栏入口，首页区）。
- 卡片列表，每张卡片：**现场注册的预览字形**（`永 Aa 123 永`，复用 `webui/src/polish/fonts.js` 的
  `FontFace` 逻辑，**别再写第二份注册代码**）、显示名/家族/许可徽标、`程序自带` 徽标、
  `设为默认` 按钮、`删除` 按钮（自带的那支置灰 + tooltip 写明"程序自带字体不可删除"）。
- 导入：拖拽/选择文件 → 调 `/import` → 失败原因**原样展示**（"这是字体集合 `.ttc`，请先拆包" /
  "解析不到字体家族名"）—— 现在"放进目录就算数"，所以才有"预览对不上"。

### 5.4 统一来源（顺手解决"预览和实际对不上"）

**`FONT.toml` 是唯一权威**，三处派生、禁止各自拼名字：

1. 预览：前端拉 `/api/polish/fonts` 的元数据 → 用 `family` 注册（**不再用 id/文件名猜**）；
2. 出片（Chrome 腿）：`font_face_css` 用同一个 `family`；
3. Takumi：注册同一支字节、族名用同一个 `family`。

另外"设为默认字体"的语义要定死（**待你拍板**）：只影响 ① 字体库 UI 的推荐位 ② **新工程**的
`[cover] font` / `style.subtitle.font` 默认值？**老工程不动**（避免静默改变已出片的工程）——
建议这样，且把"默认"存成**程序级配置**（`<程序目录>/.yekitrim/config.toml` 或字体库 `FONT.toml` 的 `is_default`）。

---

## 6. 落地顺序（建议）

1. **决定自带哪支、怎么带**（§4 + §7）；下载 → 放 `resources/fonts/<slug>/` + `FONT.toml` + `LICENSE.txt`；
   同步 `THIRD-PARTY-NOTICES.md`。
2. `yekitrim_core::fonts` 加元数据层（读/写 `FONT.toml`、`builtin`/`is_default`、导入校验、`ttc` 处理），
   `scan()` 兼容"平铺老文件"。
3. 后端四个端点 + 路径安全（照 `api/AGENTS.md` 的"同步 IO 走 `api::blocking`"纪律）。
4. 前端 `FontsView.vue` + 路由/侧栏 + 复用 `fonts.js`；跑 `npm --prefix webui run build`（**dist 入库**）。
5. **把三处字体来源收敛到元数据**（预览/出片/Takumi），此时"预览对不上"才算真修好。
6. 验收：① 自带字体在首页可见、预览字形与出片一致 ② 删除自带 → 403 ③ 设默认 → 新工程走它
   ④ `.ttc` 导入被明确拒绝并说明 ⑤ `python3 scripts/check_module_map.py` + `check_module_boundaries.py` +
   `cargo check --workspace --all-targets`（0 warnings）+ `cargo xtask scan`。

## 7. 待你拍板 / 未做

- 自带字体**具体哪几支**（中文 1–2 支 + emoji 1 支？拉丁要不要）与**体积策略**（入仓 vs 首启下载 vs 子集化）。
- 要不要我**逐支去官方仓库核对**许可（把"CC0 的那几支"变成一张有链接的核对表）——你说一声就做。
- "设为默认"的作用域（新工程 only ？还是也改老工程）—— 建议新工程 only。
- `.ttc` 策略：自动拆包（推荐，Windows 用户库里很多 `.ttc`）还是明确拒收。
- ⚠️ 本文件写就时**代码未改、字体未入库**；`docs/plan/takumi-cover-handover.md` §3 里"Linux 侧换 Noto CJK +
  Noto Color Emoji"那条与本文件 §4 是同一件事，落地时两处一起改。

---

## 8. 命名谱系与许可核对（**已核**，2026-10-01 查上游仓库）

用户反馈"LXGW WenKai 好像还有一个名字、搜到的不是这个" —— 核实结果：**都对，是同一个字体的不同层**。

### 8.1 三层名字

| 层 | 名字 | 说明 |
|---|---|---|
| 上游 | **Klee One（クレー One）** | **FONTWORKS** 2020-12 开源的日文字体（7 款开源日文字体里字符数最多的一支，兼具仿宋与楷体味）—— 霞鹜文楷是**它的衍生**。[来源](https://github.com/lxgw/LxgwWenkaiGB/blob/main/README.md) |
| 本项目名 | **霞鹜文楷 / LXGW WenKai** | 作者 lxgw（"霞鹜" = lxgw 的号）。**"LXGW WenKai" 是文件里的族名，也是英文项目名**，两者同指一物。[仓库](https://github.com/lxgw/LxgwWenKai/) |
| 变体们（各有独立族名） | **Screen / Lite / GB / TC / Mono** | 见 §8.2 —— 用户搜到的"另一个名字"很可能就是其中之一 |

⇒ 本仓 `THIRD-PARTY-NOTICES.md:94` 写的 `LXGW WenKai（霞鹜文楷）` **没写错**；只是它同时是
"Klee One 的衍生"，且官方分发版有好几个**同名不同族**的变体。

### 8.2 官方变体（各自独立，别混用族名）

- **霞鹜文楷 / LXGW WenKai**（完整版）
- **霞鹜文楷屏幕阅读版 / LXGW WenKai Screen**（部分 Android 免 ROOT 换字体用）
- ⭐ **霞鹜文楷轻便版 / LXGW WenKai Lite** —— 官方 README 原话："与完整版相比剔除一些较不常用的字符，
  **便于开发者将字体嵌入软件中**" ⇒ **这才是我们该自带的那一支**（体积小、且作者明确面向嵌入场景）
- **霞鹜文楷 GB / LXGW WenKai GB**（简化字国标字形，独立仓库）
- 另有 TC（繁体）、Mono（等宽）等

### 8.3 许可（**比标准 OFL 更宽松**，直接解决 §7 两个待定项）

`OFL.txt` 原文关键两行（[链接](https://github.com/lxgw/LxgwWenKai/blob/main/OFL.txt)）：

```
Copyright 2021-2026 LXGW (https://github.com/lxgw/LxgwWenKai),
  with Reserved Font Name '霞鹜', '霞鶩', '落霞孤鹜', '落霞孤鶩' and 'LXGW'.
[ADDITIONAL PERMISSION] The Reserved Font Names '霞鹜', '霞鶩', '落霞孤鹜', '落霞孤鶩' and 'LXGW'
  may continue to be used in Modified Versions recompiled from the Original Version, without …
```

⇒ 结论（写进计划，别再"待核对"）：

1. **嵌入 / 捆绑 / 随软件再分发：允许**（OFL-1.1 明确允许，见 §2）✓
2. **子集化（= Modified Version）：允许；且作者额外允许"从原始版本重编译的修改版"继续用保留名**
   —— 比标准 OFL"修改版不得用保留名"更宽松 ⇒ **§7 里"子集化能不能做"：能做** ✓
3. 仍然**禁止单独出售字体本身**；**必须随附该字体发行包里的 `OFL.txt` 原文**（不是自己写的摘要）
   ⇒ 这条要进 `resources/fonts/<slug>/LICENSE.txt` 与 `THIRD-PARTY-NOTICES.md`
4. 因为是 Klee One 的衍生，**版权声明按上游发行包原样保留**（别删 Klee One / FONTWORKS 那一行）
5. ⚠️ 族名别写错：CSS 栈里现在是 `'LXGW WenKai'`（**完整版**族名）；若改用 **Lite**，族名变成
   `LXGW WenKai Lite` ⇒ 换字体时**必须同步** `SYSTEM_FONT_STACK`（`render/server.rs:205`）、
   `templates/index.html`、`PolishPreview.vue` 三处栈，否则又是"预览与出片对不上"。

**对 §4 的修订**：中文手写/标题那行由"LXGW WenKai（完整版）"改为
**"优先 `LXGW WenKai Lite`（作者明确面向嵌入），完整版仅作可选下载"**。

**对 §7 的修订**：清单里 **LXGW WenKai 的许可核对已完成**（本节）；其余候选
（Noto Sans SC / Source Han / Sarasa Gothic / Noto Color Emoji）仍待逐支核对。

---

## 9. 第三方"改名转载"与"内部名不一致"（用户报：乐米波波体下载后显示 lxgw）

### 9.1 乐米波波体是什么（**已查，与 LXGW WenKai 不是同一支**）

| | 乐米波波体 / LemiBoBoTi | 霞鹜文楷 / LXGW WenKai |
|---|---|---|
| 出品 | **乐米字库**（`font.leminet.cn`） | lxgw（衍生自 FONTWORKS 的 **Klee One**） |
| 风格 | **圆润饱满、萌趣的软萌标题体**（卡通/创意） | **楷体**（文艺、诗词/注释向正文） |
| 字符集 | **5813 字（GB2312-80 常用集）**、约 **4.20MB** | 数万字（完整版十数 MB；Lite 剔除生僻字） |
| 授权 | **免费商用**（官网写 "Free to use for commercial purposes"）；**未见"允许嵌入/再分发"的表述** | **OFL-1.1**：明确允许捆绑、嵌入、再分发，另带保留名条款（§8.3） |

来源：[乐米官网](https://font.leminet.cn/) · [100font](https://www.100font.com/thread-1752.htm) ·
[猫啃网](https://www.maoken.com/freefonts/27515.html) · [字库星球](https://www.zikuxq.com/28994.html) ·
[zfont](https://zfont.cn/cn/font_751.html)

⇒ **两者设计风格与字符数都不同，我在公开来源里也没有找到"乐米波波体是 LXGW 改名版"的证据**，
所以**不下"改名转载"这个结论**（上一轮我推测错了，此处更正）。

### 9.2 那"下载下来名称显示 lxgw"最可能的三种解释（按可能性排序）

1. **我们的默认栈第一位本来就是 `LXGW WenKai`**（`render/server.rs:205`、`render-core/types.rs:539`）——
   测试那段时间工程里用的、以及字体库里放着的，**很可能就是 LXGW WenKai**；乐米波波体是另一支下载的
   ⇒ 记忆叠在一起。
2. **下载页/压缩包是"合集"**：第三方字体站（以及官网的打包下载）常把多支字体放一个包里，
   文件名沿用了上游库名 ⇒ 解压出来看到 `LXGW…ttf`。
3. **确实存在"改名转载"的第三方版本**（把 OFL 字体改名再发，且**内部 name 表没改干净**）——
   这是业界常见现象，但**需要看你手上那个文件才能判定**，别靠猜。

### 9.3 当场判定的方法（**别再靠文件名**）

1. **仓库里现成有诊断用例**：`crates/yekitrim-core/src/fonts.rs:367` 的 `diagnose_font_family` ——
   ```bash
   cargo test -p yekitrim-core --lib diagnose_font_family -- --ignored --nocapture
   ```
2. Windows 图形界面：右键 `.ttf` → **属性 → 详细信息** → "标题/字体名称" 就是**内部族名**；
3. 长期解药 = 字体库页（§5.3）**同时显示"内部族名"与"我们给的显示名"**；导入时若两者不一致，
   **明确提示**（"文件内部族名是 X，你给它起的名字是 Y"）——这次这个坑正是该页存在的理由。

### 9.4 对决策的影响

- **乐米波波体不进自带集合**（授权只覆盖"免费商用"，没有嵌入/再分发许可；我们发行包必须能嵌入）——
  与 `api/polish.rs:1313` 那条注释一致，**它只能作"用户自己装"的字体**。
- 自带字体仍走**上游 OFL 的 `LXGW WenKai Lite`**（§8.2/§8.3：允许嵌入 + 允许子集化 + 保留名规则清晰）。
- ⚠️ **导入校验里加一条**：把"内部族名"与"显示名"分开存（`FONT.toml` 的 `family` / `display_name`），
  **运行时三处字体来源只认 `family`**（§5.4）——这样"文件名/内部名/显示名"三套名字再打架也不会影响出片。

---

## 11. P1-c 落地记录（服务端四端点，2026-10-01）

### 11.1 做了什么

| 端点 | 层 | 行为 |
|---|---|---|
| `GET /api/polish/fonts` | 用户层（原样） | 改走 `scan_all`，每支回 `id / file / **family（内部族名）** / display_name / bytes / builtin / is_default / license / scripts / emoji` |
| `POST /api/polish/fonts/import` | **admin 层** | body `{path, name?}`；校验全在 `fonts::import_font`（只收 ttf/otf、读得出族名、**拒收 .ttc/.otc**）；落盘 → `<slug>/font.ttf` + `FONT.toml` |
| `POST /api/polish/fonts/default` | **admin 层** | `{id}`；**唯一默认**（先清其它支的 `is_default`） |
| `DELETE /api/polish/fonts?id=` | **admin 层** | `builtin=true` → **403「程序自带字体不可删除」**；其余错误 400 |

### 11.2 三条设计决定（**都是有意的，别"顺手改正"**）

1. **写操作挂 admin 层，不挂 polish 层**：字体库是**程序级全局资产**（`<exe 同级>/resources/fonts/`），
   与贴纸库同规矩 —— 读谁都能读，写只给 admin。因此这三条**不经过** `scope::enforce_scope`
   （那是"项目内相对路径"的收口，而字体库根本不在任何项目里）。安全姿态与
   `projects.rs`「仅管理员可指定服务器本机文件」完全一致：非 admin 连 `/api/fs/list` 都打不开，
   所以"导入一个服务器上的路径"这条能力对普通账号不存在。
2. **导入用路径而不是上传**：字体动辄几十 MB，axum 默认 body limit 2 MB，放开它要给这一条单开
   limit（贴纸库 64 MiB 的先例）。⚠️ **上传留后续**，别以为现在就能拖拽。
3. **`id` 走查询串而不是路径段**（`DELETE /polish/fonts?id=`）：与既有 `/polish/font?name=` 同形，
   也避免 `axum::extract::Path` 与 `std::path::Path` 在本模块撞名（`polish.rs` 顶上 import 的是后者）。

### 11.3 复跑（全绿）

```bash
cargo check -p yekitrim-server --all-targets   # exit 0
cargo test  -p yekitrim-server                 # 144 passed / 0 failed
python3 scripts/check_module_map.py         # exit 0（35 模块 / 7 项）
python3 scripts/check_module_boundaries.py  # exit 0（15 规则 / 243 文件 / 0 违规）
python3 scripts/route_manifest.py           # exit 0
```

### 11.4 顺带修的一处**回退风险**

`scan_all` 里老布局（平铺文件）的 `display_name` 原计划回落成 `id`，那会让前端从"看到族名"
变成"看到文件名"——**已改成回落成解析出的族名**，与改动前前端看到的完全一致（不回归）。


---

## 10. 落地契约（P1-a 结论：读完 `crates/yekitrim-core/AGENTS.md` + `src/fonts.rs` 后定）

### 10.1 现状（要保住的部分）

- `FONT_EXTS = ["ttf","otf","ttc"]` —— **有意只收三种**，因为 woff/woff2 只有浏览器认（"预览里好看、出片是另一个样"）。
- `scan()/scan_at(root)`（**可注入根目录 ⇒ 测试友好**）、`resolve_file()/resolve_file_at(root,id)`、
  `family_of()`（手写最小 `name` 表解析，**按 `(path,mtime,size)` 缓存**，别每次重读几十 MB）、
  `is_safe_id()`（只拦路径穿越/控制字符，空格与汉字放行）、`dir()/ensure_dir()`。
- 语义：**配置里存 id**；`family` 只是展示名；**空串 = 系统字体栈**；老工程就是这一档。
- 回归钉子：`fonts::tests`（扫描 / 解析 / 越权）。

### 10.2 要改的四处（**其中两处与现有规矩冲突，必须同批改文档**）

| # | 改动 | 连带的文档改动（**别漏**） |
|---|---|---|
| ① | **id：文件名去扩展名 → 目录名**（`resources/fonts/<slug>/`） | `crates/yekitrim-core/AGENTS.md` 第 67 行"**id = 文件名去扩展名**"必须改；`resolve_file*` 要**同时支持两种布局**（老平铺文件继续可解析） |
| ② | `FontEntry` 扩元数据；**`family` 语义变严** = **字体内部族名**（唯一权威），另立 `display_name` 给人看 | 卡里"`family` 只是展示名"这句要改；§5.4 的三处来源只认 `family` |
| ③ | **自带字体**（LXGW WenKai Lite + Noto Color Emoji） | ⚠️ **与卡第 72 行「发行包不带任何字体（授权原因）」+ README「字体」一节直接冲突** ⇒ 同批改这两处，并写明依据：OFL-1.1 **允许嵌入/再分发**（§2/§8.3）+ 随附**发行包自带的 `OFL.txt` 原文** + `THIRD-PARTY-NOTICES.md` 登记 + **`cargo xtask scan` 断言"字体目录里每一支都在 notices 里"**。⚠️ **"Windows 系统字体不可分发"那条仍然成立** —— 那正是过去"不带任何字体"的原因 |
| ④ | `.ttc` 策略 | 卡里只论证了"Chrome 与 swash 读得动"，**漏了"浏览器 `@font-face` 不认字体集合"** —— 而"预览与实际对不上"很可能就是它。**第一版：导入时拒收 `.ttc/.otc` 并明确提示**（拆包留后续）；`FONT_EXTS` 是否收窄到 `ttf/otf` 要与此一致 |

### 10.3 分层纪律（别越界）

- **core 只做文件层**（读/写 `FONT.toml`、扫描、解析、导入落盘、删除、设默认），**不碰 HTTP**；
  四个端点全在 `yekitrim-server`（`api/polish.rs` + `routes.rs`），同步 IO 走 `api::blocking`。
- `yekitrim-core` **不得出现** `yekitrim_polish`/`yekitrim_service`/`yekitrim_server`/`yekitrim_cli`（门禁会红）。
- 老行为必须保住：老工程 `font = "LXGW WenKai"`（系统族名）**解析不到就不注入 `@font-face`**（有意为之）；
  `font = "SimHeiTest"`（库内 id）继续解析得到文件。

### 10.4 P1-b 的第一步（下一个动作）

在 `fonts.rs` 加：`FontMeta`（serde + `FONT.toml` 读写）+ `scan` 兼容两种布局 + `import/remove/set_default`
+ `.ttc` 拒收判据 + `fonts::tests` 补用例（老平铺文件 / 新目录 / builtin 不可删 / family 与 display_name 分离）。

---

## 12. P1-d 落地记录（首页「字体库」页）

### 12.1 落点

| 文件 | 改动 |
|---|---|
| `webui/src/views/FontsView.vue` | **新页面**：卡片墙 + 导入 + 设为默认 + 删除 |
| `webui/src/router.js` | `#/fonts`（`meta:{admin:true, title:'字体库'}`） |
| `webui/src/views/WorkbenchView.vue` | 首页 admin 区块（与 `StickerPacks v-if="isAdmin"` 同款位置）挂入口 |
| `webui/src/api/polish.js` | 新 `importFont` / `setDefaultFont` / `deleteFont` |
| `crates/yekitrim-server/src/api/polish.rs` | 导入**同时接受 `{dir,name}`**（服务端拼路径） |

### 12.2 三条要点

1. **卡片把"内部族名"与"显示名"分行显示** —— "预览与实际对不上"的现场就是这两个名字不一致
   （`family` 自 v2 起是**字体内部族名**，是预览 / 出片 @font-face / Takumi 三处唯一认的名字；
   `display_name` 只是给人看的标签）。
2. **预览字形用现场注册的同一支字节**（`new FontFace(family, /api/polish/font?name=<id>)`）；
   ⚠️ 浏览器认不出的格式（老 `.ttc` 就是这条）**在卡片上打「预览不可用」**，不许静默回退到系统字体
   假装换好了 —— 这条就是"预览与实际对不上"的可见化。
3. **导入走 `ServerDirPicker mode="file" :exts="['ttf','otf']"`**（与「表情包 → 添加」「选择素材」同一份），
   回传 `{dir, name}`、**路径由服务端拼**；前端**不许**拼 dir + 文件名：
   `/api/fs/list` 回传的目录可能带 `\\?\` verbatim 前缀，拼出来是 Windows ERROR_INVALID_NAME 123
   （与 `/api/project-data/assets` 同一条规矩）。`exts` 白名单顺手把非字体文件置灰。

### 12.3 复跑（全绿）

```bash
cargo check -p yekitrim-server --all-targets   # exit 0
cargo test  -p yekitrim-server                 # 144 passed / 0 failed
python3 scripts/check_module_map.py         # exit 0（35 模块 / 7 项）
python3 scripts/check_module_boundaries.py  # exit 0（15 规则 / 244 文件 / 0 违规）
node node_modules/vite/bin/vite.js build    # webui/dist 重建（含 FontsView-*.js/.css）
node webui/scripts/check-*.mjs              # 7 个契约脚本全 exit 0
```

### 12.4 待办（**不是做完了**）

- 导入只支持"服务器上已有文件"（admin）；**上传（拖拽/选本地文件）没做** —— 要给
  `/polish/fonts/import` 单开 body limit（贴纸库 64 MiB 的先例），见 §11.2。
- 没有"改显示名 / 改许可"的编辑入口（元数据能存，界面上只读）。
- **自带字体还没入仓**（P1-f）。

> ⚠️ **views 卡贴着预算**（24385 / 24576 B，最坏指令链 root+webui+webui-src+webui-views 也只剩百来字节）：
> 本页的细节**放这里**，卡里只留一行指针。再加内容前先跑 `check_module_map.py`。

---

## 13. P1-e 落地记录（三处字体来源收敛，2026-10-01）

### 13.1 问题（根因，不是猜的）

同一个「选中字体」在历史上被**三处各自解释**过，其中一处口径不同、且**全部静默**：

| 来源 | 用什么当 CSS 族名 | 位置 |
|---|---|---|
| WebUI 预览 | **配置里的 id**（`new FontFace(id, url)`） | `webui/src/polish/fonts.js` |
| 出片 / 封面 @font-face | **配置里的 id**（`font-family:"<id>"`） | `render/server.rs::font_face_css_in` |
| Takumi 静态封面 | **字体文件内嵌的族名**（`Fonts::register` 读 `name` 表） | `render/takumi.rs` |

前两处一致，**第三处对不上** ⇒ 塔库米那条腿在 id ≠ 内嵌族名时（很常见：id 是文件名
`SimHeiTest`、内嵌族名是 `SimHei Test Internal`）**静默回退**到内置 Geist，中文标题变豆腐块、
**不报错**。这就是"预览与实际对不上"的第三个来源（前两个见 §1）。

### 13.2 收敛方案：**保持"声明名 = id"不变，让 Takumi 按 id 注册别名**

关键取舍：**不能**把 `@font-face` 的声明名改成内部族名 —— 模板 `.cap{font-family:"<id>"}` 与
预览的 `FontFace(id)` 都是按 **id** 请求的，改了声明名反而制造第二条裂缝。
正确做法是让**唯一不一致的那个消费方（Takumi）来适配**：

- `FontResource::override_info(FontOverride { family_name: Some(id), .. })`
  —— Takumi 官方 API，注释原文 *"Family name to register the font under, instead of its embedded name."*
- 每支字体注册**两次**：① 内嵌族名（原生行为）② 库 id 别名。
  字节走 `FontSource::from_shared(Arc<Vec<u8>>)` **共享**，只克隆指针、不翻倍内存。
- 别名怎么算（`font_aliases`）：约定文件名 `font.<ext>` ⇒ **读同目录 `FONT.toml` 的 `id`**（§24 起；
  没有元数据 / `id` 为空才回落目录名）；否则 ⇒ id = **文件名去扩展名**（老平铺）。

同时修掉这条路上**两个真 bug**（都是"v2 布局下静默失效"）：

1. `cover_font_files()` 只列**顶层文件** ⇒ v2 目录布局**一个字体都扫不到**，
   而"零字体"这条路上 `render_cover_png` 是**明确报错**的 → 症状是"静态封面直接出不来"。
   现改走 `fonts::scan_all` + `resolve_all` 反查（`FontEntry` 是元数据、不带路径）。
2. `font_face_css_in` 走的是 `resolve_file_at`（**只认老平铺**）⇒ v2 布局下**一条 @font-face 都不注入**
   →"选了库里的字体、出片没换"，而预览另有 `FontFace` 看起来还是对的，只有成片不对。
   现改走 `resolve_all`。

### 13.3 新增能力（给后续阶段用）

`yekitrim_core::fonts::family_for(id)` / `family_for_at(root, id)`：**id → 该用的 CSS 族名**。
库里认这个 id ⇒ 返回 `FONT.toml` 的 `family`（内部族名，唯一权威）；**库外原样返回**
（老工程 `font = "LXGW WenKai"` 的系统族名行为一字不变）。三处消费方要"按族名做事"时都该走它。

### 13.4 验收（可复跑）

```bash
cargo test -p yekitrim-core --lib fonts                                   # 12 passed
cargo test -p yekitrim-polish --lib                                       # 213 passed
cargo test -p yekitrim-polish --features cover-takumi                     # 含 takumi 单测 + e2e
cargo check --workspace --all-targets                                  # 0 warnings
cargo check -p yekitrim-polish --features cover-takumi --all-targets      # 0 warnings
python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py
```

**E2E 钉子** `crates/yekitrim-polish/tests/takumi_font_id.rs`（真渲染、真 Takumi）：
同一份 HTML 三次渲染只换请求的族名，**逐像素比较**：

```
字体 id=SimHeiTest 内嵌族名="SimHei Test Internal"
  ⇒ id vs 缺失 = 2879 像素不同（别名接上了）
    内嵌 vs 缺失 = 0        （这支字体没有拉丁字形，回退被掩盖 → miss 是有效对照）
```

**变异自证**：把 `for alias in font_aliases(path)` 换成空迭代 ⇒ 测试**失败**并报
"请求库 id 与请求不存在的族名渲染结果完全相同 ⇒ 别名注册没生效"。所以它是真钉子，不是恒真断言。

⚠️ 写这条测试踩过两个度量陷阱（都记在测试文件头注释里，别再犯）：
① 不能用"暗像素数"当判据（回退会把差异掩盖，实测 1713 == 1713）；
② CSS 必须写在**元素内联 style** 上（Takumi 的 HTML 前端只认内联，用外部选择器那张图是全黑的）。

### 13.5 还没做

- **自带字体仍未入仓**（P1-f）：`resources/fonts/` 现在还是"空目录 + 放这里.txt"。
- `family_for` 目前只有测试与后续阶段在用；真正"让预览也按内部族名请求"没做，**也不该做**
  （见 13.2 的取舍）。

---

## 14. 修正记录（P1-e 收尾时发现的两处"说过但没落地"）

写 P1-e 的过程中编译报错，顺藤查出 P1-d 阶段**两处只在文档里存在、代码里没有**的东西。
两条都是"会静默出问题"的类型，记在这里免得再被当成已完成。

### 14.1 三条字体**写**路由的 handler 从未存在过

`routes.rs` 里挂了 `POST /polish/fonts/import`、`POST /polish/fonts/default`、
`DELETE /polish/fonts`，但 `api/polish.rs` 里**没有**对应的
`polish_fonts_import` / `polish_fonts_default` / `polish_fonts_delete` ——
`cargo check` 直接 `E0425: cannot find value … in this scope`。

⚠️ **教训**：`routes.rs` 引用了不存在的 handler 时，**编译期就会炸**；所以"文档写已落地"
必须配一次 `cargo check --workspace --all-targets`。光看 diff / 看文件里 grep 到路由字样，
会得出"已经做完了"的错误结论。

现在三个 handler 都补齐了（`api/polish.rs`），各自的规矩：

| 端点 | body | 错误码 |
|---|---|---|
| `POST /polish/fonts/import` | `{path}` 或 **`{dir,name}`**（服务端拼，避免 verbatim 前缀被拼坏） | 缺参 = 400；`import_font` 的 anyhow 原话 = 400（"只收 .ttf/.otf"等） |
| `POST /polish/fonts/default` | `{id}` | 缺 `id` = 400 |
| `DELETE /polish/fonts?id=` | — | 缺 `id` = 400；**`builtin` = 403**（文案含"程序自带"时映射 FORBIDDEN） |

三条**有意不挂 `enforce_scope`**（字体库是程序级全局资产、不在任何项目内），
但**必须在 admin 层下** —— 导入吃的是"服务器上的路径"，普通用户能调就是任意文件读取。
钉子：`api::routes::tests::font_write_routes_are_admin_only`（非 admin 三条全 403、
缺参全 400、导入不存在的路径 400）。

### 14.2 `GET /polish/fonts` 回的 `family` 是"从文件读的"而不是权威值

P1-d 的文档说 `family` 是内部族名，但实现回的是 `f.family`（`family_of` 从字体 `name` 表读的，
可能与 `FONT.toml` 不一致）。现在改走 `yekitrim_core::fonts::family_for_at(&root, &f.id)`，
并补齐 v2 字段（`display_name` / `builtin` / `is_default` / `license` / `scripts` / `emoji`）。

顺带把 `scan()` 换成 `scan_all(&root)`：前者不认 v2 目录布局。

### 14.3 复查口径

```bash
cargo check --workspace --all-targets                              # 0 warnings（这次就是它抓到的）
cargo check -p yekitrim-polish --features cover-takumi --all-targets  # 0 warnings
cargo test -p yekitrim-server                                         # 145 passed
cargo test -p yekitrim-polish --lib                                   # 213 passed
cargo test -p yekitrim-core --lib fonts                               # 12 passed
cargo test -p yekitrim-service                                        # 125 passed
python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py
```

---

## 15. P1-f 方案：自带字体走「首次使用下载」（用户 2026-10-01 拍板）

### 15.1 决策与理由

实测两支字体的真实体积：
**霞鹜文楷 Lite Regular 13.5 MB**（`lxgw/LxgwWenKai-Lite` v1.522）+
**Noto Color Emoji 24.7 MB**（`google/fonts` `ofl/notocoloremoji`）≈ **38 MB**。

用户先选"进仓库"，在**看到真实体积后**改为 **"首次运行下载"**：仓库只放清单 + 校验和，
字体落 `<程序目录>/resources/fonts/<slug>/`。这样仓库干净、且**离线也不影响已有功能**
（静态封面没字体时本来就是"明确报错 + 告诉用户放字体"，不是静默出豆腐块）。

### 15.2 复用既有两类基础设施（不新造轮子）

- **注册表 + marker 的"装好没有"判定**：抄 `yekitrim-core/src/download.rs` 的 `MODEL_REGISTRY`
  形状（`(键, 目录名, 文件名, marker)`）与 `installed` 语义 —— **半下载的目录不算装好**。
- **下载本身**：同文件的 `download_file(url, part_path)`（先落 `.part` 再改名）与
  进度/失败口径。

### 15.3 落点

| 要做的 | 落点 |
|---|---|
| 字体注册表（上游 URL + 校验和 + 许可 + 显示名） | `yekitrim-core/src/fonts.rs`（与 `scan_all` 同一模块，字体的事只在这里） |
| `ensure_builtin_fonts()` / `download_builtin_fonts()` | 同上；幂等，已装好就跳过 |
| 下载进度与日志 | 走调用方（CLI 打印 / 任务日志），本模块只回结构化结果 |
| CLI 入口 | `yekitrim fonts list` / `yekitrim fonts download`（`crates/yekitrim-cli`） |
| 界面入口 | 字体库页（`#/fonts`）加一条"下载自带字体"按钮 + 状态；API `POST /api/polish/fonts/builtin`（**仅 admin**） |
| 许可 | `LICENSE.txt` 随字体一起落盘（原样 OFL）；`THIRD-PARTY-NOTICES.md` 登记 |
| 发行包说明 | `resources/fonts/放这里.txt` 与 README「字体」改为"自带字体首次使用时下载" |

### 15.4 与「自带不可删除」的关系

`builtin = true` 那条分支（`remove_font` 拒绝删除）**现在才有真实实例**：
下载来的字体写 `builtin = true`，删它由界面置灰 + 服务端 403 两处共同挡住。
⚠️ 但**下载 ≠ 已装**：没下过的机器上 `scan_all` 里没有它，所以
"自带字体"在**安装前后**都必须能被界面区分成"未安装（可下载）"与"已安装（不可删）"。

### 15.5 验收

- `cargo test -p yekitrim-core --lib fonts`：注册表形状、marker 判定、幂等（已装好的目录不重复下载）。
- 真下载一次（**联网**）验证 URL 可用 + 落盘布局 + `family` 能读出来；校验和必须对得上。
- 两条门禁 + `cargo check --workspace --all-targets`（0 warnings）。
- ⚠️ **不把字体二进制提交进 git**（这是本条决策的重点），只提交清单。


---

## 16. P1-f 落地记录（自带字体「首次使用下载」，2026-10-01）

§15 是方案，这里是**做完了的账**：做了什么、踩到什么、怎么验的。

### 16.1 清单（唯一真源：`yekitrim_core::fonts::BUILTIN_FONTS`）

| id（= 目录名） | 显示名 | 内部族名（实测读出） | 体积 | 许可 |
|---|---|---|---|---|
| `LXGWWenKaiLite-Regular` | 霞鹜文楷 Lite | **`霞鹜文楷 轻便版`** | 13,872,424 B | OFL-1.1 |
| `NotoColorEmoji` | Noto Color Emoji | `Noto Color Emoji` | 25,332,736 B | OFL-1.1 |

校验和（sha256，**实测下载后复核一致**）：
- 霞鹜文楷 Lite：`140c99ba4e28e817cec49bf82a0c5fcdc4fe633fb9dfda16d0ee8d59a8545f15`
- Noto Color Emoji：`4d82a18d8d95f60ba883ce242bbadbf84a576e987745dd7ba38d71c67cff2d73`

⚠️ **内部族名是中文**这件事是本条最大的坑，见 §16.4。

### 16.2 落盘形状（与 §10 的 v2 契约一致）

```
<程序目录>/resources/fonts/
  LXGWWenKaiLite-Regular/
    font.ttf        ← 下载后改名（原名 LXGWWenKaiLite-Regular.ttf）
    FONT.toml       ← family/display_name/license/source_url/builtin/sha256/added_at
    LICENSE.txt     ← 上游 OFL.txt 原文（best-effort，失败只 warn）
  NotoColorEmoji/{font.ttf,FONT.toml,LICENSE.txt}
```

⚠️ **`font.ttf` 是固定名**（v2 契约里 `resolve_dir_file` 优先认它），**不是**上游原名 ——
所以 [id = 目录名] 这件事在下载路径上必须显式造出来。

### 16.3 三个入口（都幂等：已装好的跳过）

| 面 | 形态 | 权限 |
|---|---|---|
| CLI | `yekitrim fonts list` / `yekitrim fonts download [--id X]` | 本机 |
| HTTP | `POST /api/polish/fonts/builtin`（body `{}` 或 `{id}`） | **仅 admin** |
| 界面 | 首页「字体库」→ 顶部「下载自带字体」条 + 每张未下载卡上的「下载」 | 仅 admin |

⚠️ HTTP 那条**必须仅 admin**（已加进 `font_write_routes_are_admin_only`）：它会联网并把几十 MB
写进程序目录，普通用户能触发就是"任意用户打外部网络 + 往服务器磁盘写大文件"。

### 16.4 ⚠️ 最大的坑：自带字体的内部族名是**中文**，而字体栈里写的是英文

`server.rs::SYSTEM_FONT_STACK` 首项写死 `'LXGW WenKai'`，而自带那支的内部族名是
**`霞鹜文楷 轻便版`** —— 两者**永远匹配不上**。后果：**装了自带字体在页面上完全没有效果**
（静默回退雅黑），而用户明明在字体库里看到了它、也点了下载。这正是用户最初那句
"字体预览和实际的对不上"的**又一个实例**，只不过这次是"装了等于没装"。

**修法**：`SYSTEM_FONT_STACK` 首项改为**运行时算**（`preferred_fallback_family()`）：
① 库里设过默认的那支 → 用它的内部族名；② 否则内置那支中文（装了才给）；③ 都没有 → 回落
`'LXGW WenKai'` 字面量（系统里真装了这个字体的人照旧命中）。

⚠️ **顺序一个字都不能动**：首项仍排在 `'Microsoft YaHei'` **前面**。
`dhampir.rs::font_search_order` 与 `webui/src/components/PolishPreview.vue` 的 `.cap` 栈
必须同序（历史上"预览是雅黑、成片是 Noto"就是这里漂的）。

**实测（真跑）**：
- 库里只有 emoji 时 → `"myfont","LXGW WenKai",'Microsoft YaHei','Noto Sans SC',sans-serif`（回落）
- 装了霞鹜文楷 Lite 后 → `"myfont","霞鹜文楷 轻便版",'Microsoft YaHei','Noto Sans SC',sans-serif` ✅

### 16.5 顺带修掉的两处"看起来对、其实不对"

1. **`cover_static_capability()` 数错**：它手写"只数顶层文件"，v2 目录布局下**恒为 0**
   ⇒ `/api/health.cover_static.fonts` 报 0 ⇒ 前端把 Takumi 选项**置灰** —— 明明有字体却
   "用不了静态封面"。改为与 `cover_font_files()` **同源**（同一个函数），且**只数真能用的**
   （未下载的自带字体不算，否则报告"有字体"然后在出片时炸在"至少一个字体文件"）。
2. **`import_font` 从不记 `sha256`**：`FONT.toml` 里那个字段一直是空串（P1-b 说过要填）。
   现在自带下载写**校验过的实际值**、用户导入也写 `sha256_hex(&bytes)`；
   `added_at` 同样补上。⚠️ 一度把 `..Default::default()` 删了导致 `E0063`，已加回。

### 16.6 验收（全部真跑过）

```bash
cargo test -p yekitrim-core --lib fonts          # 14 passed
cargo test -p yekitrim-polish --lib              # 214 passed
cargo test -p yekitrim-server                    # 145 passed（含新增 builtin 路由的 admin 门禁）
cargo check --workspace --all-targets         # 0 warnings
cargo check -p yekitrim-polish --features cover-takumi --all-targets   # 0 warnings
python3 scripts/check_module_map.py           # 35 模块 / 7 项
python3 scripts/check_module_boundaries.py    # 15 规则 / 244 文件 / 0 违规
```

**联网真跑**（不是 mock）：
1. `yekitrim fonts list` → 4 支（2 支自带未安装 + 2 支老布局），未安装的标「未安装（可下载）」。
2. `yekitrim fonts download --id NotoColorEmoji` → 3.6 秒装好，落盘形状与 §16.2 一致。
3. `yekitrim fonts download --id LXGWWenKaiLite-Regular` → 3.0 秒装好。
4. 再跑一次 `download` → **`downloaded = []`，271.9µs**（幂等成立，没重复下 25 MB）。
5. 校验和复核：`FONT.toml` 里的 sha256 **与清单逐字相同**；`LICENSE.txt` 大小
   5176 / 4386 B，与上游 `OFL.txt` 一致。
6. 未知 id → `不是自带字体（自带清单里没有它）: xxx`（400 侧同文案）。

### 16.7 ⚠️ 测试环境陷阱（下次别踩）

`yekitrim_core::path::font_dir()` 按**可执行文件所在目录**算，而 cargo 的集成测试二进制跑在
`target/<profile>/deps/` 下 ⇒ 测试看到的库是 **`deps/resources/fonts/`**，与 `yekitrim` 主程序
（`target/<profile>/resources/fonts/`）**不是同一个目录**。第一次实测时"装了字体栈却没变"
就是这条 —— **代码是对的，探针看错了目录**。要在测试里验"装了之后栈变不变"，
得往 `deps/resources/fonts/` 里放（或直接用单测构造，不要靠机器上恰好装了什么）。

### 16.8 未做（明确记账）

- **`.ttc` 自动拆包**：仍是有意**拒收 + 报错**（浏览器 `@font-face` 不认集合）。要不要做拆包
  留待用户拍板。
- **「设为默认」只影响推荐位**：不改任何工程的 `font` 配置（有意）。
- **离线兜底**：下载失败就是失败（原话回给用户），不静默换别的字体。


---

## 17. P2 落地记录：封面默认通道切到 takumi（2026-10-01）

### 17.1 前置条件（P1-e/P1-f 解锁的）

切默认之前必须证明"字体统一之后，takumi 出的封面与参照仍然达标"。重跑
`scripts/check-cover-takumi.mjs`（真工程 clip-14，**活的 chrome 双腿对拍**）：

| 指标 | 实测 | 阈值 | 余量 |
|---|---|---|---|
| 非文字区 均值 | **1.17/255** | 4 | 3.4× |
| 非文字区 >32 | 0.58% | 1.0% | 1.7× |
| 整图 均值 | **11.44/255** | 22 | 1.9× |
| 整图 >32 | 7.75% | 13% | 1.7× |

⇒ 达标。而且这一版是**同库同字体**的对比 —— P1-e 之前两边字体来源不同，"对不上"里混着
字体来源的差；现在剩下的只有栅格化固有差。

### 17.2 改动

`resolve_cover_renderer` 的空值回落由 `chrome` 改为 **`takumi`**。
优先级不变：**显式参数 > `VTEDIT_COVER_RENDERER` > takumi**；认不出的值仍当场报错。

实测三条路（真跑）：
- 不传 `--renderer`、不设环境变量 → 走 takumi（日志 `封面(takumi)`，注册 4 个字体，4.1s）
- `--renderer chrome` → 仍能走 headless Chrome（兜底没坏）
- `--renderer bogus` → `Error: --renderer 认不出 bogus（只认 chrome / takumi）`（不静默回退）

### 17.3 ⚠️ 一个"假绿"陷阱（**记住它**）

默认通道改成 takumi **之后**，门禁脚本里 chrome 那条腿传的还是 `{}`（吃默认）——
于是它**又渲染了一遍 takumi**，冻出来的"参照帧"是 takumi 的，门禁报 **0.32/255**：
自比自当然几乎为 0，看着像"完美通过"。**判据**：
`PSNR(ref, chrome)=45.97dB` vs `PSNR(ref, takumi)=20.67dB` —— 谁近谁是真身。

教训：**改了默认值，所有"靠默认值"的调用点都要重新过一遍**；显式传值的地方不会漂，
吃默认的地方会静默变味。现在两条腿都显式给 `VTEDIT_COVER_RENDERER`。

### 17.4 副产品：takumi 是**逐字节确定**的

连跑三次同一工程，`cover.jpg` 的 sha256 **完全相同**（`6d27cae4…`）。
这比 chrome 强（chrome 每次都有轻微差异）—— 意味着冻结参照帧是**长期稳定**的基线，
不会因为"参照自己漂了"而假红。


---

## 18. P3-a 落地记录：冻结封面参照帧（删 chrome 的前提）

### 18.1 为什么必须"先冻结、后删除"

这道门禁原本的参照是**活的 chrome 渲染**（`run(A,'chrome',{})`）。chrome 一删，参照就没了 ——
门禁会退化成 SKIP，也就**再也没有东西保证 takumi 没坏**。
做法抄 `scripts/yekitrim-compare.mjs` 的 `--capture-golden/--golden` 先例（同类问题已有解法）。

### 18.2 新增的三个开关

| 开关 | 作用 |
|---|---|
| `--capture-golden <目录>` | 跑一次 chrome，把产物冻成 `ref-cover.jpg` + `golden.json` |
| `--golden [目录]` | **完全不渲染 chrome**；不给目录就用**入库的** `scripts/fixtures/cover-golden/` |
| `--mutate` | **变异自证**：给非文字区人为加偏差，证明判据能红也能绿（秒级、不渲染） |

### 18.3 冻结产物（**入库**，320 KB）

```
scripts/fixtures/cover-golden/
  ref-cover.jpg   327,147 B（1920×1080，chrome 产物经 PNG 中转后 q=2 JPEG）
  golden.json     标定记录（冻结时间 / 来源 / 标题矩形 / 验证方法）
```

⚠️ **经 PNG 中转**：chrome 那份本身就是 q=85 的 JPEG，直接再存一遍等于**二次编码**、噪声会叠。
先从它转 PNG 再压 q=2，只吃一次编码损失。

### 18.4 验收（四模式全跑过）

- `--self-test`：3 项通过（全同必过 / 黑白必红 / 分区独立生效）
- `--mutate`：原样绿、非文字区提亮 60 后红（均值 56.37）
- `--golden`：非文字区 **1.60**、整图 **11.84** → ✅（**不需要 chrome**）
- 默认双腿：非文字区 1.17、整图 11.44 → ✅（chrome 还在时的过渡态）

### 18.5 golden 模式的稳定性与余量

连跑三次，**数字完全一致**（1.60 / 11.84）。余量：四项指标都在阈值的 40%~60%，
即 **1.7×~2.5× 余量** —— 不是"卡在阈值边上勉强过"。

> ⚠️ `--golden` 的数字（1.60/11.84）比双腿模式（1.17/11.44）**略高**，这是预期的：
> 参照帧多经了一次 JPEG 编码（q=2）。差值 0.4/255 远小于阈值余量，不影响判据。


---

## 19. P3-b 落地记录：Chrome 链路整条删除（2026-10-01）

### 19.1 删掉了什么

| 位置 | 内容 |
|---|---|
| `crates/yekitrim-polish/src/render/chrome.rs` | **整文件**（328 行，`headless_chrome::{Browser,LaunchOptions,Tab}`） |
| `render/mod.rs` | `pub mod chrome;` |
| `render/backend.rs` | `RenderBackendKind::Chrome` 变体 + `as_str` + `from_str`（含老的 `"chome"` 错拼别名）+ `create_backend` 那一臂 |
| `render/pipeline.rs` | 全部 `renderer == Chrome` 分支（逐帧 canvas 那条 + `start_file_server` + `RenderServer::build_html`） |
| `cover.rs` | `RENDERER_CHROME` 常量 + 内联的 headless 截图路径 |
| `Cargo.toml` | `headless_chrome` 依赖 + `chrome-render` feature |
| server | `/api/health` 的 `chrome` 能力位、`/api/polish/render` 白名单里的 `chrome` |
| WebUI | 出片通道下拉（`PolishView`）、封面通道弹窗（`CoverDesigner`） |
| **`[chrome] path` 配置** | 见 19.3 —— 这是收尾时**额外发现**的死配置 |
| 文档 | README / THIRD-PARTY-NOTICES / 12 张 `AGENTS.md` 卡 / `cli.rs --renderer` 帮助 |

### 19.2 语义决定：`chrome` **报错**，不静默回退

- 出片侧：`RenderBackendKind::parse("chrome")` → `None`，由 `pipeline.rs` **一处**报错。
  **没有**把它归到 `dhampir` 上 —— 那是静默换路，而两版成片的差别只有对着看才发现
  （这条仓库里已经踩过一次）。
- 封面侧：`normalize_cover_renderer("chrome")` 给的是**单独的**「这条腿已删除」错误，
  **不是**折进"认不出"那一支 —— 折进去用户只会去猜自己是不是拼错了。
- 代价（**已知且有意**）：老脚本里的 `--renderer chrome` / `VTEDIT_COVER_RENDERER=chrome`
  从"能跑"变成**硬报错**。这是唯一一处用户可见的破坏性变化，换的是"不会以为自己在跑 chrome"。

### 19.3 收尾时额外发现的死配置：`[chrome] path`

`yekitrim-core` 的 `Config.chrome: ChromeConfig`（+ `[chrome]` 模板段 + `merge()` 分支 +
`SettingsView.vue` 的「Chrome 渲染」输入框）在删掉出片/封面两条链路之后**已经没人读了** ——
`tool_probe.rs` 里那个 Chrome 探测函数也随任务一起没了，只剩它自己跟自己的 `merge()`。
留一个"Chrome 可执行文件路径"输入框会让人以为**装了 Chrome 就能用上**，实际它什么也不影响。
⇒ 四层一起删（字段 / 结构体+Default / 模板段+merge / 前端表单与 hydrate）。

### 19.4 验收（删 chrome 之后的完整一轮）

```
cargo check --workspace --all-targets   → 0 warnings
cargo test -p yekitrim-polish --lib        → 214 passed; 0 failed
cargo test -p yekitrim-server              → 145 passed; 0 failed
cargo test -p yekitrim-core --lib          → 177 passed; 0 failed
cargo test -p yekitrim-service             → 125 passed; 0 failed
check-cover-takumi --self-test / --mutate → 3 项通过 / 判据能红也能绿
check_module_map.py / check_module_boundaries.py → 35 模块 / 15 规则 0 违规
cargo tree -p yekitrim-polish | grep -i chrome → 空
webui/dist 里 chrome 字样                      → 空（重建后）
```

**端到端真跑（不是只编译）**：`clip-14` 真工程出片成功 ——
`vtuber-clip_1.mp4`（160.7 MB / 145s）+ Takumi 封面（324,943 B，hash `263d2e42`）。
即：删掉 chrome 之后，**出片（底座）+ 封面（Takumi）两条路都还在走**。

唯一剩下的 `headless_chrome` 字样在 `render/backend.rs` 的**注释**里
（"曾经有 Chrome，2026-10-01 整条删除"）—— 那是有意留的历史说明，**不是引用**；
`grep` 判据要把注释排除掉，否则会永远红。

### 19.5 ⚠️ 排查中被误导的一件事（记下来省下次的时间）

跑 `--test portrait_backdrop` 时它一路失败，看着像"删 chrome 把探针搞坏了"。**实际是两个独立问题**：

1. **真 bug（已修）**：探针的变异锚点写的是 `drawStageBackdrop(` —— 那是
   `templates/index.html` 里的函数名，**转译器根本不调它**（它把衬底内联成 `backdropTracks`，
   那个名字只在**注释**里出现过一次）。锚点命中 0 次 ⇒ 变异体 == 原件 ⇒
   **A/B 比的是同一份产物**，判据"自己跟自己比"。这正是那段代码注释里警告的静默失效形态。
   已改成锚在真实的那道闸 `if (BACKDROP_ENABLED && STAGE_BG === 'blur') {`（恰好 1 次），
   并且**两次变异都要先打开 `BACKDROP_ENABLED`** —— 它默认 `false`，
   只变异一侧的话两边都不画衬底，**还是**自己比自己。
2. **环境问题（非代码）**：探针要在一个进程里**渲染两遍**，第二遍 `wgpu error: Out of Memory`
   （显存不够，**不是**磁盘 —— C 盘还有 34 GB）。Dhampir 退出 101。
   同一台机器上 `yekitrim polish render` **单遍**跑得好好的。⇒ 与本次改动无关。

> 教训：**探针失败先看它的锚点命中几次**。"0 次"是静默空转，"1 次"才是真在比；
> 而"看起来像回归"的失败，也可能只是这台机器的显存不够。


---

## 20. ⚠️ 复验时纠正的一条**错误认知**：Takumi 的 fallback 不是"回退到 Geist"

### 20.1 原先写在代码里的说法（**错的**）

`render/takumi.rs` 那段注释与 `tests/takumi_font_id.rs` 的判据都建立在同一个前提上：

> Takumi **只按内嵌族名匹配**；CSS 请求库 id 而两者不等时，它**静默回退到内置 Geist**
> ⇒ 中文变豆腐块。

### 20.2 实测结论（本机，2026-10-01）

Takumi 的 fallback 桶 = **已注册字体按注册序的全集**
（`takumi-core-0.25.0/src/resources/font.rs`：`order` + `last_resort_order`）。
**认不出的族名不是回退到内置 Geist，而是回退到整个字体库** —— 挑第一支"有该字形"的字体来画。

| 实验 | 结果 |
|---|---|
| 只注册 LXGW，分别请求 `__nope__` / 库 id / 内嵌族名 | 三张图 **逐字节相同** |
| 注册 [Emoji, LXGW]（Emoji 在前），请求 `__nope__` / `NotoColorEmoji` / `LXGWWenKaiLite-Regular` | 三张图 **仍然逐字节相同**，中文都正常渲染（落到 LXGW） |

⇒ **多支字体时"用哪支"由注册顺序决定，不是由 CSS 族名决定。**
⇒ "请求不存在的族名 → 豆腐块"这个失败模式**在本版 Takumi 里根本不可达**。

### 20.3 因此改了什么

1. **`tests/takumi_font_id.rs`**：原断言 `d_id_miss > 0`（"库 id 必须不同于'请求不存在的族名'"）
   **必然失败且断言的东西不成立**。改成钉能钉的事实（两者**应当**相同），
   并在断言消息里写明"若哪天变红 ⇒ Takumi 收紧了 fallback ⇒ 正好该回到'别名是否生效'这个判据"。
   现在这条测试**通过**。
2. **`render/takumi.rs` 的注释**：纠正为实测结论，并说明**保留别名注册的理由**
   —— ①注册序里 id 与实际字体一一对应、不改变现状；②未来 Takumi 若收紧 fallback，
   别名能让 CSS 立刻对上，**属于前瞻保险**。
3. 记下真正的控制点：**`cover.rs::cover_font_files()` 给出的顺序**才是"用哪支"的决定因素
   —— 那才是将来该加门禁的地方。

### 20.4 这条为什么值得单独记

"P1-e 修好了预览与实际对不上"这个结论**本身仍然成立**（字体确实统一到了库 id 这条链上，
封面出图用的是 LXGW 而非豆腐块），但**它成立的原因不是当初写的那个**。
一个**建立在错误前提上的测试**比没有测试更危险：它会让后来的人以为
"这条路径被验证过了"，而实际上它验证的是一个不存在的失败模式 —— 而且它**一直是红的**，
只是被 `#![cfg(feature = "cover-takumi")]` + 默认不带该 feature 的运行方式**藏住了**。

> ⚠️ 教训：**带 feature gate 的集成测试，必须用"实际出片时的那套 feature"跑一遍**。
> `cargo test -p yekitrim-polish`（默认不带 `cover-takumi`）会把这个文件整个编译掉、
> 报 `0 passed` —— 看着像"没有测试"，实际是"测试被跳过了"。

---

## 21. P1-g 落地记录：400 修复 + 管理页收进首页区块（2026-10-02）

### 21.1 「下载自带字体 400」的根因：body 没序列化

用户报：首页点「下载自带字体」→ `下载失败：接口不存在或返回格式错误: /api/polish/fonts/builtin (400)`，
且库一直停在「已装 0 支 · 自带还缺 2 支」。

- **根因不在服务端**：`importFont` / `setDefaultFont` / `downloadBuiltinFonts`（`webui/src/api/polish.js`）
  从 f38eb4e 出生起就把 **裸 JS 对象** 传给 `api()` 的 `body`，而 `client.js` 不做序列化（约定是调用方自己
  `JSON.stringify`，全仓其它接口都这么写）。`fetch` 对非 BodyInit 的对象走 USVString 转换，实际发出去的
  body 是字面量 **`[object Object]`** → axum 的 `Json` 提取器 `JsonSyntaxError` 回 **400 + `text/plain`**
  → `client.js` 只认 JSON，把这条报成「接口不存在或返回格式错误」。
- **后果面**：三个字体写接口（导入 / 设为默认 / 下载自带）**全坏**，只有 DELETE（无 body）能通；
  「设为默认」表面有按钮、点了必 400。修复 = 这三处补 `JSON.stringify(...)`，并把这条约定
  （**裸对象会被 fetch 转成 `[object Object]`**）写进 `polish.js` 的注释里。
- 顺带澄清「之前误删了」：两支自带字体可以因换目录/手动清库而缺席，但**重新下载的入口一直被这个
  400 挡死** —— 修好它，缺的字体一键就能回来（幂等）。

### 21.2 管理页收进首页区块（用户拍板：不要单独页面）

用户：字体库 + 管理页 UI「其实只需要这个区域显示即可，而不是单独的页面」。

| 文件 | 改动 |
|---|---|
| `webui/src/components/FontLibrary.vue` | **新组件**（照 `StickerPacks` 的首页区块模式）：状态行 + 导入（`ServerDirPicker`）+ 缺自带下载条 + 卡片墙（预览/族名/徽标/设默认/删除）+「测试文本」，原 `FontsView` 的全部能力 |
| `webui/src/views/FontsView.vue` | **删除**（能力整体迁进上面那个组件，无一丢失） |
| `webui/src/router.js` | 删 `#/fonts` 路由 |
| `webui/src/views/WorkbenchView.vue` | 字体卡换成 `<FontLibrary v-if="isAdmin" />`；`homeFonts*` 状态、route watch 里的字体重查、`.wb-fonts` 样式一并删除 |
| `webui/src/api/polish.js` | §21.1 的三处 `JSON.stringify` |
| `crates/yekitrim-server/src/api/polish.rs` | 仅注释：`FontsView.vue` 指针改指 `FontLibrary.vue`（跨模块纯注释改动，理由在此） |

- 保留口径：写操作仅 admin（整块 `v-if="isAdmin"`）、读失败可见可重试、下载失败原话显示、
  预览注册出片同源字节、认不出的格式挂「预览不可用」—— 全部原样随组件迁移。
- 组件级细节的权威记录在 `docs/webui-components.md` ⑥（`webui/src/components/AGENTS.md` 的指令链
  `root+webui+webui-src+webui-components` 只剩 10 B 余量，卡里一个字都加不进 —— 这是预算墙，不是遗漏）。

### 21.3 复跑

```bash
npm --prefix webui run build                  # dist 重建（FontsView-*.js/.css 退出，FontLibrary 并入 Workbench chunk）
node webui/scripts/check-*.mjs                # 7 个契约脚本
python3 scripts/check_module_map.py           # 结构门禁（views 卡替换段落净缩短）
python3 scripts/check_module_boundaries.py    # 边界门禁
cargo check -p yekitrim-server                   # polish.rs 注释改动
```

### 21.4 复验记录（2026-10-02，真机浏览器）

- **下载自带字体**：真实下载落盘 `~/.yekitrim/fonts/{LXGWWenKaiLite-Regular, NotoColorEmoji}/`（14 + 25 MB，
  各含 `font.ttf + FONT.toml + LICENSE.txt`），首页状态行变「已装 2 支」。
- **设为默认**：浏览器点按钮 → `is_default` 服务端翻转 + 卡片 chip 实时跟移，往返各一次均正常。
- **顺带修掉一个从 FontsView 一路继承的响应式 bug**：`registerAll(list)` 遍历的是 load() 里的**原始数组**，
  `f.ok = true` 写在原始对象上 —— Vue 模板（走代理读）永远看不见，卡片**永久卡在「预览不可用」**
  （`document.fonts` 里其实两支都 `loaded`）。现在 `registerAll()` 遍历 `fonts.value`（响应式代理）；
  顺带把 chip 的判据从 `!f.ok`（注册进行中也为真）改成 `f.ok === false`，注册中不再误报。
- 预览几何断言：`.fcard .preview` 高 47px > 0，计算字体族指向注册的同名 FontFace。

---

## 22. 字体职责分离 + 输入层收敛（2026-10-02）

### 22.1 两个目标

1. **输入层收敛**：所有用字体的功能，字体输入走**同一个标准接入**，以保证库的正确性。
2. **职责分离**：字体不一定含 emoji ⇒ **管字的**与**管表情的**分开。

### 22.2 收敛前：7 个输入口、3 种形态、7 处写死栈

| 形态 | 输入口 |
|---|---|
| **库 id** | `[style.subtitle] font` · `[cover] font` · `subtitle_font` API · `cover.font` API |
| **文件绝对路径** | `VTEDIT_DHAMPIR_FONT`（env，**优先于一切**） |
| **写死的系统族名栈** | `server.rs::SYSTEM_FONT_STACK` · `cover.rs::EMOJI_FONT` · `takumi.rs::ensure_emoji_fallback` · `PolishPreview.vue` 字面量 · `CoverDesigner.vue` ×3 · `tokens.css` · `web.rs` |

同一份知识写 7 遍、靠注释互相提醒"要同序"。漂移的症状是**预览和成片不一样，而且不报错**。

### 22.3 收敛后：`yekitrim_core::fonts` 是唯一权威

```rust
pub struct FontPlan {
    pub text_family: Option<String>,   // 正文族名（用户选的那支的内部族名）
    pub selected_raw: String,          // 库外 id 原样保留（当系统族名用）
    pub emoji_family: Option<String>,  // ★ emoji 补充槽位（独立字段，不是拼进 fallback）
    pub fallback: Vec<String>,         // 兜底链（不含 emoji）
}
pub fn resolve_font_plan(root: &Path, selected_id: Option<&str>) -> FontPlan;
pub fn emoji_font(root: &Path) -> Option<FontEntry>;   // 至多一支，自动选
pub fn fallback_chain() -> Vec<String>;                // 兜底链唯一一份定义
```

**为什么 `emoji_family` 是独立字段、而不是拼进 `fallback`**：三条消费链语义不同，必须各自决定怎么用 ——

| 消费链 | 怎么用 emoji 槽位 |
|---|---|
| 字幕**预览**（浏览器） | 追加到 `font-family` **栈尾** |
| 封面（Takumi） | **不看 font-family 栈**，看**注册顺序** ⇒ emoji 排**最后注册** |
| 字幕**成片**（Dhampir 底座） | **用不上** —— 它没有逐字形回退 |

拼成一个字符串栈会让后两条被误导（A2 那条计划就是这么被推翻的）。

### 22.4 emoji 字体的三处拦截（A4）

emoji 字体**不参与正文竞争**，三道闸：

1. `fonts::set_default` —— 直接 `bail!`（"emoji 字体自动补充，不参与默认"）；
2. `fonts::scan_dirs` 出口 —— `is_default: meta.is_default && !meta.emoji`（**最关键的一处**：
   `is_default` 有两个真消费者 —— `preferred_fallback_family` 选**栈首**、`cover_font_files_in` 选**注册序第一支**，
   让 emoji 拿到默认位会让**整条栈的中文全部落空**）；
3. `resolve_font_plan` —— 不给 emoji 正文族名（用户手改配置显式选它也没用）。

界面侧：两个下拉滤掉 `f.emoji`（C9）+ 字体库卡片对 emoji 不给「设为默认」（C10，改成「自动补充」静态标签）。

### 22.5 ⚠️ emoji 字体必须 CBDT / COLRv0，**不能是 COLRv1**

**这是本节最重要的一条**，实测字节定的：

```
takumi-core-0.25.0/src/resources/glyph.rs:440-443
    self.color_glyphs.get_with_format(glyph_id, ColorGlyphFormat::ColrV0)?   // 写死 V0
。
。
三级解析：① 位图 strikes → ② 只问 COLRv0 → ③ 普通 glyf 描边
```

上游 `google/fonts` 的 `main` 分支那份 Noto Color Emoji **早已迁到 COLRv1** ⇒ Takumi 不问它，
一级级落空。**实测结果不是"渲染成单色剪影"（那还是看得见的），而是 emoji 整片消失** ——
与"一个 emoji 字体都没有"的产物**逐字节相同**（sha `f1af84a1`），且**不报错**、界面照样显示已装。

⇒ 自带字体已钉到 **`googlefonts/noto-emoji` 的 `v2020-09-16-unicode13_1`**（COLRv1 迁移前的最后一版）：
实测表结构 **`CBDT + CBLC`、无 COLR、无 sbix**，且**连 `glyf` 都没有** ⇒ 不存在退化成剪影那条路。

| 字段 | 值 |
|---|---|
| id | `NotoColorEmoji-CBDT`（**必须换新 id**，见 22.6） |
| url | `github.com/googlefonts/noto-emoji/raw/v2020-09-16-unicode13_1/fonts/NotoColorEmoji.ttf` |
| sha256 | `2b106b88034043001f28b3ee8e58f62e867f0f0666ea072a4acf6cbd956ca160` |
| bytes | 10,419,948 |
| 内部族名 | `Noto Color Emoji`（与旧那支相同 ⇒ 匹配口径不变） |

### 22.6 ⚠️ 换 id 造出的"新旧并存"窗口（执行时踩到的坑）

换 id 的理由是"下载幂等（已装就跳过）⇒ 复用同 id 老机器永远不重下"。但**换 id 同时制造了另一个问题**：

```text
"NotoColorEmoji" < "NotoColorEmoji-CBDT"   （纯 id 字典序）
=> emoji_font() 选到旧那支 COLRv1 => 封面 emoji 整片消失
```

而界面显示"已装"、接口照样回 `emoji_family` —— **没有任何一处会报错**。
修法：`emoji_font()` **自带（清单内、验证过格式）优先**，其余按 id 稳定序；
回归钉子 `builtin_emoji_wins_over_a_stale_leftover`。

> **教训**：换资源 id 时，除了想"新机器怎么拿到"，还要想"**老机器上旧那份怎么办**"。

### 22.7 三条消费链的最终状态

| 消费点 | 状态 | 判据 |
|---|---|---|
| 字幕**预览** | ✅ 支持（浏览器自带系统 emoji + 库里那支追加栈尾） | `capFallback` 追 `emojiFamily` |
| 封面**标题 + emoji 特效层** | ✅ 支持，**实测彩色** | `cover_font_files_in` emoji 收尾 + `emoji_font_stack()` |
| 字幕**成片** | ❌ **本仓不可修**，已记档 | `font_search_order` 的钉子单测：emoji 族不许进搜索序 |

**为什么字幕成片修不了**（不是"还没做"，是**这条链路结构上不支持**）：
`dhampir.rs::find_font` 命中第一个能解析的族就 `return`，只交出**一个**文件；
底座 `pick_font` → `Option<PathBuf>`，最终落 ffmpeg `drawtext=fontfile=<单支>` ——
**没有回退链、没有 fontconfig**。要支持得改底座契约（逐字形 runs + 彩色位图光栅化）。
⚠️ 反过来：emoji 族一旦混进 `font_search_order` 并排在前，**整条字幕轨**都会交给一支没有中文字形的字体
—— 中文全变豆腐块（`dhampir.rs` 有单测钉住这条）。

### 22.8 门禁：`scripts/check-font-inputs.mjs`

防"第 8 份写死又长出来"。**两向判据**：

1. **登记的来源必须真的存在**（权威符号 + 白名单文件都必须在，且白名单那些文件确实还有字面量 —— 防腐坏）；
2. **不许有没人认领的字体栈字面量**（扫 `crates` / `webui/src` / `scripts`）。

⚠️ **必须认两种形态**（踩过）：`font-family:'A','B',sans-serif`（CSS）**和**
`pub const EMOJI_FONT: &str = "'A','B',sans-serif"`（**裸常量**）—— 后者不带 `font-family:` 前缀，
而 `EMOJI_FONT` / `SYSTEM_FONT_STACK` 正是最该收敛的两处；只写 CSS 判据会让门禁"绿着"放过它们。

⚠️ **测试模块要跳过**（`#[cfg(test)]` / `mod tests` 之后的行）：那里的字体栈是**夹具**，
不是产品里的字体选择（踩过：给 `ensure_emoji_fallback` 写的幂等单测被自己的门禁报红）。

`--self-test` **7 项**，已进 `.workbuddy/gates.mjs`（主检查 + 自检**都进** —— 同 `check-effect-registry` 的教训）。

### 22.9 验证

```bash
cargo test -p yekitrim-core --lib fonts        # 22 项（含 emoji 槽位/拦截/优先序 6 条）
cargo test -p yekitrim-polish --lib render     # 29 项（含零回归钉子/幂等/emoji 不入搜索序）
cargo test -p yekitrim-server                  # 151 项
node scripts/check-font-inputs.mjs          # 字体输入面（+ --self-test 7 项）
node .workbuddy/gates.mjs                   # 全部门禁
```

出图证据留档 `.workbuddy/emoji-cover-test/`：`s2.jpg`（只有 CBDT ⇒ 全彩）、
`s1.jpg`（只有旧 COLRv1 ⇒ 与无 emoji 字体逐字节相同）、`FIXED-with-both-fonts.jpg`（两支都在 ⇒ 全彩）。

---

## 23. 自带字体改「内嵌」（2026-10-02 用户拍板）

### 23.1 为什么改

原来的方案是「首次使用时按需下载」（`yekitrim fonts download` / WebUI「下载自带字体」）。两个问题：

1. **多了一个莫名其妙的操作** —— 用户要先「下载字体」才能「选字体」。这不是用户的心智模型。
2. **离线/内网环境下静默降级** —— 没网就装不上，而症状是「字体回落到系统栈」，
   用户根本不知道少了什么（不是报错，是观感悄悄变了）。

`dsh` 那边一问「首次启动时自动下载吗」，答案是**不会**（下载全是用户显式触发的），
于是这个缺陷就显出来了：**新机器上从没点过下载 ⇒ 库里没字体 ⇒ 封面/字幕用不了库内字体**。

### 23.2 做法：`include_bytes!` 进二进制

字体字节入库到 `crates/yekitrim-core/assets/fonts/`，经 `include_bytes!` 编进 `yekitrim.exe`：

```
crates/yekitrim-core/assets/fonts/
  LXGWWenKaiLite-Regular.ttf    13.2 MB
  LXGWWenKaiLite-Regular.OFL     5.1 KB
  NotoColorEmoji-CBDT.ttf        9.9 MB
  NotoColorEmoji-CBDT.OFL        4.3 KB
```

- `BUILTIN_FONTS` 多了两个字段：`bytes` / `license_bytes`。
- `install_builtin_font(root, b)` 从「下载 + 校验 + 改名」变成「**校验内嵌字节 + `fs::write`**」
  —— **同步**、不联网。`sha256` 那条校验**保留**（内嵌字节也要核对源码里钉的常量）。
- `install_builtin_fonts` / `ensure_builtin_fonts()`：后者挂在 `ensure_dir()` 之后的启动路径上
  （`init_config.rs` 的 `config init` 与 `ensure_global_config` 两处），**失败只 warn、不拦启动**。
- CLI `fonts download` → **`fonts install`**；HTTP `POST /polish/fonts/builtin` 语义不变（仍是修复入口）。

### 23.3 授权依据（**别再把字节拿掉**）

两支都是 **SIL OFL-1.1**，OFL 正文原文：

> The fonts, including any derivative works, can be **bundled, embedded, redistributed and/or sold
> with any software** provided that any reserved names are not used by derivative works.

⚠️ **README 里「不包括嵌入式用途」那句说的是乐米波波体**（第三方字体，杭州乐米网络科技），
**不是这两支自带字体**。看到那句话就把这里的字节拿掉 = 误读。

### 23.4 顺带修掉的一个真 bug：许可 URL 404 → 存了 HTML 错误页

`NotoColorEmoji` 的 `license_url` 原指 `.../raw/main/fonts/LICENSE` —— **实测 404**
（文件挪过位置）。而 GitHub 对 404 回的是 **HTML 页面**、下载器又不校验内容，
于是 `LICENSE.txt` 里存了 **268 KB 的 GitHub 报错页**（OFL 要求随附的许可原文，成了垃圾）。

修正：`license_url` 钉到**我们那份字体的 tag**（`v2020-09-16-unicode13_1/fonts/LICENSE`，
与 `url` 同源）；并且**许可走内嵌字节**（`license_bytes`）—— 它不该依赖网络，
必须与字节同在，否则这个包的授权状态说不清。

⚠️ 注意仓库根 `LICENSE` 是 **Apache-2.0**，而 `fonts/LICENSE` 才是 **OFL-1.1** —— 别取错那一层。

### 23.5 规模代价

| | 改前 | 改后 |
|---|---|---|
| debug `yekitrim.exe` | 81 MB | **104 MB** |
| 仓库新增 | 0 | **+23.2 MB**（4 个文件入库） |
| 首次启动 | 需联网下载 ~38 MB | **不联网**，本地写 23.2 MB |

### 23.6 顺带改掉的界面口径（同一次用户拍板）

1. **字体下拉去掉「（系统默认）」**（`PolishView.vue` / `CoverDesigner.vue`）：自带字体就在库里，
   再给一个「回到系统字体栈」的选项 = 给一个**必然更差**的选择。空值仍合法（老工程就是这一档），
   但只显示成「（跟随默认）」，不再作为可选项。
2. **「设为默认」按钮撤掉**（`FontLibrary.vue`）：默认位只作**初始化设定**
   （老工程没显式选过字体时用哪支），不是给用户点的操作 —— 留按钮会让人以为
   "选了它所有工程的字幕都变"，而实际只影响没选过的那些。
   ⚠️ 服务端 `set_default` 与 CLI 面**保留**（初始化/修复要用），只是界面不再暴露。

### 23.7 验收

- 单测：`embedded_bytes_match_their_pinned_sha256`（内嵌字节 vs 源码常量）、
  `embedded_bytes_are_parseable_fonts`（不是空壳/HTML）。
- **真机全过程**：把 `~/.yekitrim/fonts` 整个移走（模拟全新机器）→ 跑一次 `yekitrim config init`
  → 日志 `自带字体已就绪（本次安装 2 支）` → 库里出现两支（`FONT.toml` 完整、`LICENSE.txt` 是**真 OFL 全文**）
  → `yekitrim polish cover --renderer takumi` 出图：**中文 + emoji 全彩**。

---

## 24. P1-h 落地记录：库内路径 ASCII 化（2026-10-02，用户拍板「方案 B」）

### 24.1 症状与根因（一次线上事故，不是设计洁癖）

用户报「出片速度基本卡死、字幕像是没有」。实测（同一份 `clip-14` 工程、同一台机器、同一个
`dhampir.exe`、同样 240 帧 `0..239`，**只换 `--font-file`**）：

| `--font-file` | 240 帧墙钟 | 画出的字幕 / 弹幕 | 栅格失败 | exit |
|---|---|---|---|---|
| `C:/Windows/Fonts/msyh.ttc`（ASCII） | **4.25s** | 216 行 + 239 条 | 0 | 0 |
| 同一支字体**复制到 ASCII 路径** | **3.96s** | 216 行 + 239 条 | 0 | 0 |
| `…\.yekitrim\fonts\乐米波波体\乐米波波体.ttf` | **37–80s** | **0 + 0** | 455 | 1 |

**根因**：底座出片把字体路径塞进 `ffmpeg` 的 `drawtext=fontfile='…'`。ffmpeg 那条路**打不开含
非 ASCII 字符的路径**（`textfile=` 与 `-i` 都不怕中文，只有交给 FreeType 的 `fontfile` 怕），
而且打不开时**不报错、直接访问违例**（`0xC0000005`；`%LOCALAPPDATA%\CrashDumps\ffmpeg.exe.*.dmp`
就是它）。失败**不进光栅缓存** ⇒ 每条字幕/弹幕都重起一个必崩的 ffmpeg（~150ms/次）。
裸 ffmpeg 即可复现（不经底座、不经 YekiTrim）：

```bash
ffmpeg -v error -nostdin -f lavfi -i color=c=black@0.0:s=400x120,format=rgba \
  -vf "drawtext=fontfile='<字体>':textfile='<任意 txt>':fontsize=48:fontcolor=white:expansion=none:x=(w-text_w)/2:y=(h-text_h)/2" \
  -frames:v 1 -f rawvideo -pix_fmt rgba -
# ASCII 路径 → 0；中文路径 → 0xC0000005；**路径不存在** → 同一个 0xC0000005
```

（那句 `Fontconfig error: Cannot load default config file` 是**红鲱鱼**：成功的调用里也有它，
只是失败时才会被底座的错误消息带出来。）

ffmpeg 与底座都不在本仓手里 ⇒ **只能保证库内落盘的路径是纯 ASCII**。

### 24.2 改法（= §5.1 / §10.2 ① 的修订）

- `FontMeta` 新增 **`id`**（= 配置/界面里那串，可含汉字）。**空 = 老库，回落「目录名就是 id」**⇒ 存量零迁移。
- 目录名与字体文件名一律 `ascii_slug(id, hint)` = `<hint>-<sha256(id)[..12]>`，字体文件统一叫 `font.<ext>`。
  只由 **id** 派生 ⇒ 重复导入不换目录、改显示名不换目录、两个 id 绝不撞同一个目录。
- `scan_dirs` / `resolve_dir_file` / `dir_for_id` / `remove_font` / `set_default` 全走 **id 索引**
  （先试“目录名就是 id”那条老路，再按元数据扫一遍）。
- `import_font_bytes`：**同 id 已存在就写回同一个目录**（否则老库里会出现两条同 id 的条目），
  并继承槽位角色（`license` / `source_url` / `emoji` / `is_default`；`builtin` 有意不继承）。
- `migrate_ascii_layouts`（在 `ensure_dir` 里跑；幂等、失败只跳过那一条）：补 `id` → 字体文件改名
  `font.<ext>` → 目录改名成 slug；**老平铺的「中文名 .ttf」也一并搬成目录布局**（它同样是非 ASCII 路径）。
  幂等判据就是“名字还是不是 ASCII / 有没有 id”，跑第二遍是空转；**ASCII 条目一个都不动**。
- `/vt-fonts/:name` 从“按文件名 join 库根”改成 **按 id 走 `resolve_all`**，`@font-face` 的 URL 也改用 id：
  ① 老写法在 v2 目录布局下**本来就会 404**（文件在子目录里）；② 每支都叫 `font.ttf` 之后，文件名根本不足以定位一支字体。
- Takumi 的 `font_aliases(path)` 改为**读同目录 `FONT.toml` 的 `id`**（没有元数据才回落目录名）——
  否则目录名变成 slug 之后，封面会认不出用户选的那支（§13 那次的症状原样复发）。

### 24.3 验收（都真跑过）

- 单测：`cjk_id_never_reaches_disk_as_a_path`、`legacy_non_ascii_dir_migrates_to_ascii_and_keeps_id`（含幂等）、
  `import_font_bytes_shares_the_same_rules_as_import_font`（新形状）、
  `font_aliases_prefer_meta_id_over_slug_dir`、`injects_font_face_for_id_indexed_slug_dir`。
- 真库迁移：把 `~/.yekitrim/fonts` 复制一份跑探针
  （`VT_FONT_MIGRATE_DIR=<库> cargo test -p yekitrim-core --lib migrate_probe -- --ignored --nocapture`）——
  `乐米波波体/乐米波波体.ttf` → `font-c0ac1d948217/font.ttf`，`resolve_all(root, "乐米波波体")` 仍命中；
  三支 ASCII 目录只补了 `id`，**没有被改名**。
- 端到端：迁移后的 ASCII 路径跑同一段 240 帧 = **3.96s、216 行 + 239 条全画上、exit 0**；
  原始中文路径做对照 = 37.1s、0 行 0 条、exit 1。
- 手工探针留在 `fonts::tests_v2::migrate_probe`（`#[ignore]`，与 `diagnose_sticker_cache` 同规矩）。
