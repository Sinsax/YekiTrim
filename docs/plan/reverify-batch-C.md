# 重验 C —— §18/19 镜头 + §6/7/8 叙事 + §9/10/11/23（三列核实）

> 任务 task-22 ｜ **只读审计，未改任何产品代码** ｜ 2026-10-11
>
> 起因：`docs/plan/polish-upgrade-consolidated.md` 的 25 项总表**只用了单列 ✅**，
> 判据是"代码在 + 有消费者"。这个判据会掩盖「**有消费者、没有生产者**」。
> 本文件按**三列**重验 + 加**第四列（用户真正要的）：能不能在成片/界面上看见它**。

## 四列口径

| 列 | 含义 |
|---|---|
| **① 代码** | 实现存在，给文件:行号 |
| **② 后端接线** | **生产者是谁、消费者是谁** —— 特别查"有没有生产者" |
| **③ 前端** | WebUI 控件/展示 + 预览（`webui/src/polish/`）支持，给 `grep` 命中数 |
| **④ 看得见吗** | **能看见 / 要特定条件 / 看不见** ← 用户真正要的那一列 |

---

## 总表（先看这个）

| 项 | ① 代码 | ② 后端接线 | ③ 前端 | ④ 看得见吗 | 断在哪 |
|---|---|---|---|---|---|
| **§19 荷兰角** | ✅ | ✅ 全链通 | ⚠️ 只有预览渲染，**无控件** | **要特定条件** | 提示词教了、守卫齐全、**真出片实测歪了 4.5°**；但**编辑器里没有 tilt 输入框** ⇒ 只能靠 LLM 写 |
| **§18-A 特写景别** | ✅ | ❌ **无生产者** | ⚠️ 有倍率滑杆，**景别下拉里没有「特写」** | **看不见** | 提示词 `segment.rs:83` 只有 `远/中/近` ⇒ LLM 永远写不出；编辑器下拉同样没有 ⇒ 人也选不了 |
| **§18-C 运镜档位配置** | ✅ | ✅ | ✅ 4 条滑杆 | **能看见** | 无 |
| **§6 爆点判据** | ✅ | ⚠️ **算了就扔** | ❌ 0 命中 | **看不见** | 多信号探测**真跑了**，但 `punches` 出块即弃，只打一行日志 |
| **§7 静默分两类** | ✅ | ❌ **零消费者** | ❌ 0 命中 | **看不见** | `split_silence_by_kind` 全仓**只有自己的单测在调** |
| **§8 `role` 叙事角色** | ✅ | ⚠️ 只有提示词 | ❌ 0 命中（10 处 role 全是 auth） | **看不见** | 提示词教了、守卫跑了，但 `role` **不改任何输出**，界面也不显示 |
| **§9 多版本成片** | ✅ | ✅ CLI 可达 | ❌ 0 命中 | **看不见**（WebUI） | 只有 CLI `--versions`，WebUI 无入口 |
| **§10 情绪轨道** | ⚠️ 只有 `fx` 档 | ⚠️ | ⚠️ 1 处（标着「情绪」） | **要特定条件** | **没有独立情绪轨道**；`fx` 下拉借用了「情绪」这个标签 |
| **§11 BGM** | ✅ | ⚠️ 只在 pipeline | ❌ **0 命中** | **看不见** | 后端全通，但 server 层 **0 命中** ⇒ 无 API、无 CLI 开关，只能手改 `polish.toml` |
| **§23 节奏求解器** | ✅ | ⚠️ **只报告不落盘** | ❌ 0 命中 | **看不见** | 设计如此（保守决策）；扫出的空档只进日志 |

---

## §19 荷兰角 —— **用户点名说"没实现"，实测：实现了，而且成片真的歪**

### 这是本批最重要的一项，结论与用户体感**相反**

任务书要求核实"这是设计如此、刻意很小，还是真没实现"。**答案是前者，而且比"设计如此"更强 ——
它真的出片了。** 我没有为了迎合体感而下"没实现"的结论，用**真实出片 + 逐像素测量**证明。

### ① 代码

