# 底座需求：字幕缩放的**双峰脉冲**（`heartbeat` 档｜T8）

> 提出方：YekiTrim（本仓）｜ 日期：2026-10-14 ｜ 对应：用户口径
> **「**动效**需要增加一个心跳，扑通扑通的心跳感」**
> 关联：`docs/plan/dhampir-elements-request.md`（T1/T3/T4/T6 那一批，已交付）
> 保守决策沿用 #125–#128、#134–#138

## 一句话

**T1 那条 `back_out3` 曲线数学上只有一个极值点，做不出「扑通**扑通**」的两个峰。**
本仓（YekiTrim）**预览侧已经做完真双峰**；**成片侧还差底座一个新原语**。

⚠️ 在底座落地之前，本仓**如实报 NOTE**（转译器里那条 `effectAnimRaw === 'heartbeat'`）——
**不静默退化**：报告里明写「预览是真双峰、成片目前只有一次过冲」。

---

## 一、为什么现有原语做不到（不是参数问题）

底座 `crates/dhampir-timeline/src/text_motion.rs::back_out3`：

```rust
pub fn back_out3(t: f32) -> f32 {
    let s = 3.0;
    let u = t - 1.0;
    u * u * ((s + 1.0) * u + s) + 1.0
}
```

它是**单次过冲**：导数在 `[0,1]` 内**只穿零一次** ⇒ 曲线**只有一个**极值点。

而「扑通扑通」要的是 `缩放` 在一个周期里出现**两个**峰
（`咚 · 咚 ……` 第二声比第一声轻）。**单峰曲线表达不了双峰** ——
这不是 `from` / `peak` / `ms` 三个参数没调好，是**函数形状不对**。

**可执行的证据**（本仓判据 `scripts/check-preview-anim.mjs` ⑤-bis 里也钉着这一条）：
对 `back_out3` 逐千分位扫局部极大值，**恰好 1 个**；而心跳曲线是**恰好 2 个**。

---

## 二、本仓已经做好的部分（可直接抄过去对拍）

`webui/src/polish/engine.js::heartbeatScaleAt`（纯函数、无累积状态）：

```
一个周期 cycle_ms 内：
  p1           第 1 峰（用 back_out3 从基线冲到 amp，fall_ms 内回落）
  p1 + fall    回到基线
  p2           第 2 峰（**弱 30%**：1 + (amp-1)·0.7）
  p2 + fall    回到基线
  其余         恒为 1.0（**停顿** —— 这就是「扑通……扑通」中间那口气）
```

**参照参数**（本仓 `ANIM_PRESETS.heartbeat`，两侧同值）：

| 参数 | 值 | 含义 |
|---|---|---|
| `peaks_ms` | `[0, 180, 300, 480]` | 第 1 峰起 / 回落完 / 第 2 峰起 / 回落完 |
| `amp` | `1.14` | **第 1 峰**的倍率（第 2 峰自动弱 30% ⇒ 1.098） |
| `cycle_ms` | `1000` | 一个周期；余下 **520ms 是停顿** |

**实测输出**（本仓逐毫秒扫，不是估算）：

```
峰位置(ms) = [90, 270]      峰值 = 1.1750 / 1.1225     ← 两个峰，第二峰更弱
停顿处(700ms) = 1.0
```

> ⚠️ 注意 `amp` 与实测峰值不同：`back_out3` **本身过冲到 1.25**，
> 所以实际峰值 = `1 + (1.14-1)·1.25 ≈ 1.175`。这与 T1 的
> 「`scale_in_peak` 是**端点**不是峰值」是**同一个口径**，别搞成两套。

---

## 三、建议的接口形状（两个选项，任选）

### 选项 A（**推荐**）：加一个"脉冲曲线"字段，把节奏当数据

在 `SubtitleStyle` 上新增一组（沿用 T1 那一组的 `Option` + `skip_serializing_if` 口径）：

