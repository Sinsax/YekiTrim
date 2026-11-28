# 音效包（sfx pack）结构规则建议

> 2026-11-28。现状取自 `crates/yekitrim-polish/src/assets/sfx_packs.rs`、
> `assets/downloader.rs`、`render/sound.rs`、`crates/yekitrim-server/src/api/polish/assets.rs`、
> `webui/src/components/MediaLibrary.vue`。
>
> 分三块：**A. 已经生效、不能碰的硬规则**（出片路径逼出来的，不是设计选择）·
> **B. 建议补的结构**（清单文件 / 命名 / 闸门）· **C. 现状与规则不一致的地方**（要修的洞）。

---

## A. 三条不变式（不可协商）

这三条不是"建议"，是出片路径决定的。任何包结构都必须满足，否则**成片少一声且只 warn 一句**。

### A1. 全局 id = `<包名>/<变体键>`，键里永远没有 `/`

- 判据只此一份：`sfx_packs::split_entry_id`（"带 `/`"就是用户包）。
- 能成立的前提是**预设包的键永远不含 `/`** —— `sfx_catalog::variant_stem` 把 `Crowds/x.mp3`
  拍平成 `Crowds-x`，所以预设与用户包在 id 空间上不重叠。
- ⚠️ 别在别处另造一种 id 形态（服务端下发、试听分流、下载、出片四处共用这一条）。

### A2. 变体键 = `stem_of(文件名)` = 去扩展名 + 把 `/` `\` 换成 `-`

与 `variant_stem` **同规则**（用户包多去一层 `wav/ogg/m4a/flac/aac`）。
两套规则一旦漂移，症状就是"导入的包出片时没声音"。

### A3. 库内必须**扁平**（或复制时拍平）

出片侧只认 `sfx/<键>.<ext>`（`render::sound::resolve_sound_file`），
`copy_user_sfx` 也只按 `entry.file_name` 单文件复制。
子目录留在库里 = 出片找 `sfx/键.mp3` 找不到 = 少一声。

**推论：键是包的公开 API。** 改文件名 = 改键 = 老工程里所有引用这一条的事件失效。
包一旦被工程用过，文件名就别再动；要改就发一个新包。

---

## B. 建议补的结构

### B1. 目录形态

```
<data>/users/<uid>/sfx/<包名>/
  pack.toml            ← 清单（建议新增，可选）
  <键>.mp3 / .wav …    ← 全部拍平在包根
  .groups.toml         ← 导入时生成（现状保留，见 B4 优先级）
```

- `pack.toml` 放包根：`scan_pack` 只认音频扩展名，非音频文件自动被忽略，
  所以它**不需要藏成 dotfile**；放包根的好处是**导出 zip 时清单跟着走**。
- 手放目录（不经 zip）**只认顶层文件**：`scan_pack` 用 `is_file()` 过滤，
  子目录里的音频**静默不出现**（连 `skipped` 都不计）。这一条要写进包作者文档。

### B2. `pack.toml` 清单（建议 schema）

**这份文件有两个写入者，但不是"二选一"**：

| 谁写 | 什么时候 | 优先级 |
|---|---|---|
| 包作者 | 打包时随 zip 一起带进来 | **最高**（有就不构建） |
| yekitrim 构建（LLM） | 导入后发现没有 `pack.toml` | 次之（可手动重建） |
| 都没有 | 手放的裸目录、构建失败或被取消 | 全部走兜底 |

规则是**"有作者写的就不动它"** —— 构建只在没有清单时跑。作者想改就编辑 `pack.toml`
再点一次「重新构建」，那是一次显式的覆盖操作。

用**数组表**而不是 `[entries.<键>]` 表：键是文件名派生的，可能带空格与点，
写成 TOML 裸键需要转义；数组表没有这个坑，也保留了作者想要的顺序。

```toml
# 可选文件。缺了包照样能用（全部走兜底）；字段缺了走兜底；未知字段忽略（前向兼容）。

[pack]
version     = "1.0.0"     # 只作溯源，不参与比较
author      = ""
license     = "CC0"       # ⚠️ 商用包里这一项是义务，见 THIRD-PARTY-NOTICES.md
source      = ""          # 来源页
description = ""
built_by    = "author"    # "author" | "yekitrim"（构建写的），只作显示与重建判据

[[entry]]
stem    = "敲门"           # 必填：= 文件名去扩展名，必须与磁盘上的文件对得上
name    = "敲门声"         # 显示名；缺省 = stem
group   = "门窗"           # 细分组（前端二级 chips）；缺省 = B4 的优先级链
tags    = ["敲门推门"]      # ⚠️ 必须是内建 84 个 tag 里的词（见 B3）
gain_db = -6.0            # 本条增益（dB）；缺省 0
```