| 环节 | 文件:行号 | 函数/常量 |
|---|---|---|
| 数据模型 | `crates/yekitrim-render-core/src/types.rs`（`Event::Camera` 的 `tilt` 字段） | — |
| 提示词 | `crates/yekitrim-polish/src/llm/segment.rs:148-159` | 教 `tilt`，含 `{"tilt":4.5}` 例子 |
| 意图推断 | `crates/yekitrim-polish/src/llm/segment.rs:283-285` | `obj.contains_key("tilt")` ⇒ 认成 camera |
| **守卫** | `crates/yekitrim-polish/src/process/fx.rs:420-469` | `cap_dutch_angle` |
| 上限 | `fx.rs:388` / `fx.rs:393` | `MAX_DUTCH_ANGLE = 6.0` / `MAX_DUTCH_SHOTS = 2` |
| 倍率 | `types.rs:557-574` | `zoom_closeup`（特写 1.8） |
| **出片转译** | `tools/polish-to-dhampir.mjs:4324-4359`（状态机）、`:4539-4556`（写键 + NOTE） | `rotation` 逐帧 |
| 出片模板 | `crates/yekitrim-polish/templates/index.html:950-951`、`:1496-1531`、`:2015` | canvas `cam.tilt` |

### ② 后端接线：全链通

```
LLM 提示词(segment.rs:148) → 事件推断(:285) → 守卫 fx.rs:420
  → 转译器 polish-to-dhampir.mjs:4539 写 rotation 关键帧
  → 底座 Dhampir 渲染 → 成片
```

- **生产者**：LLM 提示词（`segment.rs:148`）。**存在、且是活的。**
- **消费者**：转译器 `:4539` `placeKeyframe(sourceFrame, 'rotation', rot, 'linear')` → 底座。
- **守卫调用点**（生产链，非单测）：`crates/yekitrim-polish/src/process/mod.rs:275`。

### ③ 前端：`grep -rn` 命中数

| 关键词 | 命中数 | 文件:行号 |
|---|---|---|
| `tilt` | **6** | `PolishPreview.vue:2239`；`engine.js:497,506,527,528,538` |
| `荷兰角` | **4** | `PolishPreview.vue:2236,2237`（注释）；`engine.js:494,526`（注释） |

⚠️ **6 处 `tilt` 全在预览/渲染求值器里，4 处「荷兰角」全是注释。** TimelineEditor 的 camera 字段编辑区
（`webui/src/components/TimelineEditor.vue:257-287`）有 **景别 / 硬切 / 推进速度 / 手持摇摆 / 聚焦**，
**没有 tilt 输入框**。

### ④ 看得见吗：**要特定条件 —— 成片里真看得见，但只能靠 LLM 写**

#### 真实出片实测（决定性证据）

我用**孪生对照**做的：同一个工程、同一段素材，只改 `tilt = 4.5` → `tilt = 0.0`，各出一版成片。

```
tilt-clean: ✓ 成片时长断言通过：画面 720 帧 / 音轨 577536 样本 / 派生 720 帧
            FINISHED: .workbuddy\tilt-clean\export.mp4 (36.7MB, 27s)
tilt-off:   FINISHED: .workbuddy\tilt-off\export.mp4 (36.5MB, 27s)
```

转译器报告（tilt-clean）：

```
[camera.tilt] 荷兰角（§19）逐帧写进视频层的 rotation（479 帧）——
              与手持摇摆的 rot **相加**（两者是不同尺度上的倾斜，同时开时都在）
```

转译产物里 `main-0` 层确实有 rotation 关键帧，峰值 **4.5000 度**：

```
前5个: [{"f":61,"v":0.2465},{"f":62,"v":0.4861},{"f":63,"v":0.7188},{"f":64,"v":0.9444},{"f":65,"v":1.1632}]
后3个: [{"f":117,"v":4.5},{"f":118,"v":4.5},{"f":119,"v":4.5}]
最大 |rotation| = 4.5000 度
```

**逐像素验证旋转角**（取 t=5.0s 的一帧，把 tilt-on 帧旋转 `d` 度后与 tilt-off 帧求相关）：

| 旋转 d | 0.0° | −3.0° | −4.0° | **−4.5°** | **−5.0°** | −6.0° | −8.0° |
|---|---|---|---|---|---|---|---|
| corr | 0.6841 | 0.7967 | 0.8201 | **0.8295** | **0.8323** | 0.8236 | 0.7489 |

