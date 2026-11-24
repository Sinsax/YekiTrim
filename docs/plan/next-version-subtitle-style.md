# 下一个小版本：项目设置 · 字幕样式

> 立档：2026-10。**这是"下一个版本"的施工单**，目标是把字幕样式做成**项目设置里可调**。
> 用户原话：*"项目设置那里需要增加一个字幕的样式可设置，比如字幕颜色，描边颜色，字幕位置(垂直位置)，字幕大小。"*

---

## 0. ⚠️ 先说最重要的发现：**字段早就有了，缺的是"界面"和"两处接线"**

我核了全链路。**配置层（`SubtitleStyle`）早就定义了这 6 个字段**
（`crates/yekitrim-render-core/src/types.rs:501`）：

| 字段 | 默认值 | 转译器**读了吗** | WebUI **能改吗** |
|---|---|---|---|
| `font` | （字体栈） | ✅ 读了 | ✅ **能改**（`PolishView.vue:164` 字体下拉） |
| **`size`** | `72` | ✅ 读了（`subStyle.size`） | ❌ **不能改** |
| **`color`** | `#dcbda0` | ✅ 读了（`subStyle.color`） | ❌ **不能改** |
| **`stroke`** | `12px #403c3b` | ✅ 读了（`parseStroke`） | ❌ **不能改** |
| **`position`** | `bottom` | ❌ **没读**（`subStyle.position` 命中 0 次） | ❌ 不能改 |
| `highlight_color` | `#f56b41` | ❌ **没读**（`subStyle.highlight` 命中 0 次） | ⚠️ 仅**封面设计器**读它 |

### ⇒ 所以工作量是三类，不是一类

| 类 | 字段 | 要做什么 |
|---|---|---|
| **A. 只缺界面** | `size` / `color` / `stroke` | **只需在设置页加控件**（转译器已读，改完就生效） |
| **B. 缺界面 + 缺接线** | `position`（垂直位置） | 加控件 **+ 转译器要开始读它**（今天**完全没读**） |
| **C. 不在本次范围** | `highlight_color` | 转译器不读；它影响卡拉OK/高亮，**依赖底座 `highlight_color`**（见 §6） |

> 📌 **这是本任务最省力的地方**：**A 类三个字段是"零后端改动"** ——
> 只要把控件做出来、写进 `polish.toml`，**现有出片链路立刻生效**。

---

## 1. 目标

在**项目设置**里增加一块「字幕样式」，可调四项（用户点名的）：

| # | 项 | 控件 | 落到哪 |
|---|---|---|---|
| 1 | **字幕颜色** | 取色器 | `[style.subtitle] color` |
| 2 | **描边颜色** | 取色器（+ 宽度） | `[style.subtitle] stroke`（**格式 `"12px #403c3b"`** —— 宽度与颜色在**同一个字符串**里） |
| 3 | **字幕位置（垂直）** | 三选一：上 / 中 / 下 | `[style.subtitle] position` |
| 4 | **字幕大小** | 数字输入（px） | `[style.subtitle] size` |

### 1.1 ⚠️ 三个必须注意的格式细节（都已在代码里核实）

| 细节 | 说明 |
|---|---|
| **`stroke` 是"一个字符串装两个值"** | 格式 `"12px #403c3b"`，转译器用 `parseStroke(subStyle.stroke, 12, '#403c3b')` 解析（缺省回退 12px / 深灰）。**UI 要么拆成两个控件、要么拼串**，别写坏格式 |
| **⚠️ CSS 描边 ≠ FFmpeg 描边** | 转译器注释（`:3652`）记着：**CSS 的 `-webkit-text-stroke` 是居中（里外各半），FFmpeg `borderw` 全在外侧 ⇒ 要折半**。实测"直接填 12 会让描边粗一倍"。**UI 上的数值口径要写清楚是哪一个** |
| **`position` 今天没接线** | 转译器**根本没读** `position`。而且**字幕落点是算出来的**：`bottom_margin = (120 − 行盒/2) / 高度`（注释记着"直接填 120 会让字幕整体高约 54px"）。⇒ **加"上/中/下"要做的是"改这个落点计算"，不是塞个字段** |

