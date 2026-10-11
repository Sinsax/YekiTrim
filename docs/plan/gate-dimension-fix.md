# 批 0.2：修门禁的「维度漏洞」——`check-preview-engine.mjs`

> 立档：2026-10。任务：task-25。**只改判据 + 词汇表登记**，不改产品行为（`engine.js` 留给 task-26）。

---

## 1. 缺陷：判据「查了 A 就以为查了 B」

`scripts/check-preview-engine.mjs` 原先查 5 件事（①–⑤），**全部围绕"桶登记了没有"**：

| 原判据 | 查的是什么 |
|---|---|
| ① 桶清单 | `INDEXED_TYPES` 与模板 `buildIndex()` 的**成员集合**一致 |
| ② 空工程建索引 | 每个桶都是数组（不是 `undefined`） |
| ③ 求值器族 | 10 个 `(idx, t)` 求值器**不抛** |
| ④ `videoFxAt` 字段 | 返回对象字段齐 |
| ⑤ 词汇表 `ui` | 是数组 |

⚠️ **这五条全绿，也完全不能说明"预览画得出这个效果"。**

"桶登记了"与"画得出来"差着**一个维度**，而**少的那一维恰好是错的所在**。
这与旧总表「只查消费者不查生产者」**同一族** —— 两次都是验证维度少了一维。

### 实证的洞

```
webui/src/polish/engine.js:889-890
    const kind = String(e.kind ?? '').trim().toLowerCase()
    if (kind !== 'sticker') continue
```

`element` 桶登记得好好的、求值器也不抛，但 `getElements` **恒返回空数组** ⇒
**预览一片空白、成片（模板 `handlers.element`）画得出** —— 而 `check-preview-engine.mjs` 全绿。

### ★ 核实中发现：这个洞有**两处独立成因**（比任务描述更深）

| # | 成因 | 位置 |
|---|---|---|
| **(a)** | `if (kind !== 'sticker') continue` —— 只放行贴纸 | `engine.js:890` |
| **(b)** | **字段路径错位**：它读 `e.kind` / `e.text`，而 `buildIndex` 只把载荷放在 **`e.ev.kind` / `e.ev.text`** | `engine.js:889` / `:916` |

实测证据（`buildIndex` 产出的 entry 形状）：

```
e.kind     = undefined        e.ev.kind = "sticker"
e.text     = undefined        e.ev.text = "生气"
```

⇒ **`kind` 恒为 `""`，第一个 `continue` 就出局** —— 就算修掉 (a)，(b) 仍让预览一片空白。
（对照：`stickersAt` 读 `e.time` / `e.duration`，那两个**确实在 entry 上**，所以它是对的。）

**为什么它能一路绿灯**：`grep -rn "getElements" webui/src/` **只有定义那一行** ——
**这个函数没有任何消费者**，所以从没被跑过。判据只查"名字在不在"，自然抓不住。

---

## 2. 新判据：②b「登记了」必须伴随「画得出来」

### 查什么

对一个桶的**绘制路径存在性**，判据不是"文件里出现过某个词"，而是：

> 喂一条**该类型的真事件**，跑**预览真正会调的那个求值器**，
> 要求它的输出与「空工程基线」**可观测地不同**。

逐桶的探针表（`DRAW_PROBES`，12 条全覆盖）：

| 桶 | 触发事件 | 读什么（预览真会调的那个） |
|---|---|---|
| `camera` | `{type:'camera',zoom:'近'}` | `videoFxAt(idx).camera.zoom` |
| `sticker` | `{type:'sticker',name:'生气'}` | `stickersAt(idx).length` |
| `shake` | `{type:'shake',intensity:14}` | `videoFxAt(idx).shake` |
| `flash` | `{type:'flash'}` | `getFlash(idx).opacity` |
| `blur` | `{type:'blur'}` | `videoFxAt(idx).blur` |
| `overlay` | `{type:'overlay',overlay_type:'color'}` | `getOverlayState(idx)` |
| `vignette` | `{type:'vignette'}` | `getVignette(idx)` |
| `hue_shift` | `{type:'hue_shift'}` | `getHueShiftFlash(idx).opacity` |
| `noise` | `{type:'noise'}` | `getNoiseOpacity(idx)` |
| `pulse` | `{type:'pulse'}` | `videoFxAt(idx).zoom` |
| `sfx` | `{type:'sfx',name:'pop-notice'}` | `idx.soundEvents.length`（`audio.js::sync` 消费它） |
| `element` | `{type:'element',kind:'sticker',text:'生气'}` | `getElements(idx).length` |

### 三条**覆盖面**判据（防"新加的桶从缝里溜过去"）

1. **探针覆盖全**：`INDEXED_TYPES` 里每个桶都必须有探针，缺了就红（点名要求补）；
2. **探针参数正确**：三条踩过的坑已写进注释 ——
   · `shake` 不给 `intensity` 时 `getShake` 回 `{0,0}`（与基线相同 ⇒ **假红**）；
   · `pulse` 可视窗口只有 **0.48s**，`t` 取 1.0 早已出窗（同样假红）；
   · `hue_shift` 在空索引上会抛 ⇒ 基线那侧必须用**正常 `buildIndex` 产物**，不能拿 `{}`。