相关在 **−4.5°~−5.0° 处取到峰值**（0.83），0° 只有 0.68 ⇒ 成片相对参照被转了约 **+4.5~5°**，
与配置的 `tilt = 4.5` 吻合。（正方向单调下降：+3.0° = 0.5737、+6.0° = 0.4803，符号无歧义。）

并排对照图（左 `tilt=0` / 右 `tilt=4.5`，同一时刻 t=5.0s）：黑板顶边与窗框在右图明显顺时针歪了。

![tilt 开关对照](.workbuddy/tilt-on-off-side-by-side.png)

#### 三道守卫的实际门槛（这是"为什么体感小"的真实答案）

| 守卫 | 门槛 | 实际咬合概率 |
|---|---|---|
| 夹角度 | ±6°（规格"防滥用值"） | **很少咬** —— 提示词已经教 LLM 写 ≤6° |
| 同段有画中画就剔除 | 同段存在 `zoom == "远"` | **很少咬** —— 默认档是「中」(1.3)，`远` 在工程里罕见 |
| **全片最多 2 条** | `MAX_DUTCH_SHOTS = 2` | ⚠️ **这条才是真正的限制** |

⚠️ **`±6°` 不是"刻意做小"，是有几何依据的**（`fx.rs:385-410`）：整帧旋转 θ 后要盖满画布
需缩放 `max((W·cosθ+H·sinθ)/W, (W·sinθ+H·cosθ)/H)`；反解各档最大安全角 = **远 0.00° / 中 10.24° / 近 17.98°**。
「远」档画面本来只有画布 55%（左右各 432px 黑边），**一格都转不得** ⇒ 同段剔除「远」是硬几何约束，
不是保守。6° 远在 10.24° 的安全线之内，**是"看起来明显但不露边"的值**。

#### 我的判断

**"没实现" —— ❌ 不成立。** 代码、接线、出片**三段全通**，且有真实成片逐像素证据。
用户体感"没实现"的**真实原因**有两个，都可修：

1. **编辑器里没有 tilt 控件** ⇒ 用户无法主动做荷兰角，只能等 LLM 恰好写；
2. **提示词的门槛很严**（`segment.rs:150-155`）：要求该段**情绪必须很高**（`fx == high` 或标成 `punch`）
   **且**语义属于 紧张/不安/尴尬/荒诞 **之一**，而 `punch`/`high` 在全片里本来就少
   ⇒ 结合「全片最多 2 条」，一条普通切片里**大概率一条都没有**。

⇒ **"看不见"是概率问题，不是实现问题。**

---

## §18-A 特写景别 —— **确认无生产者**（用户点名"镜头也没有特写景别"，体感准确）

### ① 代码 ✅

| 环节 | 文件:行号 |
|---|---|
| 档位名 | `crates/yekitrim-polish/src/process/fx.rs:328-329`（`CLOSEUP_LABEL`） |
| 守卫 | `crates/yekitrim-polish/src/process/fx.rs:354` `cap_closeup_anchor`（特写强制 `anchor = liver`） |
| 倍率字段 | `crates/yekitrim-render-core/src/types.rs:557-558, 574`（`zoom_closeup`，`"特写" => self.zoom_closeup`） |
| 倍率默认 | `types.rs:662` `default_zoom_closeup()`（1.8） |
| 出片模板 | `crates/yekitrim-polish/templates/index.html:609` —— `if(label==='特写'){v=c.zoom_closeup;d=1.8;}` |
| 预览 | `webui/src/polish/engine.js:463` —— `if (label === '特写') return pick(c.zoom_closeup, 1.8)` |

### ② 后端接线 ❌ **生产者不存在**

**这是总表标 ✅ 却看不见的根因。** 提示词 `crates/yekitrim-polish/src/llm/segment.rs:83`：

```
zoom = "远"/"中"/"近"（远=画面缩小成画中画）
```

`grep -c "特写" crates/yekitrim-polish/src/llm/segment.rs` → **0**。

⇒ **LLM 从来不知道「特写」存在** ⇒ 永远写不出 `zoom = "特写"` ⇒
`cap_closeup_anchor`（守卫）、`types.rs:574` 的倍率映射、模板 `:609` 的分支**全部是死代码**。

> ⚠️ 这正是任务书说的那个判据缺陷的**教科书案例**：转译器、模板、预览、守卫**四处都在**
> （所以"代码在 + 有消费者"= ✅），**唯独没有生产者**。