---

## 2. 范围界定（做什么 / 不做什么）

### ✅ 本次做

1. **`size` / `color` / `stroke`（颜色+宽度）的 UI 控件** —— 零后端改动；
2. **`position`（上/中/下）的 UI + 转译器接线**；
3. **预览即时生效**（`engine.js` 已有 `idx.subStyle`，读的就是这几个字段 ⇒ 大概率自动跟随，**但要验证**）；
4. **落盘 `polish.toml`**，老工程零迁移（`serde(default)` 已有）。

### ❌ 本次不做（明确排除）

| 项 | 原因 |
|---|---|
| `highlight_color` | 转译器不读，且**依赖底座 `highlight_color`**（见 §6） |
| 字体（`font`） | **已经有了**（`PolishView.vue:164`），不重复做 |
| 描边**渐变/阴影/圆角** | 超出范围 |
| **逐条字幕**的样式覆盖 | 那是 `SubtitleEditor` 的事，本次只做**全局样式** |

---

## 3. 落点：UI 放哪

### 3.1 现状（已核实）

字幕相关 UI **已经在** `webui/src/views/PolishView.vue` 的**「字幕 / 弹幕」泳道组**里
（`:161–164`，`lane-group` for `subtitles`），目前**只有"字幕字体"一个下拉**。

⇒ **本次应该把新控件加在同一个泳道组里**（不是新开页面）——
因为用户说的"项目设置"在本仓的映射就是**精修页的泳道设置区**。

### 3.2 ⚠️ 一个需要你确认的取舍

| 方案 | 说明 |
|---|---|
| **A. 加在现有「字幕/弹幕」泳道组内**（推荐） | 与字体下拉挨着，用户找得到；改动集中在一个文件 |
| **B. 新开一个"样式"面板** | 更整齐，但要新建设置结构 |

⇒ **建议 A**（最小改动、语义正确）。

---

## 4. 触点数（施工前的清单）

| # | 位置 | 动作 | 类 |
|---|---|---|---|
| 1 | `PolishView.vue` 泳道组（`:161`） | 加 4 组控件 + 绑到 `cfg.style.subtitle.*` | UI |
| 2 | `PolishView.vue` 的保存函数（`:621` 附近，参照 `onSubtitleFontChange`） | 写 `polish.toml`（**走 `update_polish_toml`**） | UI |
| 3 | **转译器** `tools/polish-to-dhampir.mjs`（`:3608–3620`） | **读 `position`，改落点计算** | 接线 |
| 4 | `engine.js`（`idx.subStyle`，`:319`） | 验证预览跟随（可能已自动） | 预览 |
| 5 | 模板 `templates/index.html`（字幕 CSS / 落点） | **参照侧要对上**（⚠️ 改了要**重建二进制**） | 参照 |
| 6 | `webui/scripts/check-*.mjs` | 若有字幕样式的对拍，**要加断言** | 门禁 |

⚠️ **第 3 与第 5 必须一致** —— 本仓的规矩是"参照（模板）与转译器要对齐"，
不然就是**预览看到的不等于成片**（本仓已有 4 个这类 bug）。

---

## 5. 验收判据

```bash
# 1. 前端改了必须重建 dist（它是编译期输入）
npm --prefix webui run build

# 2. 契约对拍
node webui/scripts/check-overlay.mjs
node webui/scripts/check-handheld.mjs
# （若新增字幕样式对拍，一并加进来）

# 3. 转译器守卫 + 变异自证
node scripts/check-yekitrim-translator.mjs --mutate

# 4. DROP/CLAMP 基线
node scripts/check-dhampir-drop-list.mjs

# 5. ⚠️ 改了 templates/index.html ⇒ 必须重建二进制
cargo build -p yekitrim-cli

# 6. 结构门禁
python3 scripts/check_module_map.py && python3 scripts/check_module_boundaries.py
```

### 本任务特有判据

