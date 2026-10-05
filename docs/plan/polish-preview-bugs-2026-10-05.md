# 精修预览 5 个问题的定位与处置（2026-10-05）

> 用户报的 5 条，逐条给**根因 + file:line**。已修的记"已修 + 提交"；需要拍板或跨仓的记在
> 「待决」。⚠️ 其中 **② 与 ③ 的初判都被后续证据推翻过**，本文只留**核实后**的结论，
> 被推翻的版本写进「走过的弯路」一节，免得有人照着错的再走一遍。

---

## ① 改字幕后预览不刷新 —— **已修**（`da6de7e` → `efeeb2a`）

**根因**：Dhampir 热重载由 `persistedConfig` 的**内容指纹**驱动
（`webui/src/components/PolishPreview.vue` 那个 `watch`），指纹只取
`segments / mute_ranges / silence_ranges / layout / canvas / stage / sticker / style / filter /
sfx / sticker_source / duration` —— **不含字幕**。而字幕**不住 `polish.toml`**（住
`data/subtitles.json`）⇒ 只改字幕时那几棵子树**一个字节都不变** ⇒
`if (key === dhampirCfgKey) return` 把这次改动吞掉。

⚠️ 重载**本身**会重喂字幕（`reloadDhampirProject` 里 `await dhampirApplySubtitles(...)`，
2026-10-02 就修过）—— 缺的只是**触发**。典型的"半次修复"：链路接通、判据没接。

**改法**：给字幕加"磁盘上那份"信号（与 `persistedConfig` 同款，拖动中的镜像不算数）：
`PolishView.onEditSaved()`（时间轴 `@saved`，字幕保存成功也走它）更新 `persistedSubs`
（**深拷贝**）→ 新 prop `:persisted-subtitles` → 进指纹。

## ② LLM 剪辑后黑屏（F5 才好）—— **已修**（`efeeb2a`）

**根因**：`PolishView.vue:1230` 那句 `if (!silent && !keepPreview) previewKey.value++` ——
LLM / llm_segment / render 三个任务入口都**没传** `keepPreview`，任务成功就把整个
`PolishPreview` **重挂**、`#dhampir-canvas` 被换成新元素。而底座
`dhampir_project_attach()` 是**一页只接一次**的语义
（`../Dhampir/crates/dhampir-wasm/src/timeline_host.rs`：`PROJECT_HOST` 存在就直接返回
`{"already":true}`，**没有 detach 导出**），wasm 实例又是页面级单例
（`mod.default()` 是 wasm-bindgen 的幂等 init，`pkg/dhampir_wasm.js`：`if (wasm !== undefined) return wasm`）
⇒ 新 canvas **从出生起没被 present 过**，引擎继续把帧画在**已卸载的旧 canvas** 上，
新 canvas 露出自己的 CSS `background:#000` ⇒ 整块黑；而工具条 fps/帧号**照涨**、控制台**无报错**。
F5 能好 = 新的 wasm 实例（`PROJECT_HOST = None`）。

**改法**：任务完成后**绝不重挂预览**。要刷新的东西本来就由**就地表**驱动：
`after` 里的 `load({silent:true})` → `persistedCfg` → `reloadDhampirProject()`
（它存在就是为了"改效果/挪事件时间不用刷新页面"），顺带白拿"播放位置不丢"。

**⚠️ 尚未覆盖的同类路径**（同一个底座限制，任何"换掉 canvas 元素"都会黑）：
换片段/换工程（路由或 `ready` 翻转）、离开精修页再回来。两条候选：
- **B（结构性，推荐）**：把 `<canvas id="dhampir-canvas">` 与 `.dhampir-hosts` 提到 keyed 子树**之外**
  （`PolishView` 里），用 prop/ref 传进预览 ⇒ 以后任何 `:key`/`v-if` 都不会换掉它。
- **C（正缝，跨仓）**：给底座加 `dhampir_project_detach()`（`PROJECT_HOST = None`），
  `stopDhampir()` 里调它；加了要同步 `scripts/check-dhampir-wiring.mjs` 的跨仓对拍。

## ③ shake 与运镜同时存在时 shake 没正常触发 —— **R1 已修，R2/R3/R4 处置见下**

