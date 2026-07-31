# Polish 特效均衡 + SFX 兜底设计

日期: 2026-07-31
状态: 已批准

## 背景

`vtrim polish preview` 在 clip-01 工程（141s, 62 条字幕, 9 segments）上暴露三个问题：

1. **SFX 大量失效**：LLM 为独立 `sfx` 事件编造了 18 个自由描述名（`night_ambient`、`heartbeat`…），既非 catalog id 也非 `tag:` 引用，全部被下载器丢弃（"未在 catalog 中找到"）。
2. **贴纸全部是 emoji**：`sticker_source` 默认 `["emoji"]`，轮询只有一种来源；且 emoji 匹配有错配（`fall`→枫叶 1f342）和漏配（`anxiety` 无结果）。
3. **运镜缩放过大**：camera 事件 `中`=1.5x / `近`=1.8x 且持续保持，画面始终大特写，观感过重。

用户要求：SFX/贴纸数量按字幕数 + 视频时长判断；贴纸多来源平均使用；运镜缩放降低。

## 设计

### A. 数量预算（字幕数 N + 时长 D）

纯函数 `content_budget(subtitle_count, duration, seg_count: Option<usize>) -> (sticker_budget, sfx_budget)`：

- `sticker = min(round(N/10), ceil(sqrt(D/7)), seg_cap)`
- `sfx = min(round(N/15), ceil(D/25), seg_cap)` — 仅统计独立 `Sfx` 事件（视觉特效自带音效不占名额）
- `seg_cap = Some(seg_count) 时 = max(1, seg_count-1)`；`None`（LLM prompt 用）时省略该上限

落地两处：
1. `llm/segment.rs` prompt：明确写出 "sticker ≤ X 个，独立 sfx ≤ Y 个"，源头控制。
2. `process.rs` Step 1b：现有"均匀抽稀保留"逻辑抽成 `cap_events(segments, predicate, budget)`，对 sticker 和独立 sfx 各执行一次，作为兜底。

clip-01 参考值：sticker 5（现状 5），sfx 4（现状 18）。

### B. SFX 三级兜底

`sfx_catalog.rs` 新增 `fuzzy_search(query)`：query 按非字母数字分词，与每个条目的 id/label/tags 拼接文本做包含计分（token 命中 +3，id 精确 +10，tag 精确 +8），最高分 > 0 返回。

`process.rs` `resolve_event_sfx` 升级为 `resolve_sfx_value(value) -> Option<String>`：
1. `lookup(id)` 命中 → 原样
2. `tag:` 前缀 → `search_by_tags`
3. 裸 tag 名 → `search_by_tags`
4. `fuzzy_search` 命中 → 替换 + 日志 `✓ sfx 模糊匹配「x」→ y`
5. 全失败 → 丢弃（Sfx name 置空 / 其他事件 sfx 置 None）+ 警告

**resolve 移到 download 之前**，保证下载的是最终 id。幂等：解析结果写回 polish.toml。

### C. 贴纸三来源均衡

1. `default_sticker_source` → `["emoji", "irasu", "local"]`。
2. `Sticker` 事件新增 `source: Option<String>`（serde default），process 解析成功后写入；下载阶段优先读持久化 source（旧工程无字段时回退轮询），消除 process/downloader 双轮询不同步问题。
3. 轮询 + 逐源兜底：每个 sticker 从轮询指针起依序尝试各来源，首个成功者生效，指针每次 +1；全部失败 → 无 name + 警告。
   - emoji 失败判定：`search` 无结果或全部 codepoint 已被使用
   - irasu 失败判定：process 阶段实际查询 irasutoya（`resolve_image_urls`，带缓存）
   - local 失败判定：本地索引无命中或组无文件
4. emoji 匹配改进：`search(kw, 8)` + 已用 codepoint 去重（含历史 name）；扩充 `FALLBACK_MAP`（anxiety/fall/game/dream/nightmare/shock 等）。
5. 关键词语言规则：sources 含 `emoji` → 英文（emoji catalog 881 条全英文标签）；仅 `irasu` → 日文；仅 `local` → 英文。
6. 使用率汇总日志：process 结束输出 `贴纸分布: emoji N / irasu N / local N + 未解析关键词列表`。

### D. 运镜缩放

- 新增 `[style.camera] zoom_mid = 1.15, zoom_close = 1.3`（serde default，可配置）。
- `index.html` camera handler 读取配置替代硬编码 1.5/1.8；缩放后保持行为不变。

## 测试

- 单测：`content_budget`（含无 seg_cap 分支）、`fuzzy_search`（"night_ambient" 等实际失败样例）、`cap_events` 均匀保留。
- 手动：clip-01 重跑 `vtrim polish preview`，验证贴纸分布、SFX 全部有声、运镜观感。
