# V-Trim Roadmap

待检验和排期的新功能。

## 高优先级

### A. `vtrim validate` / `vtrim doctor` 增强

验证 vtrim.toml + 运行环境：

- 检查 ffmpeg/ffprobe 在 PATH 中
- 验证模型文件完整性和版本兼容性
- 测试 LLM API 连接（复用现有 `test_llm`）
- 预检视频格式 / 弹幕文件格式 / 音频采样率
- 检查磁盘空间是否足够
- 报告缺失的可选组件（sherpa-onnx 模型等）

### B. 断点续传 / 进度持久化

ASR 分块转写时，每完成一块写 `.vtrim/progress.json`，重启时跳过已完成块。
LLM 订正批次也支持断点续传（`--resume` 或自动检测）。

### C. 静音检测参数暴露到配置

`vtrim-core/src/asr/silence.rs` 中有 VAD/silence 检测逻辑（threshold / min_speech_duration / max_speech_duration），
但当前仅在 `vtrim.toml` 示例中出现，未在 `Config` 结构体中定义。
应加到 `AsrConfig` 或 `CuttingConfig` 中，让用户可配置。

### D. `vtrim process --from-stage`

当前 `process` 内部硬编码阶段路由（检测已完成的跳过）。
提供 `--from-stage 2` 让用户指定从某个阶段开始重跑，灵活调试。

### E. `vtrim detect --preview` / 事件时间线预览

对检测到的事件输出文本/JSON 时间线预览（`start - end | label | popularity`），
让用户确认后再执行剪切。可结合 `less` 或输出到文件。

## 中优先级

### F. 多 LLM Provider 支持

当前 `send_request` 写死 OpenAI 兼容接口：
- `Authorization: Bearer` header
- `/chat/completions` 路径
- OpenAI 风格的 `choices[0].message.content` 响应格式

可抽象 `LlmClient` trait，支持 Anthropic / Google / 本地 ollama 等。

### G. 事件人工标注/编辑

- `vtrim events edit`：读取 `events.json` → 打开 `$EDITOR` → 保存覆盖
- 或支持 `events.edit.json` 作为用户覆盖文件，merge 到最终输出

### H. 多语言 ASR 支持

`AsrConfig.language` 已定义并传给 sherpa-onnx，但：
- `srt.rs` 中 `write_srt` / `parse_srt` 硬编码 `language: "zh"`
- LLM prompt 目前纯中文
- 事件类型 label 和 summary 也是中文

需要抽象语言策略，英文/日文直播场景才能工作。

### I. `--json` 输出模式

`vtrim status --json`、`vtrim detect --json` 等输出 JSON 格式，
便于 CI/脚本集成。

## 低优先级

### J. ASR 并行引擎切换

当前 `std::thread::scope` 每批重新创建 OS 线程。
可切换为 `rayon` 或固定 `threadpool`，减少小块时的创建开销。
对大文件（数小时录像）效果更明显。

### K. 自动 GPU 检测报告

`vtrim doctor` 输出 GPU 编码器可用性：
- NVENC / QSV / AMF / libx264
- 推荐最优编码参数

### L. 弹幕密度热力图输出

`vtrim detect --heatmap` 生成 JSON 格式的弹幕密度热力图数据，
可导入外部可视化工具。

### M. 配置文件 Schema 导出

`vtrim config schema` 输出 JSON Schema，便于编辑器补全。