⚠️ **先证伪一条**：不是"竖素材运镜门（`CFG.vertical`）把 shake 吃掉"。
`getShake`（模板 `index.html:1606-1620` / `engine.js:413-426`）**一个字段都不读 camera**；
`renderFrame` 里 shake 的 `ctx.translate`（`:1876`）在 camera 块（`:1890-1896`）**之外**；
canvas 后乘 ⇒ shake 在设备空间、**不被 `camScale` 缩放**（实测 zoom=1.5 时仍是 `translate(42px,-42px) scale(1.5)`）。
`handheldSway` 与 shake 是**相加**（`:1891` / 转译器 `:2928-2929`），不互相覆盖。

| # | 根因 | 触发条件 | 症状 |
|---|---|---|---|
| **R1（主力）** | **`intensity` 单位漂了三处**：LLM 提示词的**唯一示例**写 `0.5`（`crates/yekitrim-polish/src/llm/segment.rs:96`，全篇无单位），而 4 个消费者**一律当像素**（模板 `:1610`/`:1021`、`engine.js:416`、转译器 `:1718`，注释 `:1707` 明写"是像素"）；编辑器默认又是第三个值 `10`（`TimelineEditor.vue:1288`）；`docs/polish-events-reference.md:255` 写"默认 14px" | LLM 生成的 shake（正常路径） | 0.3–0.9 **像素** ⇒ 亚像素 ⇒ **预览和成片都看不见**，而报告写 `shake_events: 1` |
| **R2** | 竖画布整类门里 `shake→effects`、`camera→camera` 是**两个独立开关、缺字段=false**（模板 `:1377-1392` / `engine.js:196-223` / 转译器 `:1504-1521`） | 竖画布工程，只勾「运镜」没勾「特效」 | **运镜照转、shake 整条丢**（实测 `camera_events:1 / shake_events:0`）；**片头 fade_in 也没了**（同一个开关） |
| **R3** | 转译器"密集键往前铺"的**右端封在窗口末尾**（`tools/polish-to-dhampir.mjs:2881-2885`）⇒ 窗口之后到下一个稀疏键退化成**两点直线弦**，`pow2_out` 被拉直 | **shake 窗口压在运镜过渡上** | 过渡后段 zoom 偏 ~0.04（~3%／画面边缘 ~29px）。全仓**唯一**"有 shake 就改变运镜行为"的路径 —— 正是用户说的"同时存在时" |
| **R4** | 中性相机基准只写给"**0 键层**"（`:3718`） | 竖素材（无运镜键）+ 有 shake/pulse | 键只盖窗口那 13 帧，窗口外构图错（只在"竖素材进横画布"这类非 fill 几何显形；需底座侧或像素确认） |

**给用户的 30 秒分流**：
```bash
grep -A3 'type = "shake"' <片段目录>/polish.toml                      # intensity < 2 ⇒ R1（亚像素）
grep -n 'orientation' -A8 <片段目录>/polish.toml | grep portrait      # 竖画布且无 portrait_effects=true ⇒ R2
```
命中 R1 ⇒ 手改 `14` 试一次（立刻可见）；命中 R2 ⇒ 到「画面构成 → 竖屏启用」勾**特效**。

**R1 · 已修（2026-10-05 续）**：8 处落点一次改齐 —— 提示词示例 `0.5 → 14` 并写明「是**像素**、0 = 不抖、别写亚像素」
（`llm/segment.rs:96`）· 编辑器/示例工程默认 `10 → 14`（`TimelineEditor.vue:252`/`:1288`、模板 `:2690`、
`commands.rs:713`）· 两份 JS 取默认改成**数值判定**（模板 `:1021`/`:1610`、`engine.js:421`；房规同
`webui/src/polish/AGENTS.md` 里那条"不透明度一律数值判定，别写 `|| 1`"）。**按用户"不兼容过去内容"的口径，
不做按值迁移**。判据两条都能翻面：Rust 单测 `llm::segment::tests::shake_prompt_example_is_pixel_scale`
（变异回 `0.5` ⇒ 失败，报错里带原样示例行）· `check-handheld.mjs` 新增「两份 JS + 转译器同口径」12 项
（含 3 条变异自证：把模板那份改回 `|| 14` ⇒ 同口径与 falsy 两条判据一起翻面）。

**拟改法（等拍板）**：- R1：提示词示例改 `14` + 单位说明；加一条**按值迁移**（照 `process/mod.rs::migrate_camera_zoom` 先例）
  进 `process/fx.rs` 的 Step 0：`0 < intensity < 2 ⇒ ×14`（幂等、让存量工程当场恢复）；
  三处 `|| 14` 改成**数值判定**（`0` 是合法值，现在 JS 三条副本把它变 14px、转译器是 0 ⇒ 预览抖、成片不抖）。
