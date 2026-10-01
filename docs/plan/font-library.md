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
| `crates/vtrim-polish/src/render/server.rs:205` | `SYSTEM_FONT_STACK = "'LXGW WenKai','Microsoft YaHei','Noto Sans SC',sans-serif"` |
| `crates/vtrim-render-core/src/types.rs:539` | 缺省字幕字体 = `"LXGW WenKai"` |
| `templates/index.html:19,746,772,1944`、`PolishPreview.vue:1303,1367` | 同一串栈（预览与出片必须同序 —— 顺序反过一次，见 `docs/vtrim-integration.md:233`） |

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
| 字体库目录 | `vtrim_core::path::font_dir()` → `<程序目录>/resources/fonts` | ✅ 有 |
| 扫描/解析/名称校验 | `vtrim_core::fonts::{scan, resolve_file, is_font_file_name, is_safe_id}`（`crates/vtrim-core/src/fonts.rs`） | ✅ 有（含单测：`LXGWWenKai-Regular.ttf` ✓、`思源宋体.OTF` ✓、`is_safe_id("LXGW WenKai (v1.0)")` ✓） |
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
| 中文正文字体 | **Noto Sans SC** / **Noto Serif SC**、**思源黑体/宋体**（Source Han） | OFL-1.1 | 覆盖全、体积大（10–16MB ⇒ 考虑**子集**或**按需下载**，与 ASR 模型 `vtrim download` 同套路） |
| 中文手写/标题 | **LXGW WenKai（霞鹜文楷）** | OFL-1.1（仓库已声明，`THIRD-PARTY-NOTICES.md:94`） | 就是现在默认栈第一位；自带它 = "预览与出片终于同源" |
| 中文无衬线（更窄） | **Sarasa Gothic（更纱黑体）** | OFL-1.1 | 需核对"保留名"与子集 |
| Emoji | **Noto Color Emoji** | OFL-1.1 | ⚠️ 彩色 emoji 的**嵌入**是另一层问题（CBDT 表），Takumi 单点里能画 ✓；浏览器 `@font-face` 也能 ✓ |
| 拉丁（可选） | MIT/CC0 的少数几支 | 待核 | 因为中文不能没有，**别只带拉丁** |

⚠️ 体积与入库两条红线：① 会进 `cargo xtask package` 的包 ② 每一支都要在 `THIRD-PARTY-NOTICES.md` 登记，
并被 `cargo xtask scan` 断言过（"字体目录里的每一支都在 notices 里"）。

---

## 5. 字体库管理设计（数据 + API + UI + 统一来源）

### 5.1 数据布局（**每支一个目录**，id = 目录名 = 稳定标识）

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

