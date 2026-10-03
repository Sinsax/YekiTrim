# 交给 Dhampir 底座仓的问题清单（YekiTrim 侧整理）

> 立档：2026-09-30（M4）。**本仓不改对方代码**，只把"最小复现 + 证据 + 影响"整理成一页发过去，
> 并在 `docs/plan/dhampir-parity-register.md` §四 跟状态（对方修没修、我们何时能去掉对应的 CLAMP）。
>
> 三条的**共同点**：都不是"YekiTrim 传错了字段"，而是**底座拿到的字段没被用上 / 文档与实现不一致**。
> 判定方法（本仓台账 §一）：先看转译报告，再**绕开 YekiTrim** 直接用底座自己的命令渲染同一份 doc。

---

## D1（重要）`overlay` 渐变色罩不出图

> ✅ **2026-10-01 已撤回**：原复现 JSON 无效（缺 `timeline.schema`；`end` 超素材帧数）——
> 底座在 `c7f29b9` 复现不出来，YekiTrim 侧按同形状复核也是"正常出图"（窗口内平均差 39.33 /
> 窗口外逐字节相同）。本节保留作历史；现行状态以 `docs/plan/dhampir-parity-register.md` §四 为准。

**影响**：任何用了 `overlay_type = "gradient" / "color" / "fade_in" / "fade_out"` 的工程，
在底座出片里**这一层完全没有**（片头渐入、色罩、暗角式的氛围层都属于这一类）。
YekiTrim 的默认出片通道就是底座 ⇒ 用户看到的是"这块效果没了"。

### 最小复现（自包含，用仓库自带的演示工程）

```bash
# 1) 转译（演示工程里就有 overlay 事件：0.5–2.1s 的 gradient、9.5–10.7s 的 color）
node tools/polish-to-dhampir.mjs docs/wgpu-effect-reference/demo --keep-muted --out /tmp/ov.doc.json

# 2) 只出那一帧（60fps 时间基 ⇒ 帧 60 = 1.0s，落在 gradient 的窗口 30..126 之内）
<Dhampir>/target/release/dhampir frame --project /tmp/ov.doc.json --frame 60 \
  --out /tmp/f60/ --asset-root docs/wgpu-effect-reference/demo

# 3) 期望 vs 实际
#    期望：画面上叠一层 #140A28CC → #28143CCC 的渐变（amount = 0.2 × 0.8 = 0.16）
#    实际：**没有任何色罩**（与不带这条事件时逐像素相同）
```

再缩到"只有视频层 + 一条 overlay 调整层"（其余轨全删）**同样不出**，
所以与其它轨/其它事件无关。

### 我们已核对过的（说明"不是传错字段"）

| 项 | 事实 |
|---|---|
| 效果名与参数 | 发的是 `{"kind":"overlay","params":{"amount":0.16,"r":…,"g":…,"b":…,"r2":…,"g2":…,"b2":…,"shape":1,"angle":0}}` —— 与底座**自己的**映射代码读的字段完全一致（`crates/dhampir-core/src/render/timeline.rs:362` 读 `amount/r/g/b/r2/g2/b2`），也与它自己的单测形状一致（同文件用 `effect("overlay", &[("amount",1.0),("angle",180.0),("shape",1.0)])`） |
| 图层 | 调整图层（**无 `source`**，`effects:[…]`，`keyframes` = 逐帧强度包络），挂在**视频轨之后**的那条轨上 |
| 窗口 | `start=30, end=126`（覆盖被取的那一帧） |
| 效果强度 | `Effect.opacity` 缺省 1.0（`dhampir-timeline/src/schema.rs` 的 `default_opacity`）⇒ `weight = amount × strength = 0.16 > 0` |

### 可能的方向（供参考，未验证）

1. **调整图层（无素材）的效果有没有进 ColorMask 那一趟**：`color_mask_params()` 是从**哪些图层**收集效果的？
   只有"有素材的图层"被扫的话，`overlay/flash/vignette/noise` 这类调整层就全都拿不到。
   （旁证：`flash` / `hue_shift` / `noise` 我们这边也走同一族调整图层 —— 如果它们**也没生效**，
   那就是同一条根因；如果它们生效，说明只差 overlay 这一支。）