### ③ 前端 ⚠️ 半通

| 关键词 | 命中 | 文件:行号 |
|---|---|---|
| `特写` | **3** | `engine.js:463`（预览渲染）；`PolishView.vue:1817`（**倍率滑杆**）；`PolishView.vue:1818`（描述里提到） |
| `zoom_closeup` | **3** | 同上 |

⚠️ **滑杆 ≠ 能选景别**。`PolishView.vue:1817` 那条滑杆调的是**「特写档的倍率」**
（`title: '特写缩放（camera zoom=特写）。默认 1.8，比近景更近一档'`）——
**用户能调特写有多近，但没有任何地方能让某一段事件变成特写档。**

而**景别下拉**（`webui/src/components/TimelineEditor.vue:276-280`）只有三个 option：

```html
<option value="远">远（画中画）</option>
<option value="中">中景</option>
<option value="近">近景</option>
```

⇒ **LLM 写不出、人也选不了。**

### ④ 看得见吗：**看不见**

即使把 `zoom_closeup` 滑杆拖到 3.0，**在成片里也一个像素都不会变** ——
没有任何事件会是 `"特写"` 档，那条滑杆是**空转的**。

**修法**（未做，本任务是只读审计）：提示词 `segment.rs:83` 补上「特写」档 +
`TimelineEditor.vue` 景别下拉补一个 option。两处**必须同批**，只改一处就还是看不见。

---

## §18-C 暴露运镜档位配置 —— **三列全绿**

### ① 代码 ✅ / ② 后端 ✅ / ③ 前端 ✅

| 环节 | 文件:行号 |
|---|---|
| 配置字段 | `crates/yekitrim-render-core/src/types.rs:557-558`（`zoom_closeup`，另三条同段） |
| 倍率解析 | `types.rs:574` `"特写" => self.zoom_closeup` |
| API 夹取 | `crates/yekitrim-server/src/api/polish/:1869-1875`（四条：`zoom_mid/zoom_closeup/zoom_close/zoom_wide`） |
| **前端滑杆** | `webui/src/views/PolishView.vue:1816-1821`（`ZOOM_FIELDS`，4 条）+ `:1825` `ZOOM_DEFAULTS` |

`grep -rn "zoom_closeup" webui/src/` → **3 命中**（滑杆定义 + 预览）。

⚠️ 三处上下限必须一致（前端 `1.0~3.0` ↔ 后端 `1.0~3.0`），代码注释已互相点名
（`PolishView.vue:1813-1815`）；不一致的症状是"滑块拖到 3.0、保存后弹回 2.5"。

### ④ 看得见吗：**能看见**

滑杆在 PolishView 界面上，拖了保存后出片倍率真的变（`types.rs:574` → 模板 `:609` / `engine.js:463`）。

**⚠️ 但它只调倍率、不改档位** —— 参见 §18-A：这里能调"特写有多近"，却无法让任何事件**成为**特写。

---

## §6 爆点判据 —— **算了就扔**

### ① 代码 ✅

| 环节 | 文件:行号 |
|---|---|
| 信号 | `crates/yekitrim-polish/src/process/punch.rs:253` `subtitle_signal` / `:283` `danmaku_signal` / `:307` `loudness_signal` |
| 探测器 | `punch.rs:118-201`（`PunchDetector` / `detect`），`Punch` @ `:103` |
| 调用 | `crates/yekitrim-polish/src/process/mod.rs:345-358` |

### ② 后端接线 ⚠️ **生产者真跑了，但结果被丢进日志**

`crates/yekitrim-polish/src/process/mod.rs:325-363`：

```rust
let detector = punch::PunchDetector { subtitle_points: ..., danmaku_points: ..., sfx_points: ... };  // :342-346
let signals = detector.signals();                                                    // :347
let punches = detector.detect(&config.segments, total_duration, window);              // :351
tracing::info!("  · 爆点判据（§6）：{}；判出 {} 处", signals.describe(), punches.len());  // :354
if !punches.is_empty() { /* 打分布 :356-361 */ }
}   // :362 ← punches 在此**出作用域被丢弃**（块外零引用）
```