- R2：**不改代码**（2026-10-02 用户口径）；可选把提示从 `title` 提到行内（"静默整类失效"值得明说）。
- R3：`:2881-2885` 右端铺到**下一个稀疏键前一帧**（新增 `nextSparse`）；改 `tools/` ⇒ 必跑
  `node scripts/check-yekitrim-translator.mjs --mutate` + `check-dhampir-drop-list.mjs`。
- R4：`:3718` 条件放宽到"**首帧没有键**"并补一帧 0 的中性基准键 —— **先确认底座"首键之前"的语义**
  （若是"首键保持"，只写 `layer.transform` 无效，必须写真键）。

## ④ 区域与落点 —— **定案 W1（源帧/主画面矩形 `sr` 百分比），施工中**

- 九宫格 **Rust `grid_rect`（`render-core/src/types.rs:752-770`）↔ JS `GRID_RECTS`
  （`engine.js:50-60`）9 格逐值一致** ✓ 这条排除。
- ⚠️ 但**格子中心是四份手抄值**：Rust `83.34%` / 模板硬编码表 `83.33%`（`index.html:829-837`）/
  engine 现算 `83.33500000000001%`（缺 `toFixed(2)`）/ 转译器 `5/6`。最大差 0.01%≈0.2px（无害），
  但它证明"GRID_RECTS 对齐"只对齐了**矩形**；**没有任何判据钉住中心点**
  （`check-handheld.mjs:140` 反而把 `gridOrigin/regionOrigin` **stub 掉**、把差异屏蔽了）。
- **根本冲突**：`layout.<区域>` 的**百分比坐标系没有定义** —— 编辑端按**整帧/舞台 %**记录
  （`PolishPreview.vue:2660-2670`），四个渲染端按**主画面矩形 sr 的 %**读
  （`index.html:928`/`:1888-1889`、`PolishPreview.vue:1961`、转译器 `:2599-2600`），
  而 `process/placement.rs` 又按**整帧**读 ⇒ 只要 `sr ≠ 整块画布`（源比例≠画布 / `scale≠1` /
  `padding≠0`）三方就不一致。
- 附带：**默认 origin 优先级三套**（`engine.js:356` 与模板 DOM `:846-854` 含自由选框与 `grid[0]`，
  而模板 canvas `:1427` **两者都不看**，转译器 `:1147-1170` 又补了自由选框）⇒ 实测最多**偏 288px**。
- **两条互斥改法（必须二选一）**：
  - **W1 源帧坐标**（忠于 `placement.rs:1-31` 的文档）：编辑器新增"量视频矩形"的换算
    （⚠️ **点选落点仍用舞台坐标**，是拆成两个换算、不是改 `pctAt`）+ Rust 加一份
    `stage_draw_rect` SSOT、`placement` 改为走 sr。画面几何零回归，但**改 Rust ⇒ 重建二进制**。
  - **W2 整帧坐标**：只改 4 处渲染端不动点，`placement` 不动；但**重解释所有存量 `origin`**
    ⇒ 成片运镜位置会变，必须真出片 A/B，且会顶翻 `check-stage-backdrop.mjs:363` 那条钉子。

## ⑤ 贴纸盖住区域 —— **已修 1 条（4c），其余待决**