3. **⚠️ `null` / `0` 都是合法输出** ⇒ 基线比较用 `JSON.stringify` **逐值比**，不用真值判断。

### ②c 词汇表交叉检查（出口检查）

`spec/effects.json` 的 `dom_preview: true` 是**对外承诺**，②b 是**跑出来的事实**。
两者不一致时登记表就在骗下一个人（§20 的 `noise` 就是这么骗过去的）。
判据：`dom_preview === true` ⇒ 该桶**必须**能画出来，否则红。

---

## 3. `element` 的处置：**已知死桶基线**（本仓房规）

修 `engine.js` 是 **task-26** 的写范围（它登记了 `webui/src/polish/engine.js` + `webui/dist`，
且验收要求"真机可见"的同帧对比）。⇒ task-25 只修**判据**，把 `element` 钉成
`KNOWN_DEAD_BUCKETS` 基线（照 `check-dhampir-drop-list.mjs` 的房规：**每条都写清"为什么它还在"**）。

⚠️ **基线的方向性是双向的**（这是它不沦为"掩护"的关键）：

| 情况 | 结果 |
|---|---|
| 一个新桶画不出来、**不在**基线里 | **红** —— 新缺陷立刻可见 |
| 一个**在**基线里的桶**修好了**、能画出来了 | **红** —— 提醒删基线，否则它永远替一个已修好的洞打掩护 |

两条方向**都实测验证过**（见 §5）。

---

## 4. 修改清单

| 文件 | 改动 |
|---|---|
| `scripts/check-preview-engine.mjs` | 新增 ②b（绘制探针 + 覆盖面）、②c（词汇表交叉检查）、`KNOWN_DEAD_BUCKETS` 基线、`--mutate` 变异台；**原 ①–⑤ 一条未动** |
| `spec/effects.json` | `element.dom_preview`：`true` → **`false`**，并加 `_note_dom_preview` 写清两处成因与"修好改回 true"的指引 |
| `docs/plan/gate-dimension-fix.md` | 本文 |

⚠️ **未改 `webui/src/polish/engine.js`**（产品行为留给 task-26），
⚠️ **未改 `webui/dist`**（没动 `webui/src`，无需重建）。

---

## 5. 变异自证结果

### 5.1 `--mutate`：6 处变异**全部打红**（真改源码 + 真加载变异体）

跑法：`node scripts/check-preview-engine.mjs --mutate`

```
  ✓ 变异 #1 打红：§element 的洞本身：`getElements` 里 `kind !== "sticker"` 提前 continue（**原始缺陷 a**）
      ↳ 桶 `element` 翻成"画不出来"（有事件=0 vs 基线=0）
  ✓ 变异 #2 打红：§element 的洞之二：字段路径错位 `e.kind` →（索引里其实是 `e.ev.kind`）
      ↳ 桶 `element` 翻成"画不出来"（有事件=0 vs 基线=0）
  ✓ 变异 #3 打红：`getElements` 干脆不往 out 里推（画不出来但也不抛 —— 静默空白）
      ↳ 桶 `element` 翻成"画不出来"（有事件=0 vs 基线=0）
  ✓ 变异 #4 打红：`stickersAt` 的窗口条件写反（贴纸永远出不来）
      ↳ 桶 `sticker` 翻成"画不出来"（有事件=0 vs 基线=0）
  ✓ 变异 #5 打红：`getNoiseOpacity` 恒返回 0（噪点桶登记了、画不出来）
      ↳ 桶 `noise` 翻成"画不出来"（有事件=0 vs 基线=0）
  ✓ 变异 #6 打红：`getFlash` 恒返回 0 不透明度
      ↳ 桶 `flash` 翻成"画不出来"（有事件=0 vs 基线=0）
  ✓ 反向控制：未变异的 engine.js 上除已知基线（element）外全部能画出来

✓ 变异自证：6/6 处变异全部打红（判据不是空转的）
```

**三步协议**（照 `check-yekitrim-translator.mjs` 的房规）：
① 断言替换锚点**命中恰好 1 次** → ② 断言替换后文本**真的变了** → ③ 用 `import()` **真加载**变异体
（新 URL ⇒ 模块缓存不串）、拿它跑**与 `main()` 同一份** `drawPathReport`，要求目标桶**翻面**。
⚠️ 变异体写在系统临时目录，**不动仓库文件**；`engine.js` 的相对依赖 `canvas.js` 一并复制
（第一版没复制 ⇒ `ERR_MODULE_NOT_FOUND`）。

**反向控制**：未变异的源码必须仍然"除基线外全绿" —— 否则"翻面"可能只是判据恒假。

### 5.2 锚点写错会被当场拦下（自证"自证本身"不是空转的）