| # | 判据 |
|---|---|
| 1 | **改 `size`/`color`/`stroke` 后，出片真的变了**（因为转译器已读 ⇒ 属于"验证接线"，不是"接新线"） |
| 2 | **改 `position` 后，出片真的变了**（这是**新接线**，最可能出错的一环） |
| 3 | **预览 = 成片**（同一帧对拍，**不只看平均值**） |
| 4 | **老工程零迁移**：没动过样式的工程，出片逐字节不变（R4） |
| 5 | **描边宽度口径**：UI 显示的数值与成片的实际粗细**关系明确且已记录**（CSS 居中 vs FFmpeg 外侧，**差 2 倍**） |

---

## 6. 与"总方案"的关系（本任务在全局里的位置）

| 关联项 | 关系 |
|---|---|
| **总方案 §5 `elements` 元素系统** | ⚠️ **不同东西**：本任务是**全局字幕样式**；`elements` 是**新增"文字元素"这一类零件**。两者**共用 `SubtitleStyle` 但互不阻塞** |
| **总方案 §25 底座文字层三字段** | ⚠️ **本任务会碰到同一片区域**：`stroke` 从 bool 升级为可调宽度。**本任务的 UI 可以先把宽度做出来**，但**成片侧要等底座**（今天底座只有 `outline: bool`） |
| **`elements-spec.md` §3** | 记录的就是这个缺口。**本任务与它是"同一片区域的两件事"** —— 建议**先做本任务**（小、立刻见效），`elements` 随后 |

> ⚠️ **一个重要的现实提醒**：`stroke` 的**宽度**在成片侧**今天表达不了**
> （底座只有 `outline: bool`，转译器是把 CSS 宽度折半硬塞进 `borderw`）。
> ⇒ **UI 可以做"宽度"控件，但要如实标注"当前仅影响预览/近似"**，或**先只做颜色**。
> **这个取舍需要你定**（见 §7）。

---

## 7. ✅ 已拍板（用户已选）

| # | 问题 | ✅ 决定 |
|---|---|---|
| 1 | **描边宽度**要不要做？ | **做**（颜色 + 宽度都做），**宽度在 UI 上标注"近似"** |
| 2 | **`position`** 三档还是连续值？ | **连续数值（距底百分比，如 12%）** |
| 3 | UI 放现有泳道组内还是新开面板 | **现有泳道组内**（§3.2，与字体下拉挨着） |
| 4 | **`highlight_color`** 是否顺手做 | ❌ **不做**（转译器不读，且依赖底座） |

### 7.1 ⚠️ 落点：我核实了两处"真相"，`position` 的语义必须现在定死

字幕垂直位置今天**有两个写死的 120 px**，而且**单位不同**：

| 处 | 代码 | 口径 |
|---|---|---|
| **出片（权威）** | `templates/index.html:2040` → `cy = CH - 120 * CKH` | **文字中心**距底 **120 × 高比** |
| **转译器** | `bottom_margin = (120 − 行盒/2) / 高度` | **行盒底边**距底 |
| CSS（仅预览容器） | `#captions { bottom:80px }` | 容器位置，**不是文字落点** |

⇒ **`position` 必须定义成"文字带底边距画布底部的百分比"**（与转译器的 `bottom_margin` 同口径），
**默认 12%** —— 因为 `120/1080 = 11.11%`，取整为 **12%** 时与今天**几乎一致**
（⚠️ 但**不完全一致**：这正是"存量零回归"要靠"缺省不写字段"来保证的原因，见 §7.2）。

**映射公式**（统一到转译器的口径）：
```
pct      = position_pct            // 新字段，0~50，默认 12（%）
bottom_px = pct/100 * 高度         // 行盒底边距画布底
bottom_margin = bottom_px / 高度   // 转译器要的比例值 = pct/100
```

### 7.2 ⚠️ 存量零回归的铁律

**默认值不让"没写过该字段的老工程"改变出片**：