⚠️ **行号以 2026-10-11 工作副本为准**（`:315` 是 Step 1f 的注释头）。
判据：`punches` 的**块外引用数 = 0** —— 块在 `:362` 结束，下一个引用者是 `:3248` 的**单测**。

**`punches` 在块外没有被任何代码使用** —— 它只进日志。三路信号（字幕/弹幕/响度）**都真的接了**，
`Signals::describe()` 也会明说"这次用了哪几路"（含无弹幕时的降级警告），**但判出来的爆点不做任何事**。

> ⚠️ 注意 `:322-324` 的注释自己写着"§6 自己的验收标准就是'判据多信号且无弹幕不静默失效'，
> **不是'事件被改了'**" —— 所以**这是设计如此**，不是遗漏。但用户看不到任何产物。

### ③ 前端 ❌

| 关键词 | 命中数 |
|---|---|
| `punch` | **0** |
| `爆点` | **3**（`TimelineEditor.vue:120` 输入框 placeholder、`:189` `fx` 下拉的 `high · 爆点` 标签、`:660` 注释） |

⇒ 3 处「爆点」**都不是 §6 判据的展示**（`:189` 是 `fx` 强度档，与 §6 的多信号探测是两件事）。

### ④ 看得见吗：**看不见**

爆点只在**运行日志**里（`· 爆点判据（§6）：…；判出 N 处`）。界面上没有、成片里没有
（因为它不改变任何事件）。

---

## §7 静默分两类 —— **零消费者（死代码）**

### ① 代码 ✅

| 环节 | 文件:行号 |
|---|---|
| 枚举 | `crates/yekitrim-polish/src/project/silence.rs:46` `SilenceKind { Dragging, Comic }` |
| 判据 | `silence.rs:81` `classify_silence(start, end, punches)` |
| 分类 | `silence.rs:107` `split_silence_by_kind(ranges, punches)` |
| 保护窗 | `silence.rs:70` `COMIC_AFTER_PUNCH_SECS = 4.0` |

设计理由写得很清楚（`silence.rs:38-44`）：规格 §3.4 ——「**爆点之后的"短沉默"是笑点发酵的地方，
必须保护；该剪的是"拖沓的沉默"**」，且"今天 `DEFAULT_MIN_GAP_SECS = 3.5s` 一刀切，
于是一个 4 秒的包袱停顿被剪成 2.4 秒 —— 正好砍掉喜剧节拍"。

### ② 后端接线 ❌ **消费者不存在**

```
$ grep -rn "split_silence_by_kind" --include=*.rs .
crates/yekitrim-polish/src/project/silence.rs:107   ← 定义
crates/yekitrim-polish/src/project/silence.rs:280   ← 单测
crates/yekitrim-polish/src/project/silence.rs:598   ← 单测
crates/yekitrim-polish/src/project/silence.rs:615   ← 单测
```

**除自己的单测外，全仓零调用。** `classify_silence` / `SilenceKind` / `COMIC_AFTER_PUNCH_SECS` 同样零外部消费者。

### ③ 前端 ❌

| 关键词 | 命中数 |
|---|---|
| `SilenceKind` | **0** |
| `comic` | **0** |

（前端「去静音」按钮上只有二值：应用/撤销，`TimelineEditor.vue:18-20`。）

### ④ 看得见吗：**看不见**

⇒ 用户点「去静音」时，4 秒的包袱停顿**照样被剪成 2.4 秒**，与规格想避免的那件事完全一致。
**这条是"设计了但没接上"的典型**：连 §6 那种日志都没有（`split_silence_by_kind` 压根没被调用）。

> ⚠️ **§7 与 §6 是同一条断链的两段**：§7 需要 `punches` 作为输入
> （`classify_silence(.., punches)`），而 §6 的 `punches` 恰好在 `mod.rs:368` 被丢弃。
> **把 `punches` 传出那个块、喂给 `split_silence_by_kind` 就是完整的修法** —— 一处改动同时点亮两项。

---

## §8 `role` 叙事角色 —— **有生产者、有守卫，但不改输出**

### ① 代码 ✅

