# YekiTrim Logo 提示词集（供图像生成 AI 使用）

> 用途：拿去 Midjourney / DALL·E / Stable Diffusion / 即梦 / 可灵 等生成正式 logo
> 来源：三稿几何与配色均为**实测立绘特征色**派生（反查 RGB 距离 ≤ 3.2）
> 日期：2026-10-03

---

## 0. 通用约束（每个提示词都要带）

**品牌**：YekiTrim（直播录像切片工具，界面叫"切片工作台"）

**必须遵守的硬约束**（否则生成的图没法用作 logo）：

| 约束 | 值 | 原因 |
|---|---|---|
| 输出尺寸 | 1024×1024，1:1 | 要缩到 16×16 favicon |
| 背景 | **纯透明或纯白**，不要场景元素 | logo 要贴在任何底色上 |
| 主体占比 | 高度 70–80%，四周留白 | 缩到 16px 才不糊 |
| 描边 | **粗、简洁、无渐变** | 16px 下渐变会成噪点 |
| 颜色数 | **≤ 3 色** | 同上 |
| 细节 | **禁止**：文字、字母（除 Y）、阴影、纹理、光晕、3D 立体感 | 缩到 16px 全消失 |
| 风格关键词 | flat vector / minimal geometric / logo mark / icon design | — |

**品牌三色**（可直接贴进提示词）：

```
砖橙 #E86838  （主色，取自立绘橙瞳）
裙黑 #202020  （骨架，取自立绘女仆装）
裙白 #F0F0E0  （暗底版骨架，取自立绘女仆装）
蝙蝠红 #F86860（点缀，取自立绘肩上蝙蝠伴宠的瞳）
```

**通用负面提示词**：

```
text, letters, words, watermark, signature, photorealistic, 3d, bevel,
drop shadow, glow, gradient mesh, noise, texture, complex details,
multiple logos, frame, border, background scenery
```

---

## ① 瞳（Y 的交点嵌入一只橙瞳）

**概念**：Y 字的交叉点处嵌入一只橙瞳。角色是橙瞳设定，眼睛是签名；Y 骨架保留品牌识别。

### 浅底版

```
Minimal flat vector logo mark, capital letter "Y" drawn with thick rounded
strokes in near-black #202020, at the exact crossing point of the Y sits a
single stylized eye: circular iris in brick-orange #E86838, solid black pupil
in the center, one small white highlight dot at the upper-left of the iris.
Geometric, symmetrical, no gradients, no outline text, thick even line weight.
White background, centered, 70% of canvas height, generous margins.
flat vector, minimal geometric, logo mark, icon design, 1:1
```

### 暗底版

```
Same composition as above but the letter "Y" strokes are warm off-white
#F0F0E0 instead of black, on a solid near-black #202020 background.
The orange eye #E86838 stays identical: black pupil, white highlight dot
at upper-left. flat vector, minimal geometric, logo mark, icon design, 1:1
```

> 💡 **要点**：告诉 AI"瞳在 Y 的**交点**上"，不是"Y 里面" —— 否则会画成 Y 的肚子里有个圆。

---

## ② 蝠（Y 两臂展成蝙蝠翼）

**概念**：Y 的两臂展开成蝙蝠翼（尖耳 + 三道鳞波凹口），竖干是她的身体，
翼上嵌两颗红瞳 —— 直接取自立绘肩上那只蝙蝠伴宠。

### 浅底版

```
Minimal flat vector logo mark: a capital "Y" whose two upper arms extend
outward and transform into bat wings. Each wing has a pointed ear at the top
and three scalloped notches along the outer lower edge. The vertical stem of
the Y reads as the bat's small body. Two tiny dot eyes in bat-red #F86860
sit symmetrically on the wings. Entire silhouette in near-black #202020,
solid fill, no gradients, no internal lines.
White background, centered, 70% canvas height, generous margins.
flat vector, minimal geometric, silhouette, logo mark, icon design, 1:1
```

### 暗底版（剪影感最强，推荐主用）

```
Same bat-wing "Y" mark, silhouette in warm off-white #F0F0E0 on solid
near-black #202020 background. The two red eyes #F86860 are the only color
accent. Solid fill, no gradients, no outline. This version has the strongest
silhouette read. flat vector, minimal geometric, logo mark, icon design, 1:1
```

> 💡 **要点**：用户已确认"**蝠翼加宽加深鳞波**，16px 也能认出是蝙蝠剪影"。
> 所以提示词里"three scalloped notches"（三道鳞波凹口）和"pointed ear"（尖耳）**不能省** ——
> 少了这两个特征，16px 下就只是个普通的 Y。

---

## ③ 结（哥特蝴蝶结）

**概念**：哥特蝴蝶结 —— 双环 + 两条燕尾飘带（她裙摆与鞋上的缎带形态），
结心嵌一颗橙珠，呼应胸前的蜘蛛饰位置。**不依赖 Y 字，独立成立。**

```
Minimal flat vector logo mark: a gothic ribbon bow. Two symmetric rounded
loops that are TALL rather than wide, with a small gathered pinch at the
center. Below the loops hang two long trailing ribbon tails, each ending in
a notched swallowtail point. At the center knot sits a small circular bead
in brick-orange #E86838 with one white highlight dot. The whole bow is solid
near-black #202020, no gradients, no internal lines.
White background, centered, 70% canvas height, generous margins.
flat vector, minimal geometric, gothic, logo mark, icon design, 1:1
```

### 暗底版

```
Same gothic bow mark in warm off-white #F0F0E0 on solid near-black #202020
background, with the center bead in orange #E86838.
flat vector, minimal geometric, gothic, logo mark, icon design, 1:1
```

> ⚠️ **这一稿最容易画错**：实测原设计稿的双环**高仅 16px、宽 23px**（比例接近水平）
> ⇒ 生成出来是**领结（bow tie）**，不是裙摆蝴蝶结。
> 提示词里必须强调 **"loops that are TALL rather than wide"**（环要高不要宽）
> 和 **"long trailing ribbon tails"**（长燕尾飘带）。
> 我已按这个改法修过设计稿，效果见 `bow-variants.png`。

---

## 交叉参考：三稿对比

| 稿 | 是否需要"Y" | 16px 表现 | 适用场景 |
|---|---|---|---|
| ① 瞳 | 需要 | 橙瞳 + Y 骨架都清晰 | 强调"品牌延续性" |
| ② 蝠 | 需要（Y 变形） | **暗底剪影最强** | 强调"角色气质"，暗色界面 |
| ③ 结 | **不需要** | 清晰（修几何后） | 独立图形，最"logo" |

**配色建议**：三稿都遵循「**骨架用裙黑/裙白，只用橙色点睛**」——
这与界面的「黑白为骨 + 橙瞳点睛」是同一套逻辑。

---

## 附：如果你想手绘微调

三稿的**可编辑 SVG 源码**已落在实现里，可直接拿去改：

| 位置 | 内容 |
|---|---|
| `webui/index.html` | favicon（当前是 ①瞳 的简版） |
| `webui/src/views/LoginView.vue` | 登录页 .logo 的 SVG |
| `webui/src/components/side/SideNav.vue` | 侧栏 .brand-logo 的 SVG |

几何坐标都是 64×64 viewBox，笔画粗细 6–7，端点 `stroke-linecap="round"`。