⚠️ **清单放在包目录里（`<包名>/pack.toml`），不是库根** —— 与贴纸的 `<包名>.toml`
（在 `stickers/` 根下）**不同**。理由：音效包要能整个目录导出/搬迁，清单必须跟着走；
贴纸那份是库级索引、本来就不属于某个包。

**每一条都有兜底，且兜底只在 Rust 一份：**

| 字段 | 缺省 |
|---|---|
| `name` | `stem`（现状 `label` 就是它） |
| `group` | `pack.toml` → `.groups.toml` → 「其他」 |
| `tags` | `[]`（参与不了 tag 匹配，但照样能按名选） |
| `gain_db` | `0.0`（由全局 `sfx_volume` 与事件级覆盖决定） |

**清单坏了不拒绝整包**：解析失败只 warn、整份清单当不存在 —— 与"转码失败不是错误"
（`transcode_failed`）同一条哲学：一个手滑的 TOML 不该让 200 个音效全部不可用。

### B3. `tags` 是**封闭词表**

`tags` 不是自由文本，**必须取自 `sfx_catalog::all_tags()`（84 个）**。理由与 `emoji_taxonomy`
一模一样：tag 是两套库之间的**连接键** ——

- `SFX_RULES` 的规则右值全是 `tag:xxx`（字幕关键词 → tag → 音效）；
- `process::resolve_sfx_value` 认 `tag:` 前缀；
- `LLM` 的候选按 `event_type` 从**这 84 个**里挑（`sfx_tags_for_event_type` 返回的就是它们）；
- `is_blacklisted` 的 `tag:xxx` 语义只对能 `lookup` 到的条目生效。

写了词表外的 tag = 永远解析不到 = 静默没声音。**导入时校验并丢弃未知 tag（不失败）**，
日志报一句"丢弃 N 个未知 tag"。这条校验必须走 `all_tags()`，别在前端抄一份。

⚠️ **这是构建的核心约束**：贴纸的 `build_package_tags` 让 LLM **自由发明** tag（10~20 个情绪词，
没有任何词表约束）—— 那套照搬过来对音效**是错的**。音效构建必须把 84 个词连同「大类 + 组数」
一起喂给 LLM（`categories_text()`，`llm/segment.rs:601` 已经在用同一份），并要求它**只能从这里面选**。

### B3.5 构建（`POST /polish/sfx-packs/:name/build`）

对应贴纸的 `POST /polish/stickers/:pack/build`，走同一套 `spawn_job` + `JobManager`。

**输入**：包内全部音频文件名（不带扩展名的键）+ `categories_text()`（84 个词按大类分组、带组数）。

**输出**：`pack.toml`（`built_by = "yekitrim"`）。

**提示词要点（与贴纸那份的差异）**：

| | 贴纸 | 音效 |
|---|---|---|
| tag 来源 | LLM 自由发明 10~20 个情绪词 | **只能从 84 个里选**，词表外一律丢弃 |
| 每个文件 | 可有多个 tag | 建议 1~3 个 tag（音效语义窄，多 tag = 噪音） |
| 分组 | 无 | 可顺带让 LLM 按语义归组（`group`），但**文件名/zip 子目录已有分组时以它为准** |
| 批量 | 120 个文件一批 | 同上；**每批独立解析，聚合时同名 tag 合并** |
| 失败 | `anyhow::bail` | 同；**空结果不写清单**（`build_package_meta_from_tags` 返回 None 那条语义照抄） |

⚠️ **构建产出必须再过滤一遍**：LLM 可能返回 84 个词之外的 tag、可能返回包里不存在的文件名
（贴纸那边 `llm_generate_package_tags` 用 `chunk.iter().any(|c| c == &f)` 挡了后者，音效照抄）。
过滤后的 tag 若全空，**不写 `pack.toml`**、报告"这个包的文件名给不出语义标签"。

⚠️ **构建要能取消**：`progress.cancelled()` 只在**批次边界**查（与贴纸同一口径，
在跑的 LLM 调用照常返回，不要 abort）。

### B3.6 没构建的包怎么办

**照常可用**，只是 LLM 选不到它：

- `scan_pack` 的 `label` 仍是文件名去扩展名，`group` 仍走 `.groups.toml`；
- 素材库里正常列出、能试听、能手选；
- LLM 候选里只出现**有清单**的包（与贴纸 `local_sticker_candidates_text` 只列已构建的**同一口径**）。