```rust
/// **心跳的双峰脉冲**（T8）。`None` = 关（**不缩**）。
///
/// ⚠️ 与 `scale_in_*` **互斥**：两者都非 `None` 时，**本字段优先**
///    （心跳是"更具体"的意图；两个一起给是宿主写错了，不该静默任选一个）。
#[serde(default, skip_serializing_if = "Option::is_none")]
pub heartbeat_amp: Option<f32>,          // 第 1 峰倍率，参照 1.14
#[serde(default, skip_serializing_if = "Option::is_none")]
pub heartbeat_cycle_ms: Option<u64>,     // 一个周期，参照 1000
/// 两个峰的起点（相对周期起点，毫秒）。参照 `[0, 180, 300, 480]`。
/// ⚠️ 只要前两个就够（第 2 峰起点 + 回落时长可由它们导出），
///    但**照抄四个**可以让"两峰不等宽"将来也能表达。
#[serde(default, skip_serializing_if = "Option::is_none")]
pub heartbeat_peaks_ms: Option<Vec<u64>>,
```

求值放进现有的 `text_envelope_scaled` 旁边，**同一个 `(opacity, offset, scale)` 出口**：

```rust
let scale = if span_ms > 0 && let Some(amp) = heartbeat_amp { /* 见 §二 的曲线 */ }
            else if /* …… 现有的 scale_in 分支 */ { … }
            else { 1.0 };
```

**优点**：节奏是**数据**不是新函数 ⇒ 以后要「三峰」「不等宽」都不用再加原语；
且两侧（YekiTrim 预览 / 底座成片）**照同一组数字求值**，对拍容易。

### 选项 B：只加一个曲线枚举

`scale_curve: Option<String>` ∈ `{"back_out3", "heartbeat"}`。
**不推荐**：节奏写死在底座里，本仓要改节奏就得再来找底座一次；
而且本仓预览那份实现已经按"节奏是数据"写好了（`heartbeatScaleAt` 收 `peaksMs`）。

---

## 四、验收标准（本仓会照着验）

1. **双峰**：一个周期内 `scale` 的**局部极大值恰好 2 个**；
2. **第二峰更弱**：`peak2 < peak1`（等高看着像**抖动**，不像心跳）；
3. **停顿**：两峰之外**恒为 1.0**（不是停在峰上 —— 与 T1 那条"窗口外回 1.0"同一纪律）；
4. **周期**：`t` 与 `t + cycle_ms` **逐值相同**；
5. **纯函数**：同一 `local_ms` **永远同一值**（无累积状态 ⇒ 任意 seek 都一致）；
6. **缺省不动**：三个字段都不写 ⇒ **一个键都不多**、`scale` 恒 1.0（R4）；
7. **版本号**：返回体加键 ⇒ `HOST_API_VERSION` **7 → 8**；本仓 `DHAMPIR_HOST_API` 同步。

第 1~5 条本仓判据 `scripts/check-preview-anim.mjs`（⑤-bis，9 条）已经钉在**预览那一侧**；
底座落地后把同一组断言搬到 `text_motion.rs` 的单测里，**两边逐值对拍**即可。

---

## 五、⚠️ 本仓不会做的事（避免两边同时改）

* **本仓不先删 `scale_in_*`** —— 拿掉的话，底座落地前成片里心跳**一点都不动**
  （比"不完整"更坏）。所以 `ANIM_PRESETS.heartbeat` 里 `scale_in_*` **留着**，
  作为"底座能力到位前的最低限度观感"（= 单次过冲）。
* **本仓不改 `DHAMPIR_HOST_API`** —— 那是底座升版本号之后的事（顺序反了会让
  预览整个不可用，见 `dhampir-elements-request.md` 的「换产物这一步」）。

⇒ 底座做完之后**回一条回执**（照 `dhampir-elements-request.md` 里 T7 那张表的格式），
本仓再：① 改 `DHAMPIR_HOST_API`；② 删掉那条 NOTE；③ 把 `scale_in_*` 换成 `heartbeat_*`；
④ 两侧逐值对拍。
