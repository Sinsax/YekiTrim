# ASR 时间戳对齐原型资产

从 `%LOCALAPPDATA%\Temp\opencode\asrtest` 移入仓库，用于复现 ROADMAP 中
“qwen3 文本 + sensevoice 时间轴”双模型对齐方案。

## 文件说明

- `align_srt.py` — Python 原型：LCS 锚点 + 锚点间局部 DP + SRT 断句。
  用法：
  ```bash
  uv run --with sherpa-onnx==1.13.3 --with numpy python align_srt.py <wav> <out.srt>
  ```
- `results.json` — 5 模型 × 7 音频的 CER/RTF 回归基准。
- `clip_aligned.srt` — 68.8s 直播片段的人工确认/原型输出参考。
- `wavs/` — 测试音频，包含 `clip_full.wav` / `clip30.wav` 等回归素材。

## 依赖模型

运行 Python 原型需要本地模型：

- `~/.vtrim/models/qwen3-asr/`
- `~/.vtrim/models/sensevoice/`

## 回归基准

Rust 实施完成后，应使用 `wavs/clip_full.wav` 对照 `clip_aligned.srt`，
并补充 10~20 个 60s 人工标注片段作为验收集。
