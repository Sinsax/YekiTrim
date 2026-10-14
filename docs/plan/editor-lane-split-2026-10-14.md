# 前端编辑器：把「元素」轨拆成「文字 / 贴纸」两条

> 用户口径（2026-10-14）：「**前端编辑器也要分离**」（承「分离贴纸和文字层」）

## 一、为什么拆

磁盘上 `element` 与 `sticker` 是**同一条轨**（`Lane::Sticker` ⇒ `"sticker"`，
四处 `LANE_OF_TYPE` 镜像 + `templates/index.html` 都这么映射）—— **那个不能改**：
改了就要数据迁移，而老工程里全是 `type:"sticker"`。

但**界面上**它们该是两条轨，因为载荷 / 属性面板 / 能选的东西**完全不同**：

| | 文字 | 贴纸 |
|---|---|---|
| 载荷 | `text` / `style` / `style_anim` / `effect_anim` / `color` | `text`（包名）/ `sticker_tag` / `source` |
| 面板 | 内容 + 颜色 + 两条轴 + 排版角度 | 贴纸选择器 |
| 落点 | `x`/`y`，由 `process` 按安全落点算 | 同左 |

混在一条轨上的**代价**：那个「＋」必须弹一个**「文字 / 贴纸」二选一菜单** ——
而用户本来就是在**某一条轨**上点的加号，那个选择是多余的。
更糟的是它允许 `setElementKind` **把一条贴纸改成文字**，于是块会**悄悄换到另一条轨上**。

## 二、怎么拆（界面分家 / 磁盘不分家）

```
{ id: 'text',    type: 'sticker', kinds: ['text', ''], sharedEye: true, label: '文字' }
{ id: 'sticker', type: 'sticker', kinds: ['sticker'],                   label: '贴纸' }
```

* `type` **两条都是 `'sticker'`** —— 它们磁盘上确实是同一条轨（`lane` 键也照旧写 `sticker`）；
* `kinds` 才是**界面上的那一刀**（`laneBlocks` 里过滤）；
* `id` 是**界面唯一键**（`laneKey` 用它），因为 `type` 不再唯一。

⚠️ **`kinds: ['text', '']` 里的空串不能漏**：`kind` 缺失/为空是最常见的存量形状
（Rust 侧 `#[serde(default)]` 兜空串、面板新建的也是它）—— 漏了它那些元素会在
**两条轨上都看不见**，而**不报错**。

## 三、⚠️ 两个键**不是一回事**，混用都是**静默**故障

| 键 | 是什么 | 用在哪 | 混用的症状 |
|---|---|---|---|
| `laneKey(lane)` | 界面 id（`lane.id \|\| lane.type`） | `v-for :key`、选中态判等 | 用 `laneTypeOf` ⇒ 两条轨**共用 key** ⇒ Vue 复用错节点 |
| `laneTypeOf(lane)` | 轨道名（`lane.type`） | `emit('lane-open')` | 用 `laneKey` ⇒ `PolishView` 查不到 `laneGroupEls.text` ⇒ **点轨道名没反应** |

⚠️ 第二条我**差点就踩了**：`laneGroupEls` 是按**轨道名**索引的
（`laneGroupEls.sticker = el`）。发界面 id 不会报错，只是**什么都不发生**。

## 四、⚠️ 两枚眼睛管同一个开关 —— 只留一枚

两条轨的 `lane` 都是 `'sticker'` ⇒ 都画眼睛就是**两枚按钮管同一个
`[stage.lanes].sticker` 开关**：点哪一枚另一枚都跟着变，用户看到的是
「我只关了文字，贴纸怎么也没了」。

⇒ 眼睛留在**贴纸**轨（开关名本来就是 `sticker`），文字轨放一枚**不可点的占位**（`⇄`），
`title` 说明"两条是同一个开关"。

## 五、⚠️ 泳道数 7 → 8：**这一条需要用户确认**

时间轴的泳道高度是 **26px/条**（`webui/src/components/AGENTS.md` 明写
「**布局必须保持紧凑**：泳道 26px ×7 条 + 标尺 20px = **202px**」）。

拆成两条之后是 **8 条 ⇒ 8×26 + 20 = 228px**（**+26px**）。
总高是 `flex:1` **不是写死的**，所以**不会破版**；但与卡上那条"保持紧凑"的口径**不一致**。

**三个选项**（等用户定，我先按①做）：

1. **就 +26px**（现状）—— 最简单，一行泳道换掉一个二选一菜单；
2. **把文字/贴纸合并回一行显示**（两块并排画在同一条轨的高度里）——
   ⚠️ 要改 `laneBlocks` 的 `width`/`left` 算法（两块会重叠），**不是小改**；
3. **缩泳道到 23px**（8×23+20 = 204px）—— ⚠️ 会动到 `WAVE_H` 与波形 SVG 的高度
   （卡上写明"改 26px 时要同步 `WAVE_H`"），且波形动态范围本来就只有 6.6px，**会更小**。

⛔ **不要**在没定之前自己选②或③：两个都碰到别的已标定数字。

## 六、判据

`scripts/check-editor-lane-split.mjs` —— **27 项**，五组：

* ① 两条轨、`lane` 都是 `'sticker'`、`kinds` **不许重叠**、文字轨**必须含空串**；
* ①b 眼睛只给贴纸轨（`sharedEye`）；
* ② `laneKey` / `laneTypeOf` 定义得对；
* ③ `:key` 不许用 `laneTypeOf`、`emit` 不许用 `laneKey`；
* ④ 两个 ＋ 各自直接建对应 kind；**二选一菜单不许回来**；
* ⑤ ⚠️ `elementKindOf` 认不出时必须是 `'unknown'`，**不许默认成 `'text'`**
  （默认成文字 = 把坏数据伪装成正常元素），面板三个分支**各自显式认领**、不许 `v-else` 兜。

**六处变异全翻面**：合并回一条 / `emit` 发 `laneKey` / `:key` 发 `laneTypeOf` /
`unknown` 默认成 `text` / 去掉 `sharedEye` / 眼睛不排除 `sharedEye`。

⚠️ 顺带修好 **`check-layout-invariants.mjs` 的 ⑰**：它那条变异锚在
`lane.type` 上，源码改成 `laneTypeOf(lane)` 之后**变异静默失效**
（`replace` 找不到东西 ⇒ 源码没变 ⇒ "判据翻面"那条失败）。
⇒ 锚点过时的症状是**变异不生效**，不是"找不到锚点" —— 这条教训本身值得记。

## 七、验证

```
check-editor-lane-split（新）  ✓ 27 项 · 六向变异全翻面
check-layout-invariants        ✓ 全部通过（⑰ 的变异锚点已跟着改）
scripts/ + webui/scripts/ 全部 ✓ 全绿
两条结构门禁                    ✓
npm --prefix webui run build   ✓ dist 指纹 4487dd8255739b33
```