### 5.2 API（都加在 `crates/vtrim-server/src/api/polish.rs` + `routes.rs`）

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/polish/fonts` | 已有；**扩展**返回元数据（id/display/family/builtin/is_default/license/emoji/scripts） |
| POST | `/api/polish/fonts/import` | 导入（上传字节或给定本机路径）。**校验**：能解析、能取到 family、不是 `.ttc/.otc`（或自动拆包）；落盘 + 写 `FONT.toml` + 复制许可 |
| POST | `/api/polish/fonts/:id/default` | 设为默认（同时写回老工程的缺省？→ 只影响**新工程**与"缺省"读取，见 5.4） |
| DELETE | `/api/polish/fonts/:id` | 删除；`builtin=true` → **403**，理由写清"程序自带字体不可删除" |
| GET | `/api/polish/font?name=<id>` | 已有（预览取字节） |

⚠️ **路径安全**：`id` 一律过 `vtrim_core::fonts::is_safe_id` + 只允许**单段**、只允许库目录下；
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
建议这样，且把"默认"存成**程序级配置**（`<程序目录>/.vtrim/config.toml` 或字体库 `FONT.toml` 的 `is_default`）。

---

## 6. 落地顺序（建议）

1. **决定自带哪支、怎么带**（§4 + §7）；下载 → 放 `resources/fonts/<slug>/` + `FONT.toml` + `LICENSE.txt`；
   同步 `THIRD-PARTY-NOTICES.md`。
2. `vtrim_core::fonts` 加元数据层（读/写 `FONT.toml`、`builtin`/`is_default`、导入校验、`ttc` 处理），
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

1. **仓库里现成有诊断用例**：`crates/vtrim-core/src/fonts.rs:367` 的 `diagnose_font_family` ——
   ```bash
   cargo test -p vtrim-core --lib diagnose_font_family -- --ignored --nocapture
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
cargo check -p vtrim-server --all-targets   # exit 0
cargo test  -p vtrim-server                 # 144 passed / 0 failed
python3 scripts/check_module_map.py         # exit 0（35 模块 / 7 项）
python3 scripts/check_module_boundaries.py  # exit 0（15 规则 / 243 文件 / 0 违规）
python3 scripts/route_manifest.py           # exit 0
```

### 11.4 顺带修的一处**回退风险**

`scan_all` 里老布局（平铺文件）的 `display_name` 原计划回落成 `id`，那会让前端从"看到族名"
变成"看到文件名"——**已改成回落成解析出的族名**，与改动前前端看到的完全一致（不回归）。


---

## 10. 落地契约（P1-a 结论：读完 `crates/vtrim-core/AGENTS.md` + `src/fonts.rs` 后定）

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
| ① | **id：文件名去扩展名 → 目录名**（`resources/fonts/<slug>/`） | `crates/vtrim-core/AGENTS.md` 第 67 行"**id = 文件名去扩展名**"必须改；`resolve_file*` 要**同时支持两种布局**（老平铺文件继续可解析） |
| ② | `FontEntry` 扩元数据；**`family` 语义变严** = **字体内部族名**（唯一权威），另立 `display_name` 给人看 | 卡里"`family` 只是展示名"这句要改；§5.4 的三处来源只认 `family` |
| ③ | **自带字体**（LXGW WenKai Lite + Noto Color Emoji） | ⚠️ **与卡第 72 行「发行包不带任何字体（授权原因）」+ README「字体」一节直接冲突** ⇒ 同批改这两处，并写明依据：OFL-1.1 **允许嵌入/再分发**（§2/§8.3）+ 随附**发行包自带的 `OFL.txt` 原文** + `THIRD-PARTY-NOTICES.md` 登记 + **`cargo xtask scan` 断言"字体目录里每一支都在 notices 里"**。⚠️ **"Windows 系统字体不可分发"那条仍然成立** —— 那正是过去"不带任何字体"的原因 |
| ④ | `.ttc` 策略 | 卡里只论证了"Chrome 与 swash 读得动"，**漏了"浏览器 `@font-face` 不认字体集合"** —— 而"预览与实际对不上"很可能就是它。**第一版：导入时拒收 `.ttc/.otc` 并明确提示**（拆包留后续）；`FONT_EXTS` 是否收窄到 `ttf/otf` 要与此一致 |

### 10.3 分层纪律（别越界）

- **core 只做文件层**（读/写 `FONT.toml`、扫描、解析、导入落盘、删除、设默认），**不碰 HTTP**；
  四个端点全在 `vtrim-server`（`api/polish.rs` + `routes.rs`），同步 IO 走 `api::blocking`。
- `vtrim-core` **不得出现** `vtrim_polish`/`vtrim_service`/`vtrim_server`/`vtrim_cli`（门禁会红）。
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
| `crates/vtrim-server/src/api/polish.rs` | 导入**同时接受 `{dir,name}`**（服务端拼路径） |

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
cargo check -p vtrim-server --all-targets   # exit 0
cargo test  -p vtrim-server                 # 144 passed / 0 failed
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
- 别名怎么算（`font_aliases`）：约定文件名 `font.<ext>` ⇒ id = **目录名**（v2 布局）；
  否则 ⇒ id = **文件名去扩展名**（老平铺）。与 `fonts::scan_all` 同一条规则。

同时修掉这条路上**两个真 bug**（都是"v2 布局下静默失效"）：

1. `cover_font_files()` 只列**顶层文件** ⇒ v2 目录布局**一个字体都扫不到**，
   而"零字体"这条路上 `render_cover_png` 是**明确报错**的 → 症状是"静态封面直接出不来"。
   现改走 `fonts::scan_all` + `resolve_all` 反查（`FontEntry` 是元数据、不带路径）。
2. `font_face_css_in` 走的是 `resolve_file_at`（**只认老平铺**）⇒ v2 布局下**一条 @font-face 都不注入**
   →"选了库里的字体、出片没换"，而预览另有 `FontFace` 看起来还是对的，只有成片不对。
   现改走 `resolve_all`。

### 13.3 新增能力（给后续阶段用）

`vtrim_core::fonts::family_for(id)` / `family_for_at(root, id)`：**id → 该用的 CSS 族名**。
库里认这个 id ⇒ 返回 `FONT.toml` 的 `family`（内部族名，唯一权威）；**库外原样返回**
（老工程 `font = "LXGW WenKai"` 的系统族名行为一字不变）。三处消费方要"按族名做事"时都该走它。

### 13.4 验收（可复跑）

```bash
cargo test -p vtrim-core --lib fonts                                   # 12 passed
cargo test -p vtrim-polish --lib                                       # 213 passed
cargo test -p vtrim-polish --features cover-takumi                     # 含 takumi 单测 + e2e
cargo check --workspace --all-targets                                  # 0 warnings
cargo check -p vtrim-polish --features cover-takumi --all-targets      # 0 warnings
python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py
```

**E2E 钉子** `crates/vtrim-polish/tests/takumi_font_id.rs`（真渲染、真 Takumi）：
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
可能与 `FONT.toml` 不一致）。现在改走 `vtrim_core::fonts::family_for_at(&root, &f.id)`，
并补齐 v2 字段（`display_name` / `builtin` / `is_default` / `license` / `scripts` / `emoji`）。

顺带把 `scan()` 换成 `scan_all(&root)`：前者不认 v2 目录布局。

### 14.3 复查口径

```bash
cargo check --workspace --all-targets                              # 0 warnings（这次就是它抓到的）
cargo check -p vtrim-polish --features cover-takumi --all-targets  # 0 warnings
cargo test -p vtrim-server                                         # 145 passed
cargo test -p vtrim-polish --lib                                   # 213 passed
cargo test -p vtrim-core --lib fonts                               # 12 passed
cargo test -p vtrim-service                                        # 125 passed
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