| 环节 | 文件:行号 |
|---|---|
| 提示词（分段） | `crates/yekitrim-polish/src/llm/segment.rs:361-364` —— 三选一 `hook`/`punch`/其余 |
| 提示词（配方） | `segment.rs:219-232` —— **只对 `hook`/`punch` 两档**加配方说明 |
| 字段 | `crates/yekitrim-render-core/src/types.rs:69,74` — `is_hook()` / `is_punch()` |
| **守卫** | `crates/yekitrim-polish/src/process/fx.rs:1407` `enforce_narrative_shape` |
| 规则 | `hook` 至多一段（留最早）@ `:1426`；`punch` 至少一段（提拔最长 `fx=="high"`）@ `:1443` |
| 调用 | `crates/yekitrim-polish/src/process/mod.rs:375` |

规格第三条「`setup` 不超过 40%」**有意不做**（`fx.rs:1393-1402`），理由：三档最小版里没有 `setup`，
"其余"天然是大多数段，拿 40% 卡它必然误伤。

### ② 后端接线 ⚠️ 只有提示词

`role` 的**唯一消费者是提示词本身**（`segment.rs:224` `match segment.role()`
→ 给 `hook`/`punch` 段加配方说明）。**它不改变任何事件、不影响出片。**

守卫 `enforce_narrative_shape` 也确实**只改 `role` 字段**（`fx.rs:1429` `role.clear()`、
`:1464` `role = "punch"`），产物是 `NarrativeShape` 的计数，只用于打日志（`mod.rs:376-382`）。

### ③ 前端 ❌ 0 相关命中

`grep -rn "role" webui/src/` → **10 命中**，但 **10/10 都是账号权限的 `role`**：

```
api/auth.js:32, components/ServerFileBrowser.vue:27 (role="tree"),
components/side/SideNav.vue:148,158, components/UserQuotaManager.vue:17,20,
router.js:88, stores/auth.js:21, views/IngestView.vue:107, views/WorkbenchView.vue:424
```

**叙事角色 `role` 在前端一个字符都没有。**

### ④ 看得见吗：**看不见**

用户既看不到某段是不是"钩子/爆点"，也无法手工指定。
`fx` 下拉（`TimelineEditor.vue:185-190` 的 low/med/high）是**另一个字段**（情绪强度），不是 `role`。

---

## §9 多版本成片 —— **CLI-only**

### ① 代码 ✅ / ② 后端 ✅（CLI 可达）

| 环节 | 文件:行号 |
|---|---|
| 候选推导 | `crates/yekitrim-polish/src/project/versions.rs`（`mod versions` @ `project/mod.rs:5`） |
| 入口 | `crates/yekitrim-polish/src/render/pipeline.rs:181` `cmd_render_versions(input, versions)` |
| CLI 参数 | `crates/yekitrim-cli/src/cli.rs:308-311` `versions: usize`（`--versions`） |
| CLI 分派 | `crates/yekitrim-cli/src/main.rs:394-397`（`versions > 1` 时走多版本） |

### ③ 前端 ❌

| 关键词 | 命中数 |
|---|---|
| `versions` | **0** |
| `多版本` | **0** |

### ④ 看得见吗：**WebUI 里看不见**

只有命令行 `yekitrim polish render <目录> --versions N` 能用。WebUI 无任何入口 ⇒
**普通用户（走界面）永远出不了多版本。**

---

## §10 情绪轨道 —— **没有独立轨道**

### ① 代码 ⚠️

没有 `emotion` 字段/轨道。`grep -rn "emotion" crates/yekitrim-polish/src/` → **0** 命中
（命中的都是 `sfx_catalog.rs` 的"① 主播情绪反应"分类名和 `commands.rs` 的贴纸 tag 提示词，与轨道无关）。

现有的是**段级** `fx` 强度档（`low`/`med`/`high`），**不是时间轴上的情绪轨道**：

| 环节 | 文件:行号 |
|---|---|
| 段情绪密度 | `crates/yekitrim-polish/src/llm/mod.rs:71`（Step 1 分段 + 段情绪密度） |
| 用途 | `segment.rs:151` 荷兰角门槛、`fx.rs:1446` punch 提拔（`fx == "high"`） |

任务书说「L3 LLM 打标**有意没做**」—— 与代码一致：**没有情绪轨道这个东西**。

### ③ 前端 ⚠️ 1 处（借名）

`grep -rn "情绪" webui/src/` → **1**：

```
webui/src/components/TimelineEditor.vue:185:  <label>情绪</label>
            :186:  <select v-model="selItem.fx">
            :187-189:  <option value="low">low · 安静叙述</option> / med · 日常 / high · 爆点
```