2. **强度包络**：`strength` 取自 `effect.opacity`，而我们把逐帧曲线写在**图层**的 `keyframes`
   （`target: "opacity"`）上 —— 若那一趟只读效果的 `opacity`、不读图层关键帧，则 `amount` 是常量 0.16
   （仍应可见，所以这解释不了"完全不出"）。
3. `OverlayShape::from_param(1.0) = Linear` ✓ 取值没错。

### YekiTrim 侧的姿态

**不做近似补偿**（例如拿 `flash` 凑色罩）—— 那会掩盖问题、也会在底座修好后变成第二份实现。
转译侧已经把它**正确发出**，报告里没有 DROP；只有一条台账记录（§四 D1）。

---

## D2（小）`--from` 单独给出时，文档与代码不一致

`crates/dhampir-worker/src/bin/dhampir.rs::frame_range` 的注释写着：

> * 只给 `--from 10`："从第 10 帧起，到工程结尾"；
> * 只给 `--to 5`："从第 0 帧到第 5 帧"。

而代码是：

```rust
let from = args.from.unwrap_or(0);
let to = args.to.unwrap_or(from);   // ← 只给 --from 10 时 to = 10 ⇒ 只出第 10 帧
```

**影响**：信了注释的调用方会拿到"一帧"而不是"到结尾"。YekiTrim 两头都给（`--from 0 --to N-1`）所以没踩到，
但这是个**会静默出错**的不一致（结果看着成功、只是短得离谱）。
两条路都行：改注释，或把 `to` 缺省成"工程结尾"（注意另一头的不对称是**有意的**，别把 `--to` 也改成对称）。

✅ **2026-10-01 底座已改实现**（`frame_spec` + 纯函数 `resolve_frames`：只给一头按工程补全；
互斥用法报用法错，并加了 CLI 判据 + 变异验证）。本仓两头都给参数，**无需改代码**。

---

## D3（小）`DanmakuSpec.duration_ms` 的模块头注释与实现相反

`crates/dhampir-core/src/…/danmaku.rs` 的模块头注释描述的语义与 `layout` 实现相反（谁优先、谁覆盖谁）。
YekiTrim 侧没用到那段逻辑，所以只是"顺手记一笔"——**改注释或改实现之前先确认哪个是事实**。

✅ **2026-10-01 底座已按实现补全注释**（`travel_ms` 优先 → 回退轨道级 `duration_ms`）。

---

## 自包含交接单（推荐直接用它）

**`docs/dhampir-base-handoff.md`** 是本文件的"可直接贴给对方会话"的版本：不依赖 YekiTrim 仓，
每条都带"症状 / 影响 / 最小复现 / 已排除项 / 可疑点（带 file:line）/ 验收判据"，
并且把最小 doc 内联成了 JSON（不需要本仓的演示工程）。本文保留作历史与细节。

⚠️ 状态以 `docs/plan/dhampir-parity-register.md` §四 为准（2026-10-02 对齐，此前这里有一段
过期的重复粘贴）：**D1 已撤回**、**D2 底座已改实现**、**D3 底座已补全注释** —— 三条全部关闭；
底座侧当前唯一开着的是 handoff **§0b（背景层被收下但没画出来）**，YekiTrim 侧的
`BACKDROP_ENABLED` 开关在它查清之前保持关闭。

## 跟进约定

1. 对方修好后：本仓**先**用 M1 的对拍样本复测（`node .workbuddy/run-parity.mjs demo`），
   确认 D1 修好之后 overlay 的像素确实变了（基线会动 —— 那是**期望的**变化，要把三个样本的基线重新标定）；
2. 然后更新台账 §四（划掉该条）与 `docs/yekitrim-integration.md` §4.8 的对应行；
3. D2 修好后本仓**不需要改代码**（我们两头都给参数）；只需要把 `tools/AGENTS.md` 里"底座约定"那几句核对一遍。
