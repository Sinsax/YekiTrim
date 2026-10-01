# wgpu 渲染效果参考（**历史**：那条 wgpu 后端已删除）

> ⚠️ **这里记的是一条已经删掉的路径**：V-Trim 自带的 wgpu 原生渲染后端（`vtrim-render-core` 的 wgpu
> 合成管线 + `vtrim-polish/src/render/native/` + `VTEDIT_RENDERER=wgpu`）连同浏览器真渲染预览
> `vtrim-wasm` 已整体删除 —— GPU 那侧交给独立的 **Dhampir 底座**。所以本目录只剩**参照价值**：
> 它是"同一 demo 工程经两条后端渲染"的同帧对比，可以当作**特效观感/边缘情况**的视觉参照，
> **不是**当前出片行为的说明。当前出片路径只有一条：`templates/index.html` 的 canvas（headless Chrome）。

wgpu 原生渲染后端 vs Chrome headless 基准的同帧对比。两份成片由**同一 demo 工程**分别经 `VTEDIT_RENDERER=wgpu` 与 `VTEDIT_RENDERER=chrome` 渲染生成。

## 文件

| 文件 | 说明 |
|---|---|
| `demo/` | 效果参考工程：12s 源视频 + 覆盖全部特效的 polish.toml + 字幕（含 hl 高亮）+ 弹幕 + 贴纸资源 |
| `demo/wgpu_export.mp4` | wgpu 后端出片 |
| `demo/chrome_export.mp4` | Chrome 后端出片 |
| `compare_<t>.png` | 12 组同帧并排对比（左 wgpu / 右 chrome），每个特效时刻一组 |
| `effect_reference_sheet_1/2.png` | 对比图合订（3 组/张） |

## 特效时刻表（demo 时间轴）

| 时间 | 特效 |
|---|---|
| 0.3–1.8 | camera 近（zoom 1.8） |
| 0.5–2.5 | sticker GIF（问号）+ overlay 渐变（linear 多 stop） |
| 2.5–2.8 | shake |
| 3.0–3.34 | flash 闪白 |
| 4.2–4.6 | blur 失焦（wgpu 高斯 / Chrome CSS blur） |
| 4.8–5.4 | color_shift sepia→正常 衰减 |
| 5.4–5.8 | noise 白噪 |
| 5.8–6.2 | zoom_bounce |
| 6.4–7.0 | pulse |
| 7.0–7.6 | vignette 暗角 + camera 中 |
| 7.6–8.55 | hue_shift 色闪 |
| 8.6–8.9 | stutter 抖动 |
| 9.0–9.12 | split（skew 剪切） |
| 9.5–10.7 | overlay 纯色 |
| 9.8–11.3 | sticker PNG（静态） |
| 0–11.5 | 字幕（hl 高亮 span）+ 弹幕滚动 |

## 结论

12 组同帧对比：**wgpu 与 Chrome 出片逐效果一致**（贴纸位置/动画、字幕字体/描边/高亮、渐变几何、blur/color_shift 观感、弹幕滚动、intro black、运镜）。

已知差异（可接受）：
- 字幕字体两者均回退系统字体（本机未安装 LXGW WenKai 字体文件时）
- emoji 码点贴纸无本地资源时 wgpu 跳过（离线不回退远程 CDN）

## 重新生成

```bash
# demo 双渲染
VTEDIT_RENDERER=wgpu   cargo run -p vtrim-cli --features wgpu-render -- polish render docs/wgpu-effect-reference/demo
VTEDIT_RENDERER=chrome cargo run -p vtrim-cli -- polish render docs/wgpu-effect-reference/demo

# 采样帧（手动）
VTEDIT_SAMPLE_DIR=... VTEDIT_SAMPLE_OUT=... \
  cargo test -p vtrim-polish --features wgpu-render --test render_sample -- --ignored --nocapture
```

渲染性能：12s@30fps 全片 wgpu 17s / chrome 16s（4 chunk 并行）。