我第一版把 `stickersAt` 的锚点写成单行 `if (t < et || t > et + dur) continue` ——
它在 `engine.js` 里出现**两次**（`getElements:892` 与 `stickersAt:926`）。门禁当场报：

```
Error: 变异自证失败：「`stickersAt` 的窗口条件写反（贴纸永远出不来）」的锚点在 engine.js 里命中 2 次（应为 1）。
  ⇒ 变异台写错了（锚点漂了），这次自证是**空转**的 —— 修锚点，别放过。
```

这正是本仓四次"变异该红没红"的防线的价值 —— **它抓到了我自己的错误**。

### 5.3 基线两个方向都实测验证

**方向 1（未知死桶必红）**：把 `element` 从基线里移除（模拟"未登记的新死人"）：
```
Error: 这些桶「登记了」但**画不出来**，且**不在**已知基线里：
  · element: 有事件=0  基线=0
  ⇒ 症状 = 预览里看不到、成片里画得出，且过去**没有任何门禁会红**。
```

**方向 2（修好了仍在基线必红）**：造一份"真修好"的 `engine.js` 副本
（同时修 (a) 去掉 `continue`、修 (b) 改读 `e.ev.kind` / `e.ev.text`；实测 `getElements` 返回
`[{"src":"","opacity":0.265...,"scale":0.779...,"rot":-4.72...,"yOff":0}]`），
让门禁指向它：
```
Error: 这些桶**已经能画出来了**，但仍挂在 KNOWN_DEAD_BUCKETS 里：element
  ⇒ 缺陷修好了就该把它从基线里删掉 —— 留着会让这条基线**永远替一个已经修好的洞打掩护**。
```

**方向 3（②c 交叉检查可红）**：把 `element.dom_preview` 改回 `true`：
```
Error: 这些效果在 spec/effects.json 里写着 `dom_preview: true`，但**预览画不出来**：element
  ⇒ 登记表承诺了预览支持，实际没有 —— 下一个人会照它行事，这正是 §20 那条能一路绿灯的原因。
```

⚠️ 方向 2 的第一次尝试**没能打红** —— 因为我模拟的"修复"只改了 `if` 条件、没修字段路径，
所以 `getElements` **仍然返回空**，判据（正确地）没红。这说明判据咬的是**真行为**，不是文本。
发现 (b) 正是从这次失败里来的。

---

## 6. 门禁结果

| 命令 | 结果 |
|---|---|
| `node scripts/check-preview-engine.mjs` | **EXIT 0**（22 项全绿） |
| `node scripts/check-preview-engine.mjs --self-test` | **EXIT 0** |
| `node scripts/check-preview-engine.mjs --mutate` | **EXIT 0**（6/6 打红） |
| `node scripts/check-effect-registry.mjs` | **EXIT 0**（112 个消费点一致） |
| `node scripts/check-effect-registry.mjs --self-test` | **EXIT 0**（8 项自证） |
| `python3 scripts/check_module_map.py` | **EXIT 0** |
| `python3 scripts/check_module_boundaries.py` | **EXIT 0** |

`webui/scripts/check-*.mjs` 10 个：**9 个 EXIT 0**；`check-noise-parity.mjs` **EXIT 1**。

⚠️ **`check-noise-parity.mjs` 的红不是本任务造成的**，如实记录：
- 它是**未入库的新文件**（`git status` 显示 `??`，mtime 2026-10-12 01:13），由 §20 那条线（task-23/26）创建；
- 它的 4 项失败是「预览 `engine.js` / `PolishPreview.vue` 没有 `noiseTile`/`noiseHash`」——
  即**§20 那个未修的缺陷本身**（见 `docs/plan/reverify-batch-A.md` §20）；
- 我**没有**改 `engine.js`，所以这条红在我开工前后**状态相同**。

---

## 7. 结论与交接

**新判据查什么（一句话）**：对 `INDEXED_TYPES` 的**每一个桶**，喂一条该类型的真事件，
跑预览真正会调的那个求值器，要求输出与空工程基线**可观测地不同** ——
证明的是**绘制路径存在**，不是"文件里出现过某个词"。

**交给 task-26 的两件事**：
1. **`element` 的洞有两处**（(a) `continue` + (b) `e.kind`/`e.text` 字段路径错位），
   修的时候**两处都要验** —— 只修 (a) 仍然画不出来（本文 §5.3 方向 2 的第一次失败就是证据）。
2. 修好后**门禁会主动报红**提醒你从 `KNOWN_DEAD_BUCKETS` 删掉 `element`，
   并把 `spec/effects.json` 的 `dom_preview` 改回 `true`。

**顺带发现（不在本任务范围，建议另立）**：`spec/effects.json` 的 `sfx` 写着
`dom_preview: false`，但它经 `idx.soundEvents` → `audio.js::sync` 在预览里**真的会响** ——
属于"登记得比事实保守"。②c **只报告不判红**（保守登记不会误导人，
判红会在本任务范围外制造一条红门禁），但建议对齐。
