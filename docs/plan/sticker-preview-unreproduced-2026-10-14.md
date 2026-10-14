# 贴纸「编辑器设置了、预览不显示」——已验层与未复现说明

> 2026-10-14。用户口径：**「贴纸在前端编辑器设置了后，预览无法正常显示，这是这个版本采用的问题」**。
> 状态：**未复现**。下面记的是**已经验证过是对的**那几层，以及**还剩哪些没验**。

## 为什么要写这份

本仓的纪律是「**真机可见**才算证据」。这一条我**没能**复现，
所以**不能**说"修好了"，也不能假装它不存在 —— 只能把"验到哪一步"如实写下来，
免得下一个人从零再走一遍，或者误以为它已经被解决。

---

## 一、已经**逐层实测**、结果都对的

### 1. 预览求值器（`engine.js::stickersAt`）—— 对

构造与组件 `indexConfig` **完全同形**的配置直接驱动：

| 场景 | 实测输出（画布 1920×1080） |
|---|---|
| 已保存 `manual 50% 50%` | `x=820, y=382, opacity=1` |
| 草稿 `manual 20% 30%`（点选中间态） | `x=244, y=166` |
| `auto`（事件 `x=50,y=60`） | `x=960, y=630` |
| `manual` 但事件**没有** `x/y`（没跑过 `process`） | `x=244, y=166` ✅ |
| `auto` 但事件**没有** `x/y` | `x=960, y=522`（居中兜底）✅ |
| 没有 `_canvas.sticker` 整块 | `x=960, y=522` ✅ |
| ⚠️ `x/y = "auto"`（**已知陷阱**：没跑 process 时该键存在、值是字符串） | `x=960, y=522`（**正确落到兜底，没有 NaN**）✅ |

⇒ 草稿响应、两种基准（manual=盒中心 / auto=盒左上角）、`"auto"` 字符串陷阱、缺字段兜底，**全部正确**。

### 2. 转译器 —— 对（**真产物**测的）

造真工程跑 `tools/polish-to-dhampir.mjs`，读产物 JSON：

| `[sticker]` | 产物里那一层的 `transform.x` |
|---|---|
| `manual "20% 30%"` | **−576** |
| `manual "80% 90%"` | **+576** |
| `auto` | **140** |

⇒ 三档**明显可分**，`manual` 落点真的进了成片产物。
（⚠️ 夹具里**必须放真的贴纸文件**，否则整条 DROP、量到的是"没进产物"，
而真因只是夹具缺文件 —— 我第一次就是这么被骗的。）

### 3. 配置与 API —— 对

- `StickerConfig.position_mode / position` 字段与默认值正确
  （`crates/yekitrim-polish/src/project/config.rs:1063`）；
- 热重载指纹**包含** `cfg.sticker` 子树
  （`PolishPreview.vue` 的 `persistedConfig` watcher）⇒ 改落点**会**触发重载；
- 编辑器的 `toggleStickerMode` / `startPointEdit` / `savePointEdit` / `parsePos` / `fmtPos`
  逐行读过，逻辑自洽。

---

## 二、**还没验**的（下次从这里接着查）

我能在命令行跑的都跑了；剩下的都需要**真浏览器 + 真工程**：

1. **真机 DOM**：`.sticker-layer` 的 `overflow:hidden` +
   `.overlay-space` 的 `transform: scale(overlayScale)` 组合下，贴纸是否被裁掉
   （`--sticker-s` 继承自 `.stage`，理论可继承，但**没在真浏览器里量过**）。
2. **Dhampir 那条腿**：`startDhampir` 之后贴纸由**底座**画，
   而 `reloadDhampirProject` 只在 `persistedConfig` 变时触发 ——
   "点选草稿态"（`props.point`）**只走 DOM 那条腿**、不进引擎。
   ⇒ 若用户当时看的是 **Dhampir 预览**，那么**拖草稿时画面不动是符合设计的**，
   但**保存之后**应当热重载。这一步没实测过。
3. **用户的具体工程**：`position_mode` 是不是真的被写进了 `polish.toml`
   （`patchSticker` 走 `saveProjectPatch`，失败会 toast —— 现场有没有 toast？）

---

## 三、下次怎么一次问清楚

要区分"整条不显示"和"位置不对"，两个问题就够：

- 预览里**完全看不到**贴纸，还是**位置不对**？
- 当时是 **Dhampir 预览**（走底座）还是 **DOM 预览**？
- 「固定」模式下点完「保存落点」，`polish.toml` 的 `[sticker]` 那一节有没有变化？