⚠️ **这是 `fx` 字段的 UI，标签写成了「情绪」。** 它确实给了用户一个"给这段标情绪"的入口 ——
但它是**段级枚举**，不是"情绪轨道"（没有时间轴、不能做曲线）。

### ④ 看得见吗：**要特定条件**

`fx` 下拉在时间线编辑器里，选中一个 segment 就能改（能看见）。
但**"情绪轨道"作为一个概念不存在** ⇒ 用户要的如果是"一条随情绪起伏的轨道"，**看不见**。

---

## §11 BGM —— **后端全通，前端零命中**

### ① 代码 ✅

| 环节 | 文件:行号 |
|---|---|
| 配置 | `crates/yekitrim-polish/src/render/bgm.rs:26` `BgmConfig`（`enabled` 默认 **false**、`file`、`volume` 0.22、闪避） |
| 混音链 | `bgm.rs:147` `bgm_filter_chain` |
| 接入 | `crates/yekitrim-polish/src/project/config.rs:71`（`pub bgm: BgmConfig`） |
| 消费 | `crates/yekitrim-polish/src/render/pipeline.rs:915-930` |

⚠️ 按设计**只做引入、不做卡点**（无节拍网格 ⇒ 不适用项），`enabled` **默认关**。
`pipeline.rs:916-919`：`enabled = true` 但没选曲 ⇒ **warn 并静默不出 BGM**。

### ② 后端接线 ⚠️ 只到 pipeline

**`grep -rn "bgm\|Bgm" crates/yekitrim-server/src/` → 0 命中。**
**`grep -rn "bgm" crates/yekitrim-cli/src/` → 0 命中。**

⇒ **没有 API、没有 CLI 开关**。唯一启用方式 = **手写 `polish.toml` 的 `[bgm]`**。

### ③ 前端 ❌ **0 命中**

`grep -rni "bgm" webui/src/` → 16 命中，**逐条核对全部是 CoverDesigner 的封面背景**
（`bgMode` / `bgImage` / `bgBlur` / `bgStamp` / `bgFrame` / `bg_mode` 等）：

```
CoverDesigner.vue:23,24,25,133,134,136,143,151,567,1320 …（全是封面背景图/模糊模式）
```

排除这些噪声后，**「背景音乐」真命中 = 0**。

### ④ 看得见吗：**看不见**

WebUI 里没有任何 BGM 入口 ⇒ 用户无从开启。即使手改了 `polish.toml`，
**前端也无法把音频文件放进 `<工程>/bgm/`**（`bgm.rs:117` 读的是工程目录下的 `bgm/`），
只能手工拷文件。

---

## §23 节奏求解器 —— **只报告、不落盘（设计如此）**

### ① 代码 ✅

| 环节 | 文件:行号 |
|---|---|
| 求解器 | `crates/yekitrim-polish/src/process/pacing.rs:99` `find_idle_gaps(duration, events)` |
| 阈值 | `pacing.rs:38` `IDLE_THRESHOLD_SECS = 6.0` |
| 模块说明 | `pacing.rs:1-27`（三条保守取舍） |
| 调用 | `crates/yekitrim-polish/src/process/mod.rs:925` |

### ② 后端接线 ⚠️ **只报告不落盘（有意）**

`crates/yekitrim-polish/src/process/mod.rs:907-913` 的注释明写：

```
// ⚠️⚠️ **本趟只报告、不落盘**（保守决策：R4 零回归）。
//    理由…规格特别强调过"留白是'没有说话'，不是'画面死着'"，
//    而本仓有**大量**安静叙述切片 —— 自动往它们身上补运镜，是**改观感**的行为…
```

`:925-948` 只把扫出的空档 `tracing::info!` 出去（含"建议补 N 档最低强度推近"），
**`config.segments` 一个字节都不动**。

### ③ 前端 ❌

| 关键词 | 命中数 |
|---|---|
| `节奏` | **0** |

### ④ 看得见吗：**看不见**

只有日志。任务书说的"只报告不写配置"**核实属实**，且**代码注释明确说这是保守决策**
（不是漏做）。要让用户看见，需要：① 一个开关（写配置）；或 ② 把扫出的空档显示在时间轴上。

