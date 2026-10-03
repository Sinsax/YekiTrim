# ASR 阶段设计方案

## 目标

对 57 分钟直播录像音频（16kHz mono WAV）进行语音转文字，输出
与参考 SRT（`[后处理].srt`, 474 段, 覆盖全程 57min25s）质量接近的字幕文件。

## 现状问题

| 指标 | 当前输出 | 参考 SRT |
|------|---------|----------|
| 段数 | 47~108 | 474 |
| 平均段长 | ~70s | ~7s |
| 覆盖范围 | 部分缺失 | 0:09 ~ 57:22 全量 |
| 分段方式 | Silero VAD 外部切分 | 模型内部逐句时间戳 |
| 根因 | 背景音乐导致 VAD 漏检大量语音段 | — |

## 实验结论

已排除的路径：

1. **VAD+ASR (`sherpa-onnx-vad-with-offline-asr`)**: Silero VAD 对带背景音乐的
   直播音频分段力不足，threshold 从 0.5 降到 0.05 段数仅从 47 增到 108，远不及 474。
2. **SenseVoice 全量转写 (`sherpa-onnx-offline`, 无分块)**: 模型 OOM，
   MatMul 请求 52.8GB 内存，57min 音频无法一次性处理。
3. **Paraformer**: 无句子级时间戳输出，仅返回完整文本。
4. **固定分块 + split_sentences**: SenseVoice 输出不含标点符号 (。！？)，
   导致 `split_sentences` 无法切分，段数不变。

已验证可行的关键组件：

- **SenseVoice int8 模型**: 中文/粤语转写质量可接受；输出 JSON 中包含
  `timestamps[]` 和 `tokens[]` 数组（逐字符时间戳）。
- **build_segments_from_timestamps()**: 利用 `timestamps` 数组按时间间隙
  (>0.4s) 和段长上限 (15s) 做分句，每 60s chunk 可产出 5~10 段。
- **chunk::split_fixed_with_overlap()**: ffmpeg 切割音频分块，支持重叠。

## 最终方案

### 管线架构

```
57min WAV 音频
  |
  +- 1. 切割为 70s chunks（60s 有效 + 10s 重叠）
  |     chunk 0: [0s, 70s]
  |     chunk 1: [50s, 120s]
  |     chunk 2: [100s, 170s]
  |     ...
  |     chunk N: [..., 3448.8s]
  |     约 69 个 chunk，写入 $TEMP/yekitrim_asr/
  |
  +- 2. 逐 chunk 调 sherpa-onnx-offline + SenseVoice
  |     命令: sherpa-onnx-offline
  |       --sense-voice-model=model.int8.onnx
  |       --sense-voice-use-itn=true
  |       --tokens=tokens.txt
  |       --num-threads=2
  |       chunk_N.wav
  |
  |     输出: JSON { text, timestamps[], tokens[], ... }
  |
  +- 3. build_segments_from_timestamps()
  |     输入: tokens[], timestamps[], chunk_offset
  |     逻辑:
  |       - 连续 token 间 gap > 0.4s → 分段
  |       - 当前段持续 > 15s → 强制分段
  |       - 每个分段: start=offset+first_ts, end=offset+last_ts+est_dur
  |     输出: Vec<TranscriptSegment>
  |
  +- 4. 去重（处理重叠区域）
  |     规则: 段中心点 < chunk_effective_end → 归当前 chunk
  |     （非最后一个 chunk 的 segments 截断到 effective_end）
  |
  +- 5. 排序 + 清理临时目录
  |
  +- 6. 写 SRT 文件
```

### 重叠设计

```
时间轴:  0       50      60      70     100     110     120
         |--------|-------|-------|-------|-------|-------|
Chunk 0: [===================]   有效 0-60, 重叠 50-70
Chunk 1:               [======================] 有效 50-110, 重叠 100-120
Chunk 2:                               [======================]

重叠区去重:
  [50,60]: 段中心 < 60 → 归 Chunk 0
  [60,70]: 段中心 >= 60 → 归 Chunk 1
```

任何跨固定边界的句子都完整落在至少一个 chunk 内，不会丢失或切断。

### 预期产出

| 指标 | 预期值 |
|------|--------|
| 段数 | 400~500 |
| 覆盖 | 0:00 ~ 57:30 全程 |
| 精度 | 逐字符时间戳 + ITN |
| 耗时 | ~2.5 分钟 (69 chunk x ~2s) |
| 临时文件 | $TEMP/yekitrim_asr/ (处理后自动清理) |

### 涉及文件

| 文件 | 改动 |
|------|------|
| `crates/yekitrim-core/src/asr/mod.rs` | 替换 VAD 路径为 chunk + local 路径 |
| `crates/yekitrim-core/src/asr/chunk.rs` | 临时目录改用 `std::env::temp_dir()` |
| `crates/yekitrim-core/src/asr/local.rs` | 已就绪 (build_segments_from_timestamps) |
| `crates/yekitrim-core/src/asr/vad.rs` | 保留备用，暂不调用 |

### 依赖

- ffmpeg + ffprobe (在 PATH 中)
- sherpa-onnx-offline binary + SenseVoice int8 model (~/.yekitrim/)
- sherpa-onnx-vad-with-offline-asr binary (备用)

## 备选探索

- **FunASR-nano** (sherpa-onnx-sense-voice-funasr-nano-int8, 179MB):
  基于 Qwen3-0.6B 的 LLM ASR 模型，可能支持流式处理（无需分块），
  时间戳输出格式需验证。