| | 结论 | 数值 / 证据 |
|---|---|---|
| **4a 主因** | `process/placement.rs` 的贴纸避让模型**忽略 `stageDrawRect(sr)`**（`process/mod.rs:691-697`、`:585-592`、`placement.rs:163-170/300-318`；只有 `main_frame_geometry` 消费了 stage） | 4:3 源进 16:9、liver 框 10–40%、zoom=1：真障碍 `[384,816]` vs 模型 `[192,768]` ⇒ 临界候选**真压住主播 47px**；`[stage] scale=1.6` 时 **100% 落在真人身上**，`relaxed` 不触发、**零告警** |
| **4b** | `main_frame_geometry` 的 `scale×position` 合成与模板不同（先缩放再插值 vs 先定位再绕盒心缩放） | `pct=0/scale=2` 差 **303.75px**；既有单测只用 `pct=50` ⇒ 测不出 |
| **4c ✅ 已修** | **出片那条腿不钳安全区**（两条预览都钳） | manual `"98% 50%"`：不钳 **1741.6** → 钳后 **1608**（出框 101.6px）。修 `tools/polish-to-dhampir.mjs` + 把**转译器补进**贴纸落点契约（原是漏掉的第 6 份实现），带变异自证 |
| **4d（已降级）** | 贴纸/弹幕/字幕**三套叠放顺序**：出片=贴纸最上（转译器注释明写「**贴纸排在最后**是这一轮核出来的」，对齐 `renderFrame`）· 模板 DOM=弹幕压贴纸 · WebUI 预览=字幕弹幕都压贴纸 | ⚠️ **但默认预览里这条看不见**：承载那三层的 `.overlay-space` 是 `z-index:10`，而 Dhampir 画布是 **40**（`PolishPreview.vue:2812`）⇒ 画布把 DOM 那三层**整块盖住**，可见的叠放顺序来自**引擎**，而引擎按转译器轨道序 = **出片口径** ✓ 所以分歧只存在于**已禁用的旧 DOM 臂** ⇒ **降级为遗留问题，不是用户能看到的 bug**（别照着改 z-index —— 改了也看不见，还平添风险） |
| 4e | 显示框 ≠ 生效区域（多格工程里 `regionRect` 是包围盒，生效的是 `grid[0]` 中心/逐格障碍） | 用户在框选模式**不动任何东西直接保存**就会把语义改掉 |
| 4f | `_canvas.sticker.safe_margin` 是**死契约**（`config.rs:472`→`canvas_contract` 注入，全仓零消费者） | 真正的 32 在四处各写一份 |

## 判据缺口（本次暴露，全都没有覆盖）

1. 九宫格**中心点**四处手抄值（现在必不一致 0.01%）· 2. `regionOrigin ↔ region_origin` 的串与优先级 ·
3. 默认 origin 的优先级只有一套 · 4. `placement` 的障碍/不动点必须走 sr · 5. 转译器 manual 落点必须 clamp
（**本次已补**：`check-sticker-anchor.mjs`）· 6. 三层叠放顺序（模板/预览/canvas 三处）· 7. 区域的坐标系口径 ·
8. `intensity` 的单位 · 9. "shake 在 camera 存在时仍生效"（`check-yekitrim-translator.mjs`
`:769-771`/`:1284-1286` **明说不测 camera 行为面**）。

## 走过的弯路（留档，别重走）

- **② 我先判成"素材集合变了 ⇒ 旧 `<video>` 没有对应源"**，并据此加了 `dhampirAssetsKey` 指纹 +
  `stopDhampir()+startDhampir()` 重启分支。**错的**：转译器的视频素材是写死的
  （`videoAssetId='clip.mp4'`、`uri=config.source?.video`，`tools/polish-to-dhampir.mjs:1066-1072`），
  而 `cmd_llm` 只写 `segments`+`filter.*`（`commands.rs:354-358`）、`cmd_process` 只写
  `segments`+`intro_fade_seeded`+`processed`（`process/mod.rs:551-555`）⇒ **指纹恒等、是死代码**，
  且它**修不了**这个黑屏（警告：同一个 canvas 里 stop+start "能跑"，但 canvas 一旦被换过照样黑）。
- **③ 我先判成"两者相加就没问题"**。相加确实没错，但**单位漂移（R1）与竖屏双开关（R2）**
  才是主因；而"同时存在"命中的是 **R3**（密集键右端截断）。
- **口径过期纠正**：`templates/index.html` 现在只是**参照实现**（`tools/AGENTS.md`、
  `webui/src/polish/AGENTS.md` 都写明）；**出片与预览的活路径是转译器 + 底座**
  （`f9d7a50` 删 chrome 腿、`85d7fc0` 让底座成为默认出片通道）。所以"几份实现要对口径"实际是
  **转译器也要算一份**。

## 结论速览：哪条是真 bug、哪条是设计如此（2026-10-05 收口）