---

## ⚠️ 顺带发现：一个**与本任务无关的真实 bug**（未修，只报）

审计过程中为了给 §19 做出片证据，踩到一个 **`--keep-muted` 路径的合流断言误报**。
它与 §18/§19/§6-§11/§23 **无关**，但会**阻断出片**，所以我照实报：

**症状**：无 EDL、有 `mute_ranges` 的工程（即 `--keep-muted` 那条腿）出片**必然失败**：

```
Error: 音视频合流失败: 成片画面帧数与成片时间线不一致：
       派生 540 帧（9.000s），实得 720 帧（12.000s，fps=60）。
```

**根因**（`crates/yekitrim-polish/src/render/pipeline.rs`）：

- `:540-549` `edl_total_frames` 在**无 EDL** 时取的是 **mute 补集**的长度（12s−3s = **9s = 540 帧**）；
- 但 `--keep-muted` 那条腿出的画面是**整条 12s（720 帧）**，尾部那道 `cut_ranges` 在
  `:1050` 才剪 —— **晚于** `:1020` 的断言。

⇒ 断言用的是"剪完之后的长度"，却在"剪之前"执行。`check_frames` 应当在无 EDL 时传 `None`
（或传源长），真正要卡的长度是剪完之后。

**触发面**：`grep` 全仓 `polish.toml`，**有 `mute_ranges` 且无 EDL 的工程 = 2 个**
（`target/edl-e2e/output/clips/demo-clip/`、`target/io-e2e/output/clips/demo-clip/`）。
两个都在 `target/` 下（构建产物），所以**存量用户工程暂未被波及**，
但任何用户点「去静音」后出片都会撞上。

---

## 复现命令（全部零改代码，产物都在 `.workbuddy/`）

```bash
# §19 孪生对照出片（tilt=4.5 vs tilt=0.0）
VTEDIT_DHAMPIR_CLI=<repo>/target/debug/dhampir/bin/dhampir.exe \
  ./target/debug/yekitrim.exe polish render .workbuddy/tilt-clean
VTEDIT_DHAMPIR_CLI=... ./target/debug/yekitrim.exe polish render .workbuddy/tilt-off

# 转译产物里的 rotation 关键帧
node tools/polish-to-dhampir.mjs .workbuddy/tilt-clean --out .workbuddy/tilt-clean/out.doc.json

# 逐项 grep
grep -rn "特写" webui/src/          # 3
grep -rn "tilt" webui/src/          # 6
grep -rn "punch" webui/src/         # 0
grep -rn "SilenceKind" webui/src/   # 0
grep -rn "bgm\|Bgm" crates/yekitrim-server/src/   # 0
grep -rn "split_silence_by_kind" --include=*.rs . # 仅自身单测
```

---

## 结论：断链的三种形状（本批 10 项的实际分布）

| 形状 | 项 | 特征 |
|---|---|---|
| **① 无生产者** | §18-A 特写 | 下游四处齐全，**唯独 LLM 提示词不教** ⇒ 永远触发不了 |
| **② 算了就扔** | §6 爆点、§7 静默分类（同一条断链的两段）、§23 节奏 | 计算真跑了，结果只进日志/直接丢弃 |
| **③ 无界面** | §9 多版本、§11 BGM、§8 role、§10 情绪轨道 | 后端可用，**WebUI 零入口**，普通用户够不到 |
| **对照组：真的通了** | **§19 荷兰角**、§18-C 运镜档位 | 出片实测可见（§19 歪了 4.5°），§18-C 有滑杆 |

⇒ **用户"体感就是什么都没做"是可解释的**：10 项里 **8 项**要么没生产者、要么算了不用、
要么没有界面。**唯一用户能主动操作的是 §18-C 的四条倍率滑杆**，而它调的是"已有档位有多近"，
**不能创造新档位**（§18-A 就是这么卡住的）。

⚠️ **最能解释体感的一条**：§19 荷兰角**真的做了、成片真的歪**，
但**编辑器里没有 tilt 控件** + **提示词门槛严（要求 `high`/`punch` + 特定语义）+ 全片上限 2 条**
⇒ 用户在自己片子里**大概率一次都遇不到**。**"实现了但看不见"与"没实现"在体感上无法区分** ——
这正是本批要分辨的东西。
