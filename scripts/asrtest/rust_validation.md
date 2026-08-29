# Rust 双模型时间戳对齐验证（68.8s clip_full.wav）

## 验证方式

使用 `vtrim transcribe` 直接对 `scripts/asrtest/wavs/clip_full.wav` 转写：

```bash
# 在含 vtrim.toml 的项目目录中
vtrim transcribe -i <path>/clip_full.wav -o output
```

关键配置：

```toml
[asr]
model = "qwen3-asr"
timestamp_alignment = true
chunk_duration = 60.0
chunk_overlap = 10.0
segment_gap = 1.0
```

## 结果摘要

- 音频时长：68.8s
- 输出片段数：9
- ASR 元数据：
  - 模型：`qwen3-asr`
  - 锚点数：16
  - 锚点覆盖率：74.8%
  - 最大插值跨度：11
  - 降级原因：无（双模型对齐正常）

## 与参考 `clip_aligned.srt` 对比

时间轴整体对齐，关键边界接近：

| 事件 | 参考 SRT | Rust 输出 |
|---|---|---|
| 开场句 | 0.12 ~ 12.78 | 0.12 ~ 12.78（拆为两句） |
| “让我来搜一下” | 18.60 开始 | 18.60 开始 |
| “是那次吧” | 26.76 ~ 31.26 | 27.23 ~ 31.26 |
| 搜歌段 | 33.75 开始 | 33.72 开始 |
| 第二段歌曲讨论 | 52.74 开始 | 52.70 开始 |
| 结尾 | 1:02.40 ~ 1:08.70 | 1:03.68 ~ 1:08.72 |

文本差异主要来自 ASR 识别本身（如“从山上摔” vs “从身上摔”），不是时间轴对齐问题。

## 结论

Rust 双模型时间戳对齐已能复现 Python 原型的时间轴效果；后续可补充 10~20 个 60s 人工标注片段做正式回归。