| 字段 | 老工程（缺省）行为 |
|---|---|
| `position_pct` | **不写 = 走原来那条写死的 `120 * CKH`**，**不走百分比换算** |
| `size`/`color`/`stroke` | 缺省值**与今天逐字节相同**（本来就有默认） |

⇒ **实现上必须是**：`position_pct` 为 `Option<f64>` / `serde(default)` 且
**只有显式写了才启用百分比分支**。这样老工程逐字节不变（R4）。

### 7.3 描边"近似"的口径（要如实写进 UI 与文档）

底座今天**只有 `outline: bool`**（`polish-to-dhampir.mjs:3613` 明记）。转译器现在是把
CSS 宽度**折半**后塞进 ffmpeg 的 `borderw`（注释记着"直接填 12 会让描边粗一倍"）。

⇒ **UI 上的"描边宽度"当前是近似的**：
- **预览/模板侧**（canvas）**精确** —— 那里是真正的 CSS 描边语义；
- **成片侧**受底座限制，**可能不随宽度线性变化**。

⇒ **UI 文案必须标注"近似"**，并**指向底座字段需求（总方案 §25）**。

---

## 附：证据等级

| 等级 | 内容 |
|---|---|
| **仓库内部（已核实）** | `SubtitleStyle` 六字段与默认值（`types.rs:501–561`）；**转译器只读 `font`/`size`/`color`/`stroke`，`position` 与 `highlight_color` 命中 0 次**（grep）；`parseStroke` 的格式与折半理由（`:3620`、`:3652`）；字幕落点公式 `bottom_margin=(120−行盒/2)/高度`（`:3633`）；WebUI 只有字体下拉（`PolishView.vue:164`）；`engine.js` 读 `idx.subStyle`（`:319`）；底座只有 `outline: bool`（`:3613`） |
| ⚠️ **待验证** | `engine.js` 的预览是否**自动**跟随 size/color/stroke/position（大概率是，但**必须实测**，不能假设） |

---

# ✅ 落地记录（2026-10 已完成）

> 本节是**实现后**补写的。上面各节是施工前的判断，**有几处被实施证伪**，在此更正 —— 别只读上面。

## 一、最终交付（6 个文件）

| # | 文件 | 做了什么 |
|---|---|---|
| 1 | `crates/yekitrim-render-core/src/types.rs` | 加 `position_pct: Option<f64>`（`serde(default, skip_serializing_if)`，`Default = None`） |
| 2 | `tools/polish-to-dhampir.mjs` | 落点分流：有值 → `bottom_margin = pct/100`；无值 → 原式（逐字节不变） |
| 3 | `crates/yekitrim-polish/templates/index.html` | 出片落点分流（**循环内**按行数修正，见下 §三） |
| 4 | `webui/src/components/PolishPreview.vue` | 预览 `--cap-bottom` 分流 + 补偿（见下 §三） |
| 5 | `webui/src/views/PolishView.vue` | 「字幕 / 弹幕」泳道组内 5 项控件 + 落盘（`ui-subtitle` 完成） |
| 6 | `crates/yekitrim-server/src/api/polish/` | **服务端白名单四键** + 回归测试（见下 §二） |

## 二、⚠️ 施工前漏掉的一环：**HTTP 白名单**（本节是原文档最大的缺口）

原 §4「触点数」只列了 UI / 转译器 / 预览 / 模板 / 门禁 **五处**，**漏了服务端**。
而 `polish_project_put_locked`（`crates/yekitrim-server/src/api/polish/`）是**逐字段白名单**——
**没列进去的键，PUT 照样回 `{"ok":true}`，但磁盘一个字都不动**（该函数自己的注释就写着这句）。

⇒ **这是本版本最危险的一处**：四项控件的后端口子曾经**整条缺失**，症状是
"toast 说已保存 → 控件回弹到原值 → 全程无报错"。**已补**：
`subtitle_color` / `subtitle_stroke` / `subtitle_size` / `subtitle_position_pct`（含 `null` → `None`）。

> 📌 **下一个小版本开工时先把这条抄进施工单**：**任何新配置字段，先查"HTTP 层有没有口子"**。
> 判据 = 在 `api/polish/` 里 grep 字段名，**命中 0 次就是要补白名单**。