这决定了 UI 上要有一个「未构建」标记，与贴纸页完全一致（`p.built === false` → 卡片角标 +
在详情里给「构建」按钮 + 说明「LLM 生成事件时读不到这个包」）。

### B4. 分组优先级（一份实现）

```
pack.toml 的 group  →  .groups.toml  →  zip 子目录名（导入时已折进 .groups.toml）  →  「其他」
```

- `.groups.toml` 仍是**导入期派生物**（zip 子目录名 → 键），只由导入写。
- 若 zip 里带了 `pack.toml`：**原样保留到包根，且不再写 `.groups.toml`** ——
  一个事实一个写者。现在的导入会把 `pack.toml` 当"非音频"跳过，这条要补。
- 手放的包没有 `.groups.toml` ⇒ 全部落「其他」，这是可接受的降级。
- **`pack.toml` 里的 `group` 优先于 `.groups.toml`**（作者意图 > 导入期派生物）；
  两者都没有才落「其他」。

### B5. 命名规则（三处共用）

| 对象 | 规则 | 现状 |
|---|---|---|
| 包名 | `sanitize_pack_name`：1~32 个字母/数字/`_`/`-`（含汉字），不以 `.` 开头，非 Windows 保留名 | ✅ 已有 |
| 变体键 | `is_safe_component`：≤128、无 `/ \ :`、无控制字符、非 `CON/PRN/AUX/NUL/COM1..9/LPT1..9` | ✅ 已有 |
| 细分组 | **没有校验**（zip 子目录名直接 trim 后写入 `.groups.toml`） | ❌ 见 C8 |

⚠️ **`CON.mp3` 这类文件名会被静默跳过**（键 `CON` 过不了 Windows 保留名）。包作者文档要写。

### B6. 导入闸门

现状：`ZipLimits::default()` = **单文件 32 MiB / 解压总量 256 MiB / 2000 条目**，
但**路由的 `DefaultBodyLimit` 是 4 GiB，且请求体整份进内存**（`body: Bytes`）。
一个 3 GiB 的 zip 会先完整读进 RAM、再被 256 MiB 那层拒掉。

建议：

1. 音效包的上传体上限**降到与解压总量同量级**（512 MiB 足够：2000 条 × 32 MiB 本来就是上限），
   并在前端按 `file.size` 预检 —— 与贴纸那条同一套（`upload_limit_guard` 现在没挂）。
2. **拍平重名不能只回计数**：现在"只留第一个、其余计入 `skipped`"，
   用户看不出丢了哪几个。回包里带 `dropped: ["门/敲.mp3", …]`（前 N 条），前端提示
   "同名文件被跳过，建议加前缀改名"。

### B7. 音频技术规格（软规则，写进包作者文档）

- 格式：mp3 128 kbps（导入会尽力转，失败保留原格式并计 `transcode_failed`）。
  现状命令是 `-codec:a libmp3lame -b:a 128k`，**不强制单声道/采样率**：立体声 48 kHz 的源会原样转成
  立体声 48 kHz 的 mp3。要不要统一成 `-ac 1 -ar 44100` 是个取舍 —— 统一后包内解码路径与内建库一致、
  体积更小；但会给"立体声包"降级。建议：**默认统一，清单里 `[pack] keep_channels = true` 可关**。
- 时长：建议 ≤ 5 s，**硬上限建议 30 s**（现状没有任何时长检查，一个 5 分钟的 wav 也能进包）。
- 响度：这是用户包与预设包**听感差异的最大来源**，见 C10。

---

## C. 现状与规则不一致的洞（建议按序修）

