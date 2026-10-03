# YekiTrim Enhance — LLM 分镜 + 模块化精修

## 能力分层

```
基础（永远按 config 执行）      特效（需 LLM 判时间+内容）
─────────────────────────────  ─────────────────────────────
字幕烧入 (subtitle)             弹幕高亮弹窗 (dm_popup)
弹幕烧入 (danmaku)              弹幕热点加速 (dm_hotspot)
                                屏幕震动 (shake)
开场/结尾（config 选变体）       闪白过渡 (flash)
  intro: fade | zoom | both | none
  outro: fade | zoom | both | none
```

**基础 & 开场/结尾**: config 开关参数，LLM 不参与。  
**特效**: LLM 从可用效果表中选，指定时间点和参数。无 LLM 时不执行。

---

## 架构

```
crates/yekitrim-core/src/enhance/
├── mod.rs           ← 编排器
├── config.rs        ← 配置 (全局 + 项目 merge)
├── ass.rs           ← 字幕/弹幕 ASS 文件生成
├── llm.rs           ← LLM 分镜 + 特效建议
├── render.rs        ← filter_chain 组装 + ffmpeg
└── effects/
    ├── mod.rs       ← Effect trait + 注册表
    ├── intro.rs     ← 开场 (fade/zoom/both)
    ├── outro.rs     ← 结尾 (fade/zoom/both)
    ├── dm_popup.rs  ← 弹幕高亮弹窗 (LLM)
    ├── dm_hotspot.rs← 弹幕热点加速 (LLM)
    ├── shake.rs     ← 屏幕震动 (LLM)
    └── flash.rs     ← 闪白过渡 (LLM)
```

### Effect trait

```rust
#[async_trait]
trait Effect: Send + Sync {
    fn name(&self) -> &'static str;           // "intro" / "dm_popup"
    fn config_key(&self) -> &'static str;     // "enhance.intro"
    fn category(&self) -> EffectCategory;     // Base | IntroOutro | Llm
    fn is_enabled(&self, cfg: &EnhanceConfig) -> bool;
    fn build_filter(&self, ctx: &EffectCtx, segments: &[Segment]) -> Result<Vec<FilterDesc>>;
}

enum EffectCategory {
    Base,         // 字幕/弹幕，始终执行
    IntroOutro,   // 开场/结尾，config 决定
    Llm,          // 需 LLM 建议，无 LLM 跳过
}
```

---

## 配置

**`.yekitrim/config.toml`（全局默认）：**

```toml
[enhance.intro]
style = "zoom"        # fade | zoom | both | none
duration = 1.0

[enhance.outro]
style = "fade"        # fade | zoom | both | none
duration = 0.5

[enhance.dm_popup]
enabled = true

[enhance.dm_hotspot]
enabled = true

[enhance.shake]
enabled = true

[enhance.flash]
enabled = true

[subtitle]
font = "Microsoft YaHei"
font_size = 36
primary_color = "#FFFFFF"
outline_color = "#000000"
outline = 2
margin_bottom = 60
max_width = 30
lines = 2
```

---

## 开场/结尾实现

intro 模块根据 `style` 参数应用不同 filter：

| style | 开场 filter | 结尾 filter |
|-------|------------|------------|
| `fade` | `fade=t=in:st=0:d={d}` | `fade=t=out:st={end-d}:d={d}` |
| `zoom` | `zoompan=z='1.5-0.5*min(1,on/(FPS*d))'` 前 d 秒 | `zoompan=z='1+0.5*min(1,(on-{end_frame})/(FPS*d))'` 后 d 秒 |
| `both` | fade + zoom 叠加 | fade + zoom 叠加 |
| `none` | 跳过 | 跳过 |

---

## LLM 分镜 + 特效建议

### 输入

```json
{
  "duration": 125.0,
  "event_type": "reaction",
  "label": "看到弹幕笑疯了",
  "popularity": 0.88,
  "subtitles": [
    {"start": 0, "end": 3.5, "text": "今天来玩一个粉丝推荐的游戏"},
    ...
  ],
  "danmaku": {
    "total": 320,
    "peaks": [
      {"time": 15.0, "count": 45, "texts": ["23333", "节目效果"]}
    ]
  }
}
```

### 输出

LLM 从可用特效表中选出**子集**，指定应用位置和参数：

```json
{
  "segments": [
    {"name": "intro", "start": 0, "end": 8.0, "type": "setup"},
    {"name": "climax", "start": 15.0, "end": 35.0, "type": "reaction"},
    {"name": "outro", "start": 45.0, "end": 50.0, "type": "wind_down"}
  ],
  "effects": [
    {"type": "dm_popup", "time": 25.0, "text": "笑死了哈哈哈哈", "duration": 3.0},
    {"type": "shake", "time": 16.0, "intensity": 0.4},
    {"type": "flash", "time": 15.0}
  ]
}
```

LLM 的 `type` 只能从可用特效名中选，不能发明新效果。

---

## CLI

```bash
yekitrim enhance                           # 全部 (按 config + LLM)
yekitrim enhance --only subtitle           # 只烧字幕
yekitrim enhance --only danmaku            # 只烧弹幕
yekitrim enhance --only subtitle,danmaku   # 只烧字幕+弹幕
yekitrim enhance --skip shake              # 跳过震动
yekitrim enhance --dry-run                 # 只输出 ffmpeg 命令
yekitrim enhance --no-llm                  # 跳过 LLM 特效
```

`--only` / `--skip` 对 effect name 做过滤，支持逗号分隔。

---

## 实施顺序

| # | 模块 | 内容 | 依赖 |
|---|------|------|------|
| 1 | `effects/mod.rs` | Effect trait + registry + category 过滤 | — |
| 2 | `config.rs` | 配置类型 + merge | — |
| 3 | `ass.rs` | 字幕/弹幕 ASS 生成 | config |
| 4 | `effects/intro.rs` | 开场 (fade/zoom/both/none) | config |
| 5 | `effects/outro.rs` | 结尾 (fade/zoom/both/none) | config |
| 6 | `render.rs` | filter_chain 组装 + ffmpeg | 1-5 |
| 7 | `llm.rs` | LLM 分镜 + 特效建议解析 | — |
| 8 | `effects/dm_popup.rs` | 弹幕高亮弹窗 | llm |
| 9 | `effects/dm_hotspot.rs` | 弹幕热点加速 | llm |
| 10 | `effects/shake.rs` | 屏幕震动 | llm |
| 11 | `effects/flash.rs` | 闪白过渡 | llm |
| 12 | `mod.rs` | 编排器 + CLI 集成 | 1-11 |