- **注册表 + marker 的"装好没有"判定**：抄 `vtrim-core/src/download.rs` 的 `MODEL_REGISTRY`
  形状（`(键, 目录名, 文件名, marker)`）与 `installed` 语义 —— **半下载的目录不算装好**。
- **下载本身**：同文件的 `download_file(url, part_path)`（先落 `.part` 再改名）与
  进度/失败口径。

### 15.3 落点

| 要做的 | 落点 |
|---|---|
| 字体注册表（上游 URL + 校验和 + 许可 + 显示名） | `vtrim-core/src/fonts.rs`（与 `scan_all` 同一模块，字体的事只在这里） |
| `ensure_builtin_fonts()` / `download_builtin_fonts()` | 同上；幂等，已装好就跳过 |
| 下载进度与日志 | 走调用方（CLI 打印 / 任务日志），本模块只回结构化结果 |
| CLI 入口 | `vtrim fonts list` / `vtrim fonts download`（`crates/vtrim-cli`） |
| 界面入口 | 字体库页（`#/fonts`）加一条"下载自带字体"按钮 + 状态；API `POST /api/polish/fonts/builtin`（**仅 admin**） |
| 许可 | `LICENSE.txt` 随字体一起落盘（原样 OFL）；`THIRD-PARTY-NOTICES.md` 登记 |
| 发行包说明 | `resources/fonts/放这里.txt` 与 README「字体」改为"自带字体首次使用时下载" |

### 15.4 与「自带不可删除」的关系

`builtin = true` 那条分支（`remove_font` 拒绝删除）**现在才有真实实例**：
下载来的字体写 `builtin = true`，删它由界面置灰 + 服务端 403 两处共同挡住。
⚠️ 但**下载 ≠ 已装**：没下过的机器上 `scan_all` 里没有它，所以
"自带字体"在**安装前后**都必须能被界面区分成"未安装（可下载）"与"已安装（不可删）"。

### 15.5 验收

- `cargo test -p vtrim-core --lib fonts`：注册表形状、marker 判定、幂等（已装好的目录不重复下载）。
- 真下载一次（**联网**）验证 URL 可用 + 落盘布局 + `family` 能读出来；校验和必须对得上。
- 两条门禁 + `cargo check --workspace --all-targets`（0 warnings）。
- ⚠️ **不把字体二进制提交进 git**（这是本条决策的重点），只提交清单。