| # | 洞 | 位置 | 后果 |
|---|---|---|---|
| **C1** | **点开用户包是空白**（最严重） | `entries` 只由 `sfx_catalog::all()` 生成；`user_packs[]` 是**另一个顶层键**，前端从未读它（`MediaLibrary.vue` 只取 `sx.entries` / `sx.packs`） | 左侧卡片显示"N 组"（数字来自服务端 `count`，是对的），点进去网格里**一条都没有**。用户会以为包导坏了 |
| **C2** | **编辑器选不到用户包** | `TimelineEditor.vue` 的 `SfxPicker` 吃的是同一个 `entries` | 时间线上手动给事件配声时，候选里**只有预设**。LLM 能写 `<包名>/<键>`、人却不能选 |
| C3 | **用户包 id 写不进黑名单** | `polish_sfx_blacklist_put` 对 `lookup(s).is_none()` 一律 400 | "屏蔽这一组"对导入包**必失败**（`lookup` 只查内嵌目录）。今天被 C1 遮住，修了 C1 就会露出来 |
| C4 | **`tag:xxx` 对用户包恒不命中** | `is_blacklisted` 里 `lookup(id)` 只查内嵌目录 | 屏蔽某个 tag 管不到用户包；同理 tag 匹配也管不到 |
| C5 | **`小森平` 不是保留名** | `import_pack_from_zip` / `delete` 只比 `CATEGORY_ORDER` 去序号后的名字 | 用户能建一个同名包，左侧出现两张一模一样的卡 |
| C6 | **显示名恒等于键** | `scan_pack` 的 `label` = 文件名去扩展名 | 文件叫 `sfx_01.mp3` 就永远显示 `sfx_01`，没有另起中文名的途径 |
| C7 | **LLM 候选对用户包是"前 16 条任意条目"** | `sfx_packs::candidates_text(root, used, 16)` 按包名/文件名顺序取满即止 | 包多或包大时，后面的条目**永远进不了候选**；也无法按事件类型筛选 |
| C8 | **细分组名不校验** | zip 子目录名 `trim()` 后直接写 `.groups.toml` | 含 `"` 或换行的目录名能注入一行假分组（低危，但一行代码可堵） |
| C9 | **手放子目录静默丢失** | `scan_pack` 用 `is_file()` 过滤 | 用户把解压后的树直接拷进库，子目录里的音频一条都不出现、不报错 |
| C10 | **没有响度归一** | 全局 `sfx_volume` 默认 `0.1`，无 per-asset gain | 不同来源的包音量差 10~20 dB，同一个 0.1 对 A 包震耳、对 B 包听不见 |
| C11 | **前端无体积预检** | `onSfxPackPick` 不检查 `file.size` | 大包先传完再 400（且整份在内存里） |

**C1/C2 是同一条"没接上"**：服务端把用户包放在 `user_packs`，前端只吃 `entries`。
最小修法是**服务端在 `entries` 里一并下发用户包条目**（`id` = `<包名>/<键>`，`pack` = 包名，
`group` = ""，`subgroup` = 组名，`variants` = `[键]`，`tags` = 清单里的 tag）——
一份 `entries` 喂两个消费者，前端不用为"预设/用户包"写两条路。
⚠️ 别让前端自己拼：`pack` 为空的用户包条目会被 `sfxPackEntries` 过滤掉，症状与现在一样。

**C3/C4 是同一件事的两半**：用户包在"屏蔽/tag"这条语义链上完全不可见。
最小修法是把 `is_blacklisted` 与 `blacklist_put` 的校验都改成
"内嵌目录 **或** 磁盘用户包能解析出这一条"，并让 `is_blacklisted` 对 `<包名>/<键>`
按**包内 tag**（`pack.toml` 的 `tags`）匹配 `tag:`；没有清单的包只支持精确 id。

**C10 的最小可行方案**：导入时对每条跑一次 `volumedetect`，按目标峰值（如 -3 dBFS）
记 `gain_db` 进 `.groups.toml` 的扩展列或 `pack.toml`；出片侧 `mix_sfx` 拼 `volume` 时叠加。
不做响度归一的包，行为与现在完全一致（`gain_db = 0`）。

---

## D. 一句话版本

**包 = 一个扁平目录 + 可选 `pack.toml`；有作者写的就不构建，没有就让 yekitrim 构建
（词表只有 84 个 tag，不像贴纸那样自由发明）；键是公开 API；清单的每一栏都有兜底
且兜底只在 Rust 一份；坏清单降级不拒包。**

## E. 落地顺序（如果要改代码）

> **2026-11-28 状态**：第 1 步（C1/C2）、第 2 步（B2 + B3.5）、第 3 步的 B3/B4、
> 第 4 步（C3/C4）、C5（预设库名保留）、C8（分组名校验）已落地。第 5/6 步未做。

1. ~~**C1/C2**（用户包在界面上不可见）~~ ✅ 已修：服务端把用户包条目并进 `entries`。
2. ~~**B2 + B3.5**（`pack.toml` + 构建端点）~~ ✅ 已做：`POST /polish/sfx-packs/:name/build`，
   `JobClass::Medium` + 资源键 `sfx_packs`；词表外 tag 在导入与构建两处过滤。
3. ~~**B3/B4**（tag 封闭词表 + 分组优先级）~~ ✅ 已做（`filter_known_tags` + `scan_pack` 的优先级链）。
4. ~~**C3/C4**（黑名单与 tag 对用户包生效）~~ ✅ 已做（`user_pack_blacklisted` + 写侧校验认用户包 id）。
5. **B6/C9**（上传体上限、拍平重名提示、手放子目录提示）—— **未做**。
6. **C10**（响度归一）—— 未做，收益大但实现独立。