## 三、⚠️ 落点公式：原 §7.1 的推导**只对单行成立**

原 §7.1 把 `position_pct` 定义为"**文字带底边**距画布底"，方向正确，但**换算写漏了行数**：

- **出片（canvas）**：`cy` 是**整个文字块的中心**，块里 **N 行**绕它对称铺开
  （`y0 = cy - (N-1)*lh/2`）⇒ 正确式子是
  **`cyEff = CH - (pct/100)*CH - N*lh/2`**（`lh = 字号*1.5`）。
  **第一版写成 `+ 行盒/2`（只有 N=1 成立）**：2 行偏 **108px**、3 行偏 **216px**，且**不报错**。
- **预览（DOM）**：容器 `.captions` 是"底边锚定 + 内容向上长"，**多行时容器变高、底边不动**
  ⇒ 容器口径**天然等于**"文字带底边"，**不需要 N 修正**；
  但要补 **`14px`（`.cap` 的 `padding`）+ `0.25 × 字号`（`line-height:1.5` 的半行距）**
  ⇒ **`--cap-bottom: calc(pct% + 14px + 0.25*size)`**。
  ⚠️ 偏差**随字号变化**（size 48→120 时 26px→44px），**不能写死 14**。

**三条腿全组合实测**（pct ∈ {0,12,25,50} × N ∈ {1,2,3} × size ∈ {48,72,120}）：**最大值差 0.0000**。

## 四、遗留问题（**有意不做**，别当漏了）

| # | 项 | 为什么不修 |
|---|---|---|
| 1 | **缺省分支多行时，模板/转译器差 54/108px** | **改动前就有的老伤**（`CH-120*CKH` 不在本次 diff 内）。修它会破 R4（老工程落点会变）⇒ **记入 master plan 当独立项**；下一版若统一到"距底"口径，**要配一次迁移** |
| 2 | **模板内嵌 DOM 预览不跟随 `position_pct`**（`index.html:372` 写死 `80*CKH`） | 出片走 canvas `renderFrame`，**成片是对的**，影响面仅模板内嵌预览 |
| 3 | `highlight_color` | 转译器不读、依赖底座，见原 §6 |

## 五、⚠️ 未覆盖（**用户侧验收项**）

**没有真跑过一次带字幕的完整出片。** 已验到的边界是：
转译器产物正确 + 三条腿数学恒等 + 各门禁绿 **+ R4 逐字节零回归**。

**最后一段"底座实际把 `bottom_margin` 画在哪"没验**（需要真工程 + 底座 Dhampir）。
⇒ **建议合并后补一次**：真工程里把 `position_pct` 设成 **12 / 25 / 50** 各出一帧，**与预览同帧对拍**。

## 六、验收证据（全绿）

```
cargo check --workspace --all-targets     0 warnings
cargo test -p yekitrim-server --lib       183 passed
cargo test -p yekitrim-polish --lib project   67 passed
cargo test -p yekitrim-render-core --lib  30 passed
npm --prefix webui run build              指纹 e4732717ac8de321
webui/scripts/check-*.mjs（9 个）          9/9
scripts/check-preview-audio.mjs            OK（⚠️ 它在 scripts/，不在 webui/scripts/）
check-yekitrim-translator.mjs [+ --mutate] OK + 42/42 变异全打红
check-dhampir-drop-list.mjs               1 DROP / 0 CLAMP（与基线一致）
check_module_map.py / boundaries.py       35 模块 / 17 判据 / 395 文件 / 0 违规
R4（对拍 HEAD 版转译器，归一化 uri）        逐字节相同 sha256 58f9f6b7c85d37e5
```

⚠️ **R4 的判据要用"对拍 HEAD 版转译器"**，不要用 `target/` 下的基线文件 ——
产物里嵌了绝对路径 `uri`，直接 sha256 必假红（实测：`33ce1cda…` vs `e7b5156d…`，`diff` 只差那行路径）。