---

## 16. P1-f 落地记录（自带字体「首次使用下载」，2026-10-01）

§15 是方案，这里是**做完了的账**：做了什么、踩到什么、怎么验的。

### 16.1 清单（唯一真源：`vtrim_core::fonts::BUILTIN_FONTS`）

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
| CLI | `vtrim fonts list` / `vtrim fonts download [--id X]` | 本机 |
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
cargo test -p vtrim-core --lib fonts          # 14 passed
cargo test -p vtrim-polish --lib              # 214 passed
cargo test -p vtrim-server                    # 145 passed（含新增 builtin 路由的 admin 门禁）
cargo check --workspace --all-targets         # 0 warnings
cargo check -p vtrim-polish --features cover-takumi --all-targets   # 0 warnings
python3 scripts/check_module_map.py           # 35 模块 / 7 项
python3 scripts/check_module_boundaries.py    # 15 规则 / 244 文件 / 0 违规
```

**联网真跑**（不是 mock）：
1. `vtrim fonts list` → 4 支（2 支自带未安装 + 2 支老布局），未安装的标「未安装（可下载）」。
2. `vtrim fonts download --id NotoColorEmoji` → 3.6 秒装好，落盘形状与 §16.2 一致。
3. `vtrim fonts download --id LXGWWenKaiLite-Regular` → 3.0 秒装好。
4. 再跑一次 `download` → **`downloaded = []`，271.9µs**（幂等成立，没重复下 25 MB）。
5. 校验和复核：`FONT.toml` 里的 sha256 **与清单逐字相同**；`LICENSE.txt` 大小
   5176 / 4386 B，与上游 `OFL.txt` 一致。
6. 未知 id → `不是自带字体（自带清单里没有它）: xxx`（400 侧同文案）。

### 16.7 ⚠️ 测试环境陷阱（下次别踩）

`vtrim_core::path::font_dir()` 按**可执行文件所在目录**算，而 cargo 的集成测试二进制跑在
`target/<profile>/deps/` 下 ⇒ 测试看到的库是 **`deps/resources/fonts/`**，与 `vtrim` 主程序
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
做法抄 `scripts/vtrim-compare.mjs` 的 `--capture-golden/--golden` 先例（同类问题已有解法）。

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
| `crates/vtrim-polish/src/render/chrome.rs` | **整文件**（328 行，`headless_chrome::{Browser,LaunchOptions,Tab}`） |
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

`vtrim-core` 的 `Config.chrome: ChromeConfig`（+ `[chrome]` 模板段 + `merge()` 分支 +
`SettingsView.vue` 的「Chrome 渲染」输入框）在删掉出片/封面两条链路之后**已经没人读了** ——
`tool_probe.rs` 里那个 Chrome 探测函数也随任务一起没了，只剩它自己跟自己的 `merge()`。
留一个"Chrome 可执行文件路径"输入框会让人以为**装了 Chrome 就能用上**，实际它什么也不影响。
⇒ 四层一起删（字段 / 结构体+Default / 模板段+merge / 前端表单与 hydrate）。

### 19.4 验收（删 chrome 之后的完整一轮）

```
cargo check --workspace --all-targets   → 0 warnings
cargo test -p vtrim-polish --lib        → 214 passed; 0 failed
cargo test -p vtrim-server              → 145 passed; 0 failed
cargo test -p vtrim-core --lib          → 177 passed; 0 failed
cargo test -p vtrim-service             → 125 passed; 0 failed
check-cover-takumi --self-test / --mutate → 3 项通过 / 判据能红也能绿
check_module_map.py / check_module_boundaries.py → 35 模块 / 15 规则 0 违规
cargo tree -p vtrim-polish | grep -i chrome → 空
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
   同一台机器上 `vtrim polish render` **单遍**跑得好好的。⇒ 与本次改动无关。

> 教训：**探针失败先看它的锚点命中几次**。"0 次"是静默空转，"1 次"才是真在比；
> 而"看起来像回归"的失败，也可能只是这台机器的显存不够。