| 条目 | 判定 | 状态 |
|---|---|---|
| ① 改字幕后预览不刷新 | **真 bug**（字幕不在 `polish.toml` ⇒ 热重载指纹恒等） | ✅ 已修 `efeeb2a` |
| ② LLM 剪辑后黑屏 | **真 bug**（任务完成重挂预览；底座 attach 一页只接一次、无 detach） | ✅ 已修 `efeeb2a` |
| ③ R1 `intensity` 单位漂了三处 | **真 bug**（提示词示例 `0.5` vs 消费者按像素） | ✅ 已修 `efc5e0e` |
| ③ R2 竖画布 `camera`/`shake` 两个独立开关 | **设计如此**（2026-10-02 用户口径：整类门就是那个开关） | ⛔ **不改代码**；用户侧自检见上文 30 秒分流 |
| ③ R3 密集键右端封在窗口末尾 | **真 bug**（全仓唯一"有 shake 就改运镜曲线"的路径） | ⏳ 转译器改动施工中 |
| ③ R4 中性基准只写给 0 键层 | **未定**（取决于底座"首键之前"的语义，需底座侧证据） | ⏳ 未动 |
| ④ 区域/落点坐标系没定义（W1） | **真 bug**（编辑器按舞台写、四个渲染端按 `sr` 读） | ✅ 已修 `efc5e0e`（编辑器+Rust 两侧一起） |
| ⑤ 4a placement 忽略 `sr` | **真 bug**（实测临界候选压住主播 47px） | ✅ 已修 `efc5e0e` + 判据 `e23eec9` |
| ⑤ 4b `main_frame_geometry` 合成顺序 | **真 bug**（`pct=0/scale=2` 差 303.75px） | ✅ 已修（委托 SSOT） |
| ⑤ 4c 出片不钳贴纸安全边距 | **真 bug** | ✅ 已修 `4cc0756` |
| ⑤ 4d 三层叠放顺序三套 | **降级为遗留**（承载那三层的 `.overlay-space` z-index 10 < 画布 40 ⇒ 默认预览里看不见） | ⛔ 不改（改了也看不见，只添风险） |
| ⑤ 4e「框选模式不动任何东西直接保存」会改语义 | **真 UX 风险**（显示框 = 包围盒，生效的是 `grid[0]`/逐格障碍） | ⏳ 未动，需求待定 |
| ⑤ 4f `_canvas.sticker.safe_margin` 零消费者 | **死契约**（无害） | ⏳ 未动 |

⚠️ **竖素材 `CFG.vertical` 吃掉运镜**（任务书里点名的既有口径）**与用户报的 ③ 不是同一件事**：
`getShake` 一个字段都不读 `camera`，shake 的 `ctx.translate` 在 camera 块**之外**、不被 `camScale` 缩放
（实测 zoom=1.5 时仍是 `translate(42px,-42px) scale(1.5)`），所以竖屏门控**吃不掉 shake** ——
这条在 §③ 开头已用数值证伪。

## 剩余缺口与配方（留给下一轮，2026-10-05 收口时记）

1. **③ R3 的专属判据（唯一还缺的"能翻面的用例"）** —— 修已落 `9cd26c3`，三条既有守卫
   （translator ✓ / `--mutate` 24 处全红 ✓ / drop-list 清单不变 ✓）都过，但**这个行为本身没被判据钉住**
   （`denseRanges` 是主流程里的内联计算，守卫看不到它）。配方：
   - 把 `tools/polish-to-dhampir.mjs` 里 `prevSparse` / `nextSparse` / `denseRanges` 那段抽成
     **顶层 `export function denseRangesFor(spans, sparseSorted, lastOut)`**（主流程改成调它）；
   - 在 `scripts/check-yekitrim-translator.mjs` 加 `checkDenseRanges(mod)`，至少三条：
     ① 窗口 `[a,b]` + 下一个稀疏键在 `b+5` ⇒ 右端 = `b+4`（**不是 `b+1`**）；
     ② 没有下一个稀疏键 ⇒ 右端 = `lastOut`；③ 下一个稀疏键恰是 `b+1` ⇒ 右端不小于 `b+1`；
   - ⚠️ **抽成函数并在 `mutate()` 里一起调**（`scripts/AGENTS.md` 明写：只写在顶层的检查，
     变异台根本不跑它 ⇒ 那条变异"没被打红"）；再往 `MUTATIONS` 加一条把右端改回 `b + 1` 的变异。
2. **真出片烟测**：本批改了 `templates/index.html`（3 行：shake 强度取默认），
   `templates/AGENTS.md` 要求"改了模板必须真跑一次带字幕的出片"—— 还没跑。
3. **⑤ 4e**：真 UX 风险（框选模式不动任何东西直接保存会把"包围盒"写成"自由选框"），**要不要修待用户定**；
4. **③ R4**：中性基准只写给 0 键层 —— 需要底座"首键之前"的语义证据才能定性（`③ R4` 那条）；
5. **⑤ 4f**：`_canvas.sticker.safe_margin` 零消费者（无害，清理即可）。
