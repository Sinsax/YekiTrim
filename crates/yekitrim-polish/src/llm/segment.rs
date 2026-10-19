//! polish::llm::segment：Step 1 全局分段（一窗一段，产出 `Segment`）与分段/事件的提示词拼装。
//!
//! 入口：`generate_segments`、`build_segmentation_system_prompt`、`build_segmentation_user_prompt`、`build_events_system_prompt`、`build_events_user_prompt`、`normalize_segments_json`。
//! 边界：覆盖层示例必须给 `colors` 数组（四处求值器只认 `colors`，只写 `color` 会导致覆盖层全画不出来）；改内建 prompt 要同步用户配置里的副本 —— 见 `crates/yekitrim-polish/src/llm/AGENTS.md`

use crate::parse::srt::Subtitle;
use yekitrim_core::danmaku::emotion::{Emotion, EmotionWindow};
use crate::project::Segment;
use yekitrim_llm::agent::extract_json_array;
use yekitrim_llm::LlmClient;

/// 调用 LLM 生成话题区间 + 段内事件 + 音效判断
/// sticker_source 影响提示词中 sticker 事件的语言描述, 空数组则不提及 sticker
/// sticker_budget/sfx_budget 为数量上限 (来自 content_budget), 提示词中明确约束
/// sfx_blacklist: 被屏蔽的音效/标签, 注入 prompt 让 LLM 主动避开
pub async fn generate_segments(
    client: &LlmClient,
    subtitles: &[Subtitle],
    event_type: &str,
    sticker_source: &[String],
    sticker_budget: usize,
    sfx_budget: usize,
    sfx_blacklist: &[String],
    // §10 **情绪热区**：`(窗口, 窗口时长)` —— 让分段不只看话题，也看观众反应翻面。
    // 传 `&[]` = 没有情绪数据（没弹幕且字幕也判不出）⇒ 提示词里那一节整块不出现，
    // **与改动前逐字节相同**（存量工程的 LLM 输入不变）。
    emotion: &[EmotionWindow],
    emotion_window_secs: f64,
) -> anyhow::Result<Vec<Segment>> {
    let system_prompt = build_segmentation_system_prompt(sticker_source);
    let user_prompt = build_segmentation_user_prompt(
        subtitles,
        event_type,
        sticker_budget,
        sfx_budget,
        sfx_blacklist,
        &emotion_hint_lines(emotion, emotion_window_secs),
    );

    // 解析失败时把错误回传 LLM 重试 1 次（P4 第二部分）
    let mut last_err = None;
    let mut current_prompt = user_prompt;
    for attempt in 0..2 {
        let response = client.chat(&system_prompt, &current_prompt).await?;
        let json = extract_json_array(&response)?;
        let segments: Result<Vec<Segment>, _> = serde_json::from_str(&json);
        match segments {
            Ok(s) if !s.is_empty() => return Ok(s),
            Ok(_) => {
                last_err = Some(anyhow::anyhow!("Segments empty"));
                current_prompt = format!(
                    "{current_prompt}\n\n上一次输出是空数组。请按实际字幕话题切分并输出至少一个 Segment。"
                );
            }
            Err(e) => {
                last_err = Some(anyhow::anyhow!("Segments parse failed: {e}"));
                if attempt == 0 {
                    current_prompt = format!(
                        "{current_prompt}\n\n上一次输出无法解析：{e}\n请只输出 JSON 数组，不要 markdown。"
                    );
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("Segments failed")))
}

/// Step2 system prompt: 事件生成（音效只能从候选选择, 数量受预算约束）
pub(crate) fn build_events_system_prompt(
    sticker_source: &[String],
    sfx_blacklist: &[String],
    local_sticker_text: &str,
) -> String {
    let sticker_desc = sticker_lang_desc(sticker_source);
    let blacklist_line = if sfx_blacklist.is_empty() {
        String::new()
    } else {
        format!("- 以下音效被屏蔽, 禁止使用: {}
", sfx_blacklist.join(", "))
    };
    let local_line = if local_sticker_text.is_empty() {
        String::new()
    } else {
        format!("\n{}", local_sticker_text)
    };
    // ⚠️ **内建 emoji 词表必须进提示词**（2026-10-17）：在这之前 LLM 只被告知"写英文，
    //    如 laugh/cry/shock/game"——它**不知道有哪些词可用**，于是我们只能拿子串去接
    //    （命中谁由目录顺序决定）。词表来自 `emoji_taxonomy::prompt_vocab_text()`，
    //    **一份真源**，别在这里另抄。
    let emoji_line = if sticker_source.iter().any(|s| s == "emoji") {
        format!(
            "\n## 内建 emoji 词表（选了 emoji 来源时，keyword 只能从这些取值里拼）\n{}\n",
            crate::assets::emoji_taxonomy::prompt_vocab_text()
        )
    } else {
        String::new()
    };
    // 两份类型清单**从效果词汇表算**（见下面的 `fx_type_list` / `durable_type_list`）。
    let fx_list = fx_type_list();
    let durable_list = durable_type_list();
    format!(r##"你是一个直播切片事件助手。为指定时间段生成视觉特效/贴纸/音效事件。

事件时间一律写成 **time(开始秒) + end(结束秒)** 两个绝对时间, 不要写"时长";
两者之差才是持续时间。

事件类型 (每个事件对象以 "type" 字段开头):
- camera: 运镜. {{"type":"camera","time":0.5,"end":3.5,"zoom":"中","anchor":"liver"}}
  zoom = "远"/"中"/"近"/"特写"（远=画面缩小成画中画）
  特写比近景再近一档: 近景是"凑近了看", 特写是"怼到脸上"——
  只在情绪炸开的那一瞬间用(爆笑/震惊/名场面), 整段长时间怼着脸会很压迫, 别当常规景别
  特写只能聚焦主播: 写别的聚焦会被系统改回 "liver"
  例: {{"type":"camera","time":42.0,"end":44.0,"zoom":"特写","anchor":"liver"}}
  anchor = 聚焦谁: "liver"(主播) / "danmaku"(弹幕) / "sticker"(贴纸) / "center"(画面中央)
  想从一处移到另一处, 再写 anchor_to (同样的取值); 不写就是原地聚焦
  想**硬切**(切镜, 瞬间换构图、没有推移动画)再加 "cut":true —— 只在刻意切镜时用, 不是常规手段
  想控制**推进快慢**加 "speed":"snap"/"fast"/"med"/"slow"
  (snap=0.1秒几乎瞬移、像"怼"上去 -> 爆点/惊吓; fast=0.25秒 ->
  情绪起/话题转折; med=默认**(不用写)**; slow=1.5秒缓慢推近 -> 蓄力/铺垫/感伤)
  ⚠️ speed 与 cut 别一起写 (同时写以 cut 为准、speed 被忽略)
  例: {{"type":"camera","time":30.0,"end":34.0,"zoom":"近","anchor":"liver","speed":"slow"}}
  近景想要"手持相机"的临场晃动感, 再加 "handheld":true —— **只在 zoom:"近" 上有效**（其他景别会被系统剔除）;
  它是叠加在运镜上的**持续状态**: 开了就一直摇到下一条不带它的运镜, 不是一次性特效
  例: {{"type":"camera","time":20.0,"end":24.0,"zoom":"近","anchor":"liver","handheld":true}}
  例: 主播→弹幕→回主播 = 两条事件
    {{"type":"camera","time":5.0,"end":9.0,"zoom":"中","anchor":"liver","anchor_to":"danmaku"}}
    {{"type":"camera","time":9.0,"end":12.0,"zoom":"中","anchor":"danmaku","anchor_to":"liver"}}
- sticker: 贴图表情. {{"type":"sticker","time":10.0,"end":12.0,"keyword":"ysyk","sticker_tag":"害怕","x":"auto","y":"auto"}}
  keyword 语言: {}
- element: **文字元素**（钩子标题 / 章节卡 / 吐槽 / 计数）。**候补效果，不是常规手段**
  {{"type":"element","kind":"text","time":30.0,"end":32.4,"text":"名场面","style":"hook","pos":"top","color":"pink"}}
  {{"type":"element","kind":"text","time":75.0,"end":77.5,"text":"第3次","style":"count","pos":"center"}}
  ⚠️ **text 写"关键词/短词组"，不写句子**：2~8 个字，从这一段**字幕或弹幕**里挑那一下的
  关键词/梗/数字 —— 它像贴在画面上的一张标签，观众扫一眼就懂。
  反例："这就是传说中的…"这种整句台词就是错的：那是字幕在干的事，抄进来只会挡住画面。
  style 四选一:
  * `hook` 钩子标题 —— 高对比强描边, 用在**开场或一个爆点之前**, 要"抓人"
  * `chapter` 章节卡 —— 带底居中, 用在**话题切换**处, 像小标题
  * `comment` 吐槽 —— 细边带底色块, 用在**你对画面里的事有话说**的时候(最常见的一种)
  * `count` 计数/积累 —— 用在"第 N 次""又来了"这种**重复累积**的场合
  (四档**字号一样**, 区别在描边粗细/底衬/字重 —— 所以选哪档看"这是什么性质的话", 不看大小)
  pos 三选一: `top` / `center` / `bottom`(不写默认 center)。文字**不要**用 `x`/`y`(那是贴纸的)
  color **颜色(可选)**, 七选一 —— **按这句话的情绪选, 别每句都写**:
  * `white` 白 —— 默认/正式, 不确定就用它或干脆不写
  * `pink` 粉 —— 可爱 / 调侃 / 萌
  * `red` 红 —— 愤怒 / 警报 / 反驳 / 危险
  * `yellow` 黄 —— 吐槽旁白(系统默认就是它)
  * `cyan` 青 —— 冷静 / 疑问 / 科普
  * `green` 绿 —— 肯定 / 涨 / 通过
  * `black` 黑 —— 只在**浅色底衬**上用(chapter/comment 自带底衬); **别在没底衬的档上用**
  不写 color = 用该 style 的默认色(推荐多数情况不写, 全片五颜六色反而没重点)
  ⚠️⚠️ **定位 = 候补**（最重要的一条）：文字**很突出、指向性也强**（写出来观众一定去读它），
     滥用适得其反。只有这两种情况才写：
     ① 这一段的情绪点**没有合适的贴纸/特效**可用（抽象吐槽、要点名一个数字/名词/梗）；
     ② 这一段其它效果（贴纸/抖动/闪白…）已经**用得比较密**，再叠一个同类只会更腻 ——
        这时用一条文字换个说法，比第 4 张贴纸更像"人剪的"。
     **同一段最多 2 条**；整片也只在该被"点名"的时刻用，不要每段都来一句。
  ⚠️ **不要和贴纸同时出现**：同一时间段（±1~2 秒）已经有、或你正准备放贴纸，就**不要**写文字 ——
     两个一起上等于"一句话盖在表情包上"，比只有其中一个更乱
     （系统会把与贴纸时间重叠的文字删掉，写了也是白写）。二选一：**具象情绪用贴纸、抽象点名用文字**。
  ⚠️ **别和字幕重复**: 字幕已经在说这句话了, 就不要再写一条一样的文字元素 ——
     文字的用途是**字幕没说的那句**(标题 / 你的吐槽 / 计数), 不是复读机
  ⚠️ 一条最多 12 个字(超了会被系统按两行折, 再长就压画面了); 超过 8 个字就该删词, 别折行
- 视觉特效: {}
  {{"type":"shake","time":8.0,"end":8.3,"intensity":14,"sfx":"tag:爆発"}}
  intensity 是**像素**位移(默认 14, 常用 10~30), 0 = 不抖; **别写 0.5 这种亚像素值 —— 那等于看不出来**
- overlay: 覆盖层(氛围色). 渐变写 colors(两档起步色/终色), 纯色写 color
  {{"type":"overlay","time":0.0,"end":5.0,"overlay_type":"gradient","colors":["#ff0000","#000000"],"shape":"linear","opacity":0.3}}
  {{"type":"overlay","time":8.0,"end":10.0,"overlay_type":"color","color":"#ff0000","opacity":0.25}}
  颜色用 6 位十六进制(#rrggbb); 透明度归 opacity, 不要写进颜色里
  只写 color 不写 colors 时按「该色 → 透明」渲染, 但**优先写 colors**
  fade_in/fade_out(黑场淡入/淡出, 整块时长=渐变时长)只用于片头片尾, 片头那份系统只在初次后处理时自动补, 不要重复写
- sfx: 纯音效. {{"type":"sfx","time":3.0,"end":4.0,"name":"explosion","volume":0.5}}

选择规则:
- 音效 name/sfx 只能从"可用音效候选"中选择具体 id, 或写 tag:xxx 由系统匹配; 候选外的 id 无效
- 音效只放在情绪爆发/转折/名场面时刻, 普通叙述不要加
- 贴纸在情绪点使用; 起止时间会被系统对齐到**最近的一条字幕**（头尾都落在字幕边界上），
  你只要挑对是"哪句话"的情绪点即可; **如果提供了"可用本地贴纸包", keyword 必须从这些包名中选择**, 且必须从该包列的 tags 里选一个写进 sticker_tag; 候选外的 keyword/tag 无效
- ⚠️ **选了内建 emoji 来源时**（下面的"内建 emoji 词表"）：keyword **不要写自由英文词**，
  要写成**维度查询串**，形如 `emotion:joy,usage:punchline`（逗号分隔；同一维多个取值用 `|`，如 `usage:meme|reaction`）。
  **只能从下面那张词表里选**（词表外的词系统认不出、会退化成模糊匹配 → 选到不相干的表情）。
  最有用的是 `emotion`（情绪）与 `usage`（剪辑用途）两维，`intensity` 用来看这一下有多重（1 轻 / 2 明显 / 3 爆点）。
- 特效分两类:
  * 瞬时特效 {}:
    它们的 end-time 由系统接管, 你写不写都会被 clamp 到 0.1~0.6s; 只表现"爆发瞬间", 不要长时间驻留
  * 持续效果 camera/overlay/sticker: end 才有意义, overlay end-time 0.5~5s, sticker end-time 1.5~2.5s
- **密度下限**（2026-10-20 用户报「太平行、少内容」，这一段是**硬要求**，不是建议）:
  * **每段至少 ceil(段长秒 / 8) 个视觉事件** —— 14s 的段至少 2 个；段越长给得越多;
  * **任何连续 ≥6 秒没有视觉事件的窗口都必须补一个**（"空窗"是错误，不是风格）;
  * **camera（运镜）不算"视觉事件"** —— 全片运镜本来就是全程开着的，不许拿它凑数;
  * 视觉特效（flash / shake / pulse / blur / overlay / hue_shift）**轮换着用**,
    同一类**连续不超过 2 次**（连着三条闪白只会让人眼累）;
  * 瞬时特效的**配额放宽**了（用户 2026-10-20 口径）：**每段最多 3 次**、两次之间**至少隔 4 秒** ——
    别再用"整片只放一两次"那种稀疏写法;
  * 但**别为了凑数硬塞**：每段 0 个仍然允许，只是上面那两条下限（每 8 秒一个 / 空窗 ≤6 秒）必须守住。
- blur 只用于瞬间失焦/眩晕/被击中/闪回, 严禁长时间氛围模糊
- 运镜跟着内容走: 情绪爆发/名场面推近景("近"), 普通叙述中景("中")
- **主角是主播**: 视角移到弹幕或贴纸之后必须移回来(再写一条 anchor_to:"liver" 的事件);
  不要整段停在弹幕上
- "远"(画中画)只在尴尬、冷场、幽默反差的时刻用, **整片最多一条**（系统会兜底只留最早的一条）;
  它是**一次短暂的戏剧性停顿**: end - time 就是它在屏幕上待多久（写 1.5~3s, 到点系统会硬切回
  上一个镜头）, 别用它铺长时间; 用它时 anchor 一般写 "center"; 起点终点要落在字幕的边界上（别切在句子中间）
- 硬切(cut:true)同理: 整片偶尔一次就好, 连续硬切会让画面很跳
- **跳切**(把 cut:true 写在**被剪掉的静音间断的边界**上, 即接缝处): 这是短视频的招牌节奏手法,
  用来"跳过枯燥、制造速度感"。用法: 接缝处写一条 `cut:true` 的运镜(换景别或换聚焦点都行),
  系统会在那两帧之间制造一次构图跳。**配额**: 每段最多 3 次、两次之间至少隔 4 秒 ——
  超了系统会把多余的那条降级成平滑运镜。⚠️ 别在句子里切, 只在**接缝**上切。
  跳切之间**插一个贴纸**是教科书级的做法(它盖住跳的那一下, 观感更顺) —— 值得这么做;
  文字元素是**候补**, 别拿它当跳切的常规配套
- handheld 是**可选的附加变种**（近景专用）: 只在正想要"手持镜头/临场纪实"的紧张、沉浸时刻用,
  **不是每条近景都要摇**; 它是持续状态 —— 想切回平稳镜头就再写一条不带 handheld 的运镜
- **荷兰角**(`tilt`, 可选): 画面**持续歪一点**, 用来表达 **紧张/不安/尴尬/荒诞**。
  写 `"tilt":4.5` 这样一个小数(度, 正=顺时针, 负=逆时针)。
  ⚠️ **必须同时满足两条, 否则不要写**:
  ① **该段的情绪必须很高**(`fx` 是 `high`, 或这一段被你标成了 `punch`) ——
     荷兰角不是"随便歪一下", 它是**强情绪**的视觉隐喻; 低强度叙述里用会显得莫名其妙;
  ② 语义确实属于 紧张/不安/尴尬/荒诞/翻车 之一(最经典的是「尴尬/社死」 —— 画面"歪"= 心理失衡)。
  ⚠️ **三条硬约束**(系统会强制, 写了越界也不会生效):
  * **角度必须 ≤ 6 度** —— 超过就像拍摄事故, 系统会夹到 6;
  * **整片最多 2 处** —— 多了就不特殊了(超出的会被系统丢掉);
  * **绝对不要和画中画(zoom="远")出现在同一段** —— 画面缩小后本来就有黑边, 再转必然露出更多。
  ⚠️ 它是**持续状态**(与 handheld 同一套机制): 歪着会一直保持到下一条运镜,
  想回正就再写一条**不带 `tilt`** 的运镜。别用它铺长时间。

- 冷开场/回忆回溯（**只在真值得时才写，绝大多数段落都不该写**）: 给运镜写一个特殊的 anchor
  来表达"这一段值得在成片里**再放一次**" —— 它不是你这一段的运镜景别, 写完系统会收走:
  * `"anchor":"cold_open"` —— 「爆点提前播一次」: 把这一段**原样复制到成片最前面**当冷开场。
    只在整段就是一个**能独立成立的爆点**（几秒内信息完整、不需要前文铺垫）时才写;
    ⚠️ **整片最多一次**（系统按时间序只留最早的一条, 其余写了的会被丢掉）
  * `"anchor":"recall"` —— 「爆点后面重提」: 把它在正片之后再放一次, 做呼应/回扣。
    只在"这件事后面还要被提起"（前面埋了、后面接了梗）时才写, **整片最多两条**
  * 两者都要写满 `time`/`end`（就是被复用的那几秒, **至少 1 秒**）; 写了它**就不用**再写
    同一时间段的普通运镜了 —— 一条事件只表达一件事
  * ⚠️ 同一段里两种都想要就写两条事件（两个 anchor 各一条）; 但**别为了让片子好看而滥用**:
    整片一次冷开场 + 最多两次回忆回溯, 超出的会被丢掉, 白写
- x/y 始终写 "auto"
- **优先为每个特效/贴纸从候选清单配一个音效** (候选已按当前 clip 方向筛选, 配错风险低);
  仅当候选清单确实没有合适音效时才留空
{}
{}
{}
"##, sticker_desc, fx_list, durable_list, local_line, emoji_line, blacklist_line)
}

/// **视觉特效的类型清单** —— 从效果词汇表（`spec/effects.json`）算出来，**不许在这里手写第二份**。
///
/// 权威是 `spec/effects.json`：`scripts/check-effect-registry.mjs` 会断言它与此处、前端四张表、
/// 转译器、模板、`engine.js`、事件参考文档全部一致。加/删一个效果只改词汇表。
///
/// 解析失败时退化成"空串"而不是 panic：提示词少一行清单，比整条 LLM 链路挂掉好；
/// 而 `include_str!` 是编译期内嵌 ⇒ 真漏了在构建/门禁阶段就会暴露。
fn fx_type_list() -> String {
    effect_ids(|e| e.get("kind").and_then(|v| v.as_str()) == Some("fx"))
}
fn durable_type_list() -> String {
    effect_ids(|e| e.get("kind").and_then(|v| v.as_str()) == Some("durable"))
}

fn effect_ids(pick: impl Fn(&serde_json::Value) -> bool) -> String {
    let raw = include_str!("../../../../spec/effects.json");
    let Ok(spec) = serde_json::from_str::<serde_json::Value>(raw) else {
        return String::new();
    };
    spec.get("effects")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter(|e| pick(e))
                .filter_map(|e| e.get("id").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_default()
}

/// Step2 user prompt: 段信息 + 候选清单 + 段内字幕
pub(crate) fn build_events_user_prompt(segment: &Segment, subtitles: &[Subtitle], candidate_text: &str) -> String {
    let mut prompt = format!("段时间: [{:.1}s - {:.1}s]
", segment.start, segment.end);
    prompt.push_str(&format!("段情绪密度: {}
", segment.fx));
    // §8：把这一段的**叙事角色**告诉 LLM，并给出该角色的"配方"（规格 §3.2 那张表）。
    //
    // ⚠️ 只对 `hook` / `punch` 两档说话，**"其余"一个字都不说** ——
    //    这是有意的：「几乎不放特效（守住静，才有动的对比）」这件事
    //    **没法用一句提示词说清**，而随口一说会让 LLM 把"其余段"也当成需要表现的角色，
    //    结果又是"每段都热闹"（那正是要避免的）。
    //    不说的效果是：其余段沿用今天的行为（改动的只有被明确标了角色的那些段）。
    match segment.role() {
        "hook" => prompt.push_str(
            r#"段角色: hook（**钩子** —— 片头最抓人的那一段）
配方: 要"抓人"。可以上一条 hook 文字(text 只写关键词、不写整句) + 强音效 + 硬切运镜; 但**别把它塞满** ——
钩子的目的是让人留下来接着看, 不是让人看特效。整段 1~2 个亮点就够。
"#,
        ),
        "punch" => prompt.push_str(
            r#"段角色: punch（**爆点/名场面** —— 全片最值得被剪出来的地方）
配方: 这一段应当**最有表现力**。在预算内上足: 近景/画中画运镜 + shake + flash + 音效 + 贴纸。
⚠️ 预算仍是**硬上限**, 超了会被系统抽稀 —— 榨干配额, 但不要越界。
"#,
        ),
        _ => {}
    }
    if !candidate_text.is_empty() {
        prompt.push_str(&format!("
{}
", candidate_text));
    }
    prompt.push_str("
段内字幕:
");
    for sub in subtitles.iter().filter(|s| s.end >= segment.start - 3.0 && s.start <= segment.end + 3.0) {
        prompt.push_str(&format!("[{:.1}s-{:.1}s] {}
", sub.start, sub.end, sub.text));
    }
    prompt.push_str("
输出 JSON 数组 (不要 markdown 包裹):");
    prompt
}

/// 贴纸 keyword 语言说明（Step1/Step2 共用）
fn sticker_lang_desc(sticker_source: &[String]) -> String {
    if sticker_source.is_empty() {
        return "（不生成贴纸）".to_string();
    }
    if sticker_source.iter().any(|s| s == "emoji") {
        "(英文，如 \"laugh\" \"cry\" \"shock\" \"game\")".to_string()
    } else if sticker_source.iter().any(|s| s == "irasu") {
        "(日文优先，如「ゲーム」「驚き」「爆笑」)".to_string()
    } else {
        "(英文，如 \"laugh\" \"cry\" \"shock\" \"game\")".to_string()
    }
}

/// LLM 可能漏掉事件对象的 type 字段 (serde tagged enum 必需)。
/// 解析前按字段特征推断补全, 提高不同模型的输出容错性。
///
/// 推断规则（写的顺序就是优先级）:
/// `zoom/origin/anchor/anchor_to/cut/handheld/tilt` → camera；
/// `overlay_type/colors/shape/opacity` → overlay（后两个是 overlay 独有字段）；
/// `kind`（或 `text`+`style|pos`）→ element；`keyword` → sticker；`name` → sfx；
/// `intensity` → shake；**十六进制** `color` → overlay；其余 `color` → shake（老行为）。
/// 无法推断的保持原样（由 serde 报错）。
///
/// ⚠️ 两条容易踩空的（2026-10-19 由 B4 契约测试抓出来，见 `contract_tests.rs`）：
///
/// ① **`shape`/`opacity` 也是 overlay 独有字段**。只按 `overlay_type/colors` 认的话，
///    「纯色 + opacity」（提示词里就有这个写法）会被归成 `shake`，而 `Shake` **根本没有 color 字段**
///    ⇒ serde 把颜色**静默丢掉**（`Event` 没有 `deny_unknown_fields`）：用户在编辑器里看到
///    一条「没有颜色的抖动」，日志一个字都没有。
/// ② **十六进制 `color` 归 overlay**：提示词里 `#rrggbb` 只出现在覆盖层那一节
///    （「颜色用 6 位十六进制(#rrggbb)」），而 element 的七档是**颜色名**（white/pink/…，且一定带
///    `kind`/`text`，在本函数里排在前面）⇒ 裸 hex 不会抢走 element。
///    命名色的裸 `color` 仍按老行为归 `shake`（那种写法下 color 会被丢掉；提示词里没有这种写法，
///    这里只做记录、不改老行为）。
fn infer_event_type(obj: &serde_json::Map<String, serde_json::Value>) -> Option<&'static str> {
    if obj.contains_key("zoom")
        || obj.contains_key("origin")
        || obj.contains_key("anchor")
        || obj.contains_key("anchor_to")
        || obj.contains_key("cut")
        || obj.contains_key("handheld")
        // §19 荷兰角：只写 tilt 的运镜也该被认出来（与 handheld 同一条理由 ——
        // 附加变种常常不重复景别之外的字段）
        || obj.contains_key("tilt")
    {
        return Some("camera");
    }
    if obj.contains_key("overlay_type")
        || obj.contains_key("colors")
        || obj.contains_key("shape")
        || obj.contains_key("opacity")
    {
        return Some("overlay");
    }
    // 元素（§5）：`kind` 是它独有的分派字段，最可靠。
    //
    // ⚠️ **必须排在 `keyword` 之前**：`kind = "sticker"` 的元素既可能带 `kind`
    // 也可能带 `text`（关键词就存在 `text` 里），而**老** `sticker` 事件带的是 `keyword`。
    // 三条规则不重叠（`kind` / `keyword` / `text+style|pos`），但顺序上先认 `kind`
    // 更安全：万一将来给元素补一个 `keyword` 别名，也不会被上面那条抢走。
    if obj.contains_key("kind") {
        return Some("element");
    }
    // 兜底：只写了 `text` 的元素（LLM 漏了 `kind`）。⚠️ 判据加上 `style` 或 `pos` ——
    // **不能只看 `text`**：`sfx` 没有 `text`、但别的模型可能用 `text` 当 `name` 的别名，
    // 加上这两个元素独有的字段之后就不可能与其它类型混淆。
    if obj.contains_key("text") && (obj.contains_key("style") || obj.contains_key("pos")) {
        return Some("element");
    }
    if obj.contains_key("keyword") {
        return Some("sticker");
    }
    if obj.contains_key("name") {
        return Some("sfx");
    }
    if obj.contains_key("intensity") {
        return Some("shake");
    }
    if obj.get("color").and_then(|c| c.as_str()).is_some_and(is_hex_color) {
        return Some("overlay");
    }
    if obj.contains_key("color") {
        return Some("shake");
    }
    None
}

/// `#rgb` / `#rrggbb` / `#rrggbbaa`（提示词只要求 6 位，这里宽进）。
fn is_hex_color(s: &str) -> bool {
    let Some(rest) = s.strip_prefix('#') else {
        return false;
    };
    matches!(rest.len(), 3 | 4 | 6 | 8) && rest.chars().all(|c| c.is_ascii_hexdigit())
}

/// 给**一条**事件对象补 serde 必需的字段：`type`，以及 overlay 的 `overlay_type`。
///
/// ⚠️ `overlay_type` 在 `Overlay` 变体里是**没有 default 的必填字段**：漏写它 ⇒ 反序列化报
/// `missing field overlay_type` ⇒ **整窗事件全丢**（`apply_outputs` 走「失败不覆盖」那条路，
/// 用户看到的只是「这一段没生成事件」，而不是「少了一条覆盖层」）。
/// 所以只要这一条**是** overlay（显式 `type` 或推断出来的）就补一个合法档位：
/// 有 `colors` ⇒ `gradient`；否则 ⇒ `color`（含「只写了 color 的纯色」与「只写了 opacity」）。
fn apply_event_inference(ev: &mut serde_json::Value) {
    let Some(obj) = ev.as_object_mut() else {
        return;
    };
    if !obj.contains_key("type")
        && let Some(t) = infer_event_type(obj)
    {
        obj.insert("type".into(), serde_json::Value::String(t.into()));
    }
    let is_overlay = obj.get("type").and_then(|v| v.as_str()) == Some("overlay");
    if is_overlay && !obj.contains_key("overlay_type") {
        let kind = if obj.contains_key("colors") {
            "gradient"
        } else {
            "color"
        };
        obj.insert("overlay_type".into(), serde_json::Value::String(kind.into()));
    }
}

/// 归一化 LLM 输出的事件 JSON（`Event` 是 serde tagged enum，`type` 必需）。
///
/// **两种形状都要认**（2026-10-19，B4 实测）：
/// * `[{"events":[…]}` —— 分段形状（Step1 的输出），事件在 `events` 里；
/// * `[…]` —— **Step2 的平铺事件数组**（`EventGenTask::parse_output` 拿到的就是这个）。
///
/// ⚠️ 只认第一种的话，推断在 Step2 那条路上**一次都不会跑**：模型漏写 `type` ⇒ 直接
/// `missing field type` ⇒ 整窗判失败 ⇒ 那一段一条事件都没有，而日志里只有一句解析错误。
pub(crate) fn normalize_segments_json(json: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::Array(mut segs)) => {
            for seg in segs.iter_mut() {
                if let Some(evs) = seg.get_mut("events").and_then(|e| e.as_array_mut()) {
                    for ev in evs.iter_mut() {
                        apply_event_inference(ev);
                    }
                } else {
                    // 平铺事件数组：这一条本身就是事件
                    apply_event_inference(seg);
                }
            }
            serde_json::to_string(&segs).unwrap_or_else(|_| json.to_string())
        }
        _ => json.to_string(),
    }
}
/// Step1 system prompt: 分段 + 每段情绪密度
/// **无弹幕时按字幕语气判情绪**（与 `process::emotion_from_subtitles` **同口径**）。
///
/// ⚠️ 这份是**第二份实现**，因为 `process` 那个是私有项。两份必须一起改 ——
/// 判据见 `scripts/check-emotion-track.mjs`（它钉住"分段与徽标同一判据"）。
/// 差别只在"按谁的文本判"：那边用 `Subtitle::text`，这边一样。
/// 归窗口径也必须一致：**按中点**（`(start+end)/2`）。
pub(crate) fn emotion_from_subtitles_local(
    subs: &[Subtitle],
    window_secs: f64,
    total_duration: f64,
) -> Vec<EmotionWindow> {
    use yekitrim_core::danmaku::emotion::score_subtitle;
    if window_secs <= 0.0 || total_duration <= 0.0 {
        return Vec::new();
    }
    let n = (total_duration / window_secs).ceil() as usize;
    let mut out: Vec<EmotionWindow> = (0..n)
        .map(|i| EmotionWindow {
            start: i as f64 * window_secs,
            emotion: Emotion::default(),
            count: 0,
            hits: Vec::new(),
        })
        .collect();
    for sub in subs {
        let mid = (sub.start + sub.end) / 2.0;
        let idx = (mid / window_secs) as usize;
        let Some(w) = out.get_mut(idx) else { continue };
        let e = score_subtitle(&sub.text);
        w.emotion = w.emotion.max_with(&e);
        w.count += 1;
        if !e.is_weak() {
            w.hits.push("字幕语气".to_string());
        }
    }
    out
}

/// **弹幕情绪窗口 → 一行行可读的"情绪热区"文本**（喂给分段提示词）。
///
/// # 为什么分段要看情绪
///
/// 原来的分段**只看话题**（字幕文本）。但一段内容的"该不该切开"不只是话题变了没有 ——
/// **情绪翻面**同样是硬边界：从"铺垫"一下跳到"爆笑"，中间那刀不切，
/// 出片就是一个温吞的段里夹着一个爆点，`role`/`fx` 也没法按段给对。
///
/// ⚠️ 数据来源：**弹幕**（`emotion::track`，L1 词表 + L2 颜色），
///    没有弹幕时**退到字幕语气**（与 `process` 的 `emotion_from_subtitles` 同口径）。
///    所以这里拿到的是**与时间轴徽标同一份判据**算出来的东西 ——
///    不允许分段用一种"情绪"、界面显示另一种。
///
/// ⚠️ **只给"有明确情绪"的窗口**（`best > 0`）：四维全 0 的窗口写进提示词
///    等于告诉 LLM"这里是平静的"，而真相是"这里没数据/没判出来" —— 那是两件事。
pub(crate) fn emotion_hint_lines(windows: &[EmotionWindow], window_secs: f64) -> Vec<String> {
    /// 与前端徽标、Rust `dominant()` **同序**（平局时谁赢）。
    const ORDER: [(&str, &str); 4] = [
        ("shock", "震惊"),
        ("laugh", "爆笑"),
        ("awe", "感叹"),
        ("joy", "开心"),
    ];
    let mut out = Vec::new();
    for w in windows {
        let e: &Emotion = &w.emotion;
        let mut best = ORDER[0].0;
        let mut best_v = -1.0_f64;
        for (k, _) in ORDER.iter() {
            let v = match *k {
                "shock" => e.shock,
                "laugh" => e.laugh,
                "awe" => e.awe,
                _ => e.joy,
            };
            if v > best_v {
                best = k;
                best_v = v;
            }
        }
        // ⚠️ 阈值与 `Emotion::is_weak` 同一个（0.15）：弱信号不写进提示词，
        //    否则几十条"开心 0.16"会把真正的爆点淹掉。
        if best_v < 0.15 {
            continue;
        }
        let zh = ORDER.iter().find(|(k, _)| *k == best).map(|(_, z)| *z).unwrap_or(best);
        let hits = if w.hits.is_empty() {
            String::new()
        } else {
            // 只取前 4 个，避免一行太长把真正的重点淹掉
            format!("（{}）", w.hits.iter().take(4).cloned().collect::<Vec<_>>().join("、"))
        };
        out.push(format!(
            "{:.0}s-{:.0}s {} {:.2} x{}条 {}",
            w.start,
            w.start + window_secs,
            zh,
            best_v,
            w.count,
            hits
        ));
    }
    out
}

pub fn build_segmentation_system_prompt(sticker_source: &[String]) -> String {
    let sticker_desc = sticker_lang_desc(sticker_source);
    format!(r#"你是一个直播切片助手。将单个切片内字幕按话题切分成段。

每段输出 (JSON 对象):
{{
  "start": 0.0,
  "end": 85.0,
  "fx": "high",
  "role": "punch",
  "note": "子话题备注"
}}

分段规则:
- 目标段长 8~30 秒，整片目标段数 = ceil(视频时长/20)，允许上下浮动 1 段
- 优先在字幕间隙 >2s 或话题明显变化处切段
- 同一话题连续叙述合并，不硬切；<4s 的碎片并入前一段
- 爆点/名场面单独成段
- **情绪翻面也是切点**（与"话题变化"同等重要）：用户消息里给了「情绪热区」就是
  弹幕判出来的观众反应。**同一话题里情绪从平淡猛跳到震惊/爆笑的那一刀要切开** ——
  否则一个段里既有铺垫又有爆点，`fx` 和 `role` 给哪一档都是错的
  （给 high 则前半段过度演出，给 low 则爆点被埋掉）。
- ⚠️ **情绪热区只帮你找切点，不要照着它写 note**：note 仍然只描述"这段在讲什么"。
  也不要把每个热区都切一刀 —— 相邻热区情绪相同就并成一段
- fx: low=安静叙述/少特效, med=日常, high=爆点/密集特效
- role: 这一段在**整条片子里**干什么，三选一（不写 = 其余）:
  * `hook` —— **钩子**：片头最抓人的那一句/那一幕。**整片至多一段**，要有"不放这个观众就划走了"的强度
  * `punch` —— **爆点/名场面**：这一段是全片最值得被剪出来的地方。**整片至少要有一段**
  * （其余段**不写 role**，留空）—— 铺垫、过渡、收尾都算这一类。**不要**给每段都标 role:
    全片都是爆点等于没有爆点，而"每段都热闹"正是要避免的
- role 要靠**内容**判，不是靠 fx: fx 是"特效该多密"，role 是"它在故事里的位置"。
  两者常常同向（爆点通常 fx=high），但**不要**机械地按 fx 抄 role
- note: 只写该段在讲什么子话题，例如"发现视角不对"、"尝试换场景"，不要概括整个视频
- 不要输出 direction/budget/effects 等字段
- {} 仅在贴纸可用时说明 keyword 语言；分段本身不选贴纸
"#, sticker_desc)
}

/// Step1 user prompt: 视频类型 + 预算感知采样字幕
pub fn build_segmentation_user_prompt(
    subtitles: &[Subtitle],
    event_type: &str,
    sticker_budget: usize,
    sfx_budget: usize,
    sfx_blacklist: &[String],
    emotion_lines: &[String],
) -> String {
    let mut prompt = format!("视频类型: {}
视频时长(秒): {}
目标段数: {}
", event_type, video_duration(subtitles), target_segment_count(subtitles));

    prompt.push_str(&crate::assets::sfx_catalog::categories_text());
    if !sfx_blacklist.is_empty() {
        prompt.push_str(&format!("
音效黑名单(禁止使用): {}
", sfx_blacklist.join(", ")));
    }
    prompt.push_str(&format!("
全片预算: sticker 总数不超过 {} 个, sfx 事件总数不超过 {} 个
", sticker_budget, sfx_budget));
    // ⚠️ 情绪热区放在**字幕之前**：先让 LLM 看到"全片哪里炸"，
    //    再逐条读文本 —— 反过来会让它先入为主地按文本切完、忽略这一节。
    //    空数组 ⇒ 整节不出现（存量工程输入逐字节不变）。
    if !emotion_lines.is_empty() {
        prompt.push_str("
情绪热区 (弹幕算出的观众反应, 每行: 时间范围 情绪 强度 条数 命中词):
");
        for l in emotion_lines {
            prompt.push_str(l);
            prompt.push('\n');
        }
        prompt.push_str("
(上面只是**切点提示**: 情绪从平淡猛跳到震惊/爆笑的地方要切开; 情绪相同的相邻热区并成一段)
");
    }
    prompt.push_str("
字幕内容 (每行: [开始秒-结束秒] 文本):
");
    for sub in sample_subtitles(subtitles, 200) {
        prompt.push_str(&format!("[{:.1}s-{:.1}s] {}
", sub.start, sub.end, sub.text));
    }
    prompt.push_str("
输出 JSON 数组 (不要 markdown 包裹):");
    prompt
}

/// 预算感知采样：首尾保留，中部均匀采样到 max 条。
fn sample_subtitles<'a>(subtitles: &'a [Subtitle], max: usize) -> Vec<&'a Subtitle> {
    let n = subtitles.len();
    if n <= max {
        return subtitles.iter().collect();
    }
    let head = (max / 4).max(20);
    let tail = (max / 4).max(20);
    let head = head.min(n);
    let tail = tail.min(n.saturating_sub(head));
    let middle_max = max.saturating_sub(head + tail);
    let mut out: Vec<&Subtitle> = Vec::with_capacity(max);
    out.extend(subtitles[..head].iter());
    if middle_max > 0 {
        let middle_total = n - head - tail;
        for i in 0..middle_max {
            let idx = head + i * (middle_total - 1).max(0) / (middle_max - 1).max(1);
            out.push(&subtitles[idx]);
        }
    }
    out.extend(subtitles[n - tail..].iter());
    out
}

fn video_duration(subtitles: &[Subtitle]) -> f64 {
    subtitles.last().map(|s| s.end).unwrap_or(0.0)
}

fn target_segment_count(subtitles: &[Subtitle]) -> usize {
    let dur = video_duration(subtitles);
    ((dur / 20.0).ceil() as usize).clamp(2, 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §10 **情绪热区**进分段提示词 —— 三条边界。
    ///
    /// ⚠️ 这三条钉的是"有情绪"与"没情绪"**必须能区分**：空数组时那一节整块不出现
    /// （存量工程的 LLM 输入逐字节不变 = R4 的保证点）。
    #[test]
    fn emotion_hints_reach_segmentation_prompt() {
        use yekitrim_core::danmaku::emotion::{Emotion, EmotionWindow};
        let w = |start: f64, shock: f64, laugh: f64, cnt: u64| EmotionWindow {
            start,
            emotion: Emotion { joy: 0.0, shock, laugh, awe: 0.0 },
            count: cnt,
            hits: vec!["？？？".into()],
        };

        // ① 有窗口 ⇒ 出「情绪热区」一节，且带时间范围/中文名/强度/条数
        let lines = emotion_hint_lines(&[w(10.0, 0.9, 0.0, 12)], 10.0);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("10s-20s"), "要写窗口的**秒数范围**: {}", lines[0]);
        assert!(lines[0].contains("震惊"), "要用中文名: {}", lines[0]);
        assert!(lines[0].contains("0.90"), "要带强度: {}", lines[0]);
        assert!(lines[0].contains("12"), "要带条数: {}", lines[0]);

        let subs = vec![super::Subtitle { index: 0, start: 0.0, end: 5.0, text: "测试".into(), html: String::new() }];
        let with = build_segmentation_user_prompt(&subs, "game", 5, 5, &[], &lines);
        assert!(with.contains("情绪热区"), "提示词里必须有这一节");
        let without = build_segmentation_user_prompt(&subs, "game", 5, 5, &[], &[]);
        assert!(!without.contains("情绪热区"), "**空数组 ⇒ 整节不出现**（存量逐字节不变）");

        // ② 弱信号（< 0.15）不进提示词 —— 几十条"开心 0.16"会把真爆点淹掉
        assert!(emotion_hint_lines(&[w(0.0, 0.10, 0.05, 3)], 10.0).is_empty());
        // ③ 四维全 0 ⇒ 也不进（"没数据" ≠ "判出来是平静"）
        assert!(emotion_hint_lines(&[w(0.0, 0.0, 0.0, 0)], 10.0).is_empty());

        // ④ 主导维度按 shock > laugh > awe > joy 取（与 Rust `dominant()` 同序）
        let l = emotion_hint_lines(&[w(0.0, 0.5, 0.5, 9)], 10.0);
        assert!(l[0].contains("震惊"), "平局时 shock 赢: {}", l[0]);
    }

    /// ⚠️ **判据必须容忍"合法的不一致"**：确认系统提示词里那条规则在。
    /// （改提示词时最容易忘的就是"规则说了要按情绪切、但用户消息里没有情绪数据"。）
    #[test]
    fn segmentation_prompt_mentions_emotion_cut_point() {
        let p = build_segmentation_system_prompt(&[]);
        assert!(p.contains("情绪翻面也是切点"), "规则里必须有这条");
        assert!(p.contains("不要照着它写 note"), "要有别拿它当 note 的约束");
    }

    /// 回归测试: LLM 输出缺 type 字段时按字段特征补全 (实际失败样例)
    #[test]
    fn normalize_infers_missing_type() {
        let raw = r#"[
          {"start":0.0,"end":8.6,"fx":"low","note":"t","events":[
            {"time":0.5,"end":3.5,"zoom":"中","origin":"center"},
            {"time":8.0,"end":8.5,"intensity":0.3,"sfx":"tag:爆発"},
            {"type":"camera","time":2.0,"end":3.0,"zoom":"近","origin":"50% 50%"},
            {"time":10.0,"end":12.0,"keyword":"laugh","x":"auto","y":"auto"},
            {"time":3.0,"name":"pop","volume":0.5},
            {"time":12.0,"end":15.0,"anchor":"liver"},
            {"time":20.0,"end":24.0,"handheld":true},
            {"time":30.0,"end":34.0,"tilt":4.5}
          ]}
        ]"#;
        let norm = normalize_segments_json(raw);
        let segments: Vec<crate::project::Segment> = serde_json::from_str(&norm).unwrap();
        let evs = &segments[0].events;
        assert_eq!(evs.len(), 8);
        assert!(matches!(evs[0], crate::project::Event::Camera { .. }));
        assert!(matches!(evs[1], crate::project::Event::Shake { .. }));
        assert!(matches!(evs[2], crate::project::Event::Camera { .. }));
        assert!(matches!(evs[3], crate::project::Event::Sticker { .. }));
        assert!(matches!(evs[4], crate::project::Event::Sfx { .. }));
        // 只写 anchor（没写 zoom/origin）也要认得是 camera
        assert!(
            matches!(&evs[5], crate::project::Event::Camera { anchor: Some(a), .. } if a == "liver")
        );
        // 只写 handheld 同理（附加变种常常不重复景别之外的字段）
        assert!(
            matches!(&evs[6], crate::project::Event::Camera { handheld: true, .. }),
            "只写 handheld 的应推断成 camera: {norm}"
        );
        // §19 只写 tilt 同理 —— ⚠️ 漏了这一条的症状是"LLM 写了荷兰角、系统报
        // 「不认识的事件类型」"，而它**只在这一段生成时炸**（其它段正常），
        // 看起来像"偶发失败"。
        assert!(
            matches!(&evs[7], crate::project::Event::Camera { tilt: Some(d), .. } if (*d - 4.5).abs() < 1e-9),
            "只写 tilt 的应推断成 camera: {norm}"
        );
    }

    /// ⚠️ **§8：`role` 的配方只对 `hook`/`punch` 两档说，其余段一个字都不许加。**
    ///
    /// 反面才是重点：如果"其余段"也被塞进一段说明，LLM 会把**每一段**都当成需要表现的角色
    /// ⇒ 又回到"每段都热闹"（那正是 §8 要治的病）。
    /// 判据因此同时钉"该说的说了"与"不该说的没说"。
    #[test]
    fn role_recipe_is_only_added_for_hook_and_punch() {
        let seg = |role: &str| crate::project::Segment {
            start: 0.0,
            end: 30.0,
            fx: "med".into(),
            note: String::new(),
            role: role.into(),
            events: Vec::new(),
            extra: Default::default(),
        };
        let base_len = build_events_user_prompt(&seg(""), &[], "").len();
        let hook = build_events_user_prompt(&seg("hook"), &[], "");
        let punch = build_events_user_prompt(&seg("punch"), &[], "");
        assert!(hook.contains("hook"), "hook 段要说清角色: {hook}");
        assert!(punch.contains("punch"), "punch 段要说清角色: {punch}");
        assert!(
            hook.len() > base_len && punch.len() > base_len,
            "两档都要**多出**配方说明（{base_len} → hook {} / punch {}）",
            hook.len(),
            punch.len()
        );
        // ⚠️ 反面：其余段（空 role）与基础提示词**逐字节相同**
        assert_eq!(
            build_events_user_prompt(&seg("其他"), &[], ""),
            build_events_user_prompt(&seg(""), &[], ""),
            "认不出的 role 必须与空 role 完全等价（不许泄漏半句配方）"
        );
    }

    /// 元素（§5 E5）的**类型推断**：`kind` 独有，`text + style/pos` 是兜底。
    ///
    /// ⚠️ 这条判据的**反面**才是重点：元素与既有的 `sticker` 事件都会带"关键词"，
    /// 归错了**不会报错**（都解析成功），只会让一条文字变成贴纸、或者反过来。
    /// 所以要同时钉住"该认出来的认出来"与"不该抢的不许抢"。
    #[test]
    fn normalize_infers_element_without_stealing_sticker() {
        let raw = r#"[{"start":0.0,"end":30.0,"fx":"low","note":"","events":[
            {"kind":"text","time":3.0,"end":5.0,"text":"这就是传说中的…","style":"hook","pos":"top"},
            {"kind":"sticker","time":8.0,"end":10.0,"text":"ysyk","sticker_tag":"害怕"},
            {"time":14.0,"end":16.0,"text":"漏写了 kind","style":"comment"},
            {"time":20.0,"end":22.0,"keyword":"laugh","x":"auto","y":"auto"}
          ]}]"#;
        let norm = normalize_segments_json(raw);
        let segments: Vec<crate::project::Segment> = serde_json::from_str(&norm).unwrap();
        let evs = &segments[0].events;
        assert_eq!(evs.len(), 4);
        match &evs[0] {
            crate::project::Event::Element { kind, text, style, .. } => {
                assert_eq!(kind, "text");
                assert_eq!(text, "这就是传说中的…");
                assert_eq!(style.as_deref(), Some("hook"));
            }
            other => panic!("带 kind 的应当推断成 element，实得 {other:?}"),
        }
        match &evs[1] {
            crate::project::Event::Element { kind, text, .. } => {
                assert_eq!(kind, "sticker");
                assert_eq!(text, "ysyk", "贴纸类元素的关键词存在 `text` 里");
            }
            other => panic!("kind=sticker 的元素也该推断成 element，实得 {other:?}"),
        }
        match &evs[2] {
            crate::project::Event::Element { kind, text, .. } => {
                assert_eq!(text, "漏写了 kind");
                // ⚠️ 只写了 `text` + `style`（没有 kind）也要认 —— 认不出的症状是
                // serde 报 "missing field kind"，而 LLM 会**反复**犯这个错（提示词里
                // `kind` 与 `text` 挨着写，模型很容易只抄一个）。
                assert_eq!(kind, "", "没写 kind 时留空字符串，由 process 当认不出的值处理");
            }
            other => panic!("text+style 该推断成 element，实得 {other:?}"),
        }
        // ⚠️ **老 sticker 不许被抢走**：它带 `keyword` 且**没有** `kind`/`text`+`style`。
        // 抢走的症状是"所有存量工程里的贴纸都变成元素了"（观感可能碰巧相似，极难发现）。
        assert!(
            matches!(&evs[3], crate::project::Event::Sticker { keyword, .. } if keyword == "laugh"),
            "带 keyword 的仍必须是 sticker: {norm}"
        );
    }

    /// 已有 type 的事件不被改动
    #[test]
    fn normalize_keeps_existing_type() {
        let raw = r#"[{"start":0.0,"end":1.0,"fx":"low","note":"","events":[
            {"type":"shake","time":0.5,"intensity":0.9}
        ]}]"#;
        let norm = normalize_segments_json(raw);
        let segments: Vec<crate::project::Segment> = serde_json::from_str(&norm).unwrap();
        assert!(matches!(segments[0].events[0], crate::project::Event::Shake { .. }));
    }

    /// 提示词必须把**冷开场 / 回忆回溯的意图词**教给 LLM，且与 `process` 侧认的值**逐字一致**。
    ///
    /// 判据刻意落在"提示词里出现了 `process` 认的那两个字符串"上（而不是"出现过中文说明"）：
    /// 这两处一旦漂移，症状是 **LLM 高高兴兴写了 `anchor = "coldopen"`，`process` 一个都不认得
    /// ⇒ 一条 EDL 都不生成，而全链路零报错**（正是 `INTENT_COLD_OPEN` 常量存在的理由）。
    #[test]
    fn prompt_teaches_the_intent_anchors_process_recognizes() {
        let prompt = build_events_system_prompt(&[], &[], "");
        for (anchor, label) in [
            (crate::process::INTENT_COLD_OPEN, "冷开场"),
            (crate::process::INTENT_RECALL, "回忆回溯"),
        ] {
            assert!(
                prompt.contains(anchor),
                "提示词必须出现 `{anchor}`（{label}）—— 否则 LLM 只会写别的词，process 一条都不认"
            );
        }
        // 配额也要在提示词里明说：LLM 数不出"整片几次"，但至少要它别每段都写
        assert!(prompt.contains("最多一次") || prompt.contains("最多一条"), "提示词要写明冷开场的整片配额");
        assert!(prompt.contains("最多两条"), "提示词要写明回忆回溯的整片配额");
    }

    /// 提示词里 shake 的 `intensity` 示例必须是**像素**量级。
    ///
    /// 回归（2026-10-05 用户报的「shake 没正常触发」的主力根因）：这段示例曾写
    /// `"intensity":0.5`，而四个消费者（模板 `getShake`、`engine.js`、转译器、Rust 侧）
    /// **一律把它当像素位移**用 ⇒ 0.5 是**亚像素**，预览与成片**都看不见**，
    /// 而任务报告里照样写 `shake_events: 1` —— 全链路零报错、零可见效果。
    ///
    /// 判据故意只看"示例是不是像素量级"（>= 5px），不锁具体数字：改示例可以，
    /// 改回亚像素不行。文档口径见 `docs/polish-events-reference.md`（默认 14px）。
    /// 文字元素 = **候补**（2026-10-19 用户口径）。prompt 必须同时写清三件事 ——
    /// 只写「少用」的话模型仍会写整句吐槽、而且句句配一张贴纸：
    /// ① 填**关键词/短词组**（不是句子）；② **不要和贴纸同时出现**；③ 同一段最多 1 条。
    #[test]
    fn prompt_positions_text_element_as_a_fallback() {
        let prompt = build_events_system_prompt(&[], &[], "");
        for needle in [
            "定位 = 候补",
            "关键词/短词组",
            "不要和贴纸同时出现",
            // 2026-10-20 配额放宽：1 → 2（与 cap_text_elements 的常量同一条口径）
            "同一段最多 2 条",
        ] {
            assert!(prompt.contains(needle), "prompt 缺「{needle}」");
        }
    }

    /// **密度下限 + 放宽后的特效配额**必须写进提示词（2026-10-20 用户口径：
    /// 「太平行、少内容」+「视觉特效可以放宽一点」）。
    ///
    /// 变异自证：把提示词里的「每段最多 3 次」改回「整片最多 1 处」⇒ 本条红。
    #[test]
    fn prompt_carries_density_floor_and_relaxed_fx_quota() {
        let prompt = build_events_system_prompt(&[], &[], "");
        for needle in [
            "每段至少 ceil(段长秒 / 8) 个视觉事件",
            "≥6 秒没有视觉事件的窗口都必须补一个",
            "camera（运镜）不算",
            // ⚠️ 整句唯一：跳切那条也写着「每段最多 3 次 / 至少隔 4 秒」，
            // 只用那两个短语当针**抓不住**这一条的变异（实测过一次：改了数字仍然绿）
            "：**每段最多 3 次**、两次之间**至少隔 4 秒**",
            "连续不超过 2 次",
        ] {
            assert!(prompt.contains(needle), "prompt 缺「{needle}」");
        }
    }

    #[test]
    fn shake_prompt_example_is_pixel_scale() {
        let prompt = build_events_system_prompt(&[], &[], "");
        let example = prompt
            .lines()
            .find(|l| l.contains(r#""type":"shake""#))
            .expect("提示词里必须有 shake 示例");
        let value: f64 = example
            .split(r#""intensity":"#)
            .nth(1)
            .and_then(|rest| rest.split([',', '}']).next())
            .and_then(|v| v.trim().parse().ok())
            .expect("shake 示例里必须有 intensity 数值");
        assert!(
            value >= 5.0,
            "shake 示例的 intensity={value} 是亚像素量级（消费者按像素用 ⇒ 看不见）；\
             示例行 = {example}"
        );
        assert!(
            prompt.contains("像素"),
            "提示词里必须写明 intensity 的单位是像素"
        );
    }

    /// 提示词必须把**七档颜色预设**全教给 LLM，且**四个消费侧真源**都认这些名字。
    ///
    /// # 为什么这条判据要跨四处验（2026-10-13）
    ///
    /// 颜色这条链是"一个事实、四处实现"：
    /// ① 提示词（LLM 照它写）② `placement.rs::ElementColor::parse`（process 解析）
    /// ③ 转译器 / 预览（读 `process` 写回的第 10 段）。
    ///
    /// ⚠️ 漏任何一处的症状都是**"配了、不报错、没效果"**：
    /// * 提示词少一档 ⇒ LLM 永远不写它 ⇒ **产出恒为 0**（正是 §18-A 那个形状：
    ///   `zoom="特写"` 连着三次真实 LLM 命中 0/0/0，而模型明明在写 zoom）；
    /// * `parse` 少一档 ⇒ LLM 照着提示词写、`process` 说"认不出"并**静默退回默认色**。
    ///
    /// ⚠️ 判据**从提示词里抠出颜色名**再去问 `ElementColor::parse`，**不写第二份字面量清单** ——
    /// 两边各写一份字面量的判据拦不住漂移（这与上面 `zoom="特写"` 那条同一手法）。
    #[test]
    fn prompt_teaches_every_color_preset_the_parser_accepts() {
        use crate::process::ElementColor;
        let prompt = build_events_system_prompt(&[], &[], "");

        // ⚠️⚠️ **必须只扫「color 那一段」的档名，而且不许用 `parse` 过滤** ——
        //    第一版写的是"扫描全部 `* \`xxx\` …` 行、再用 `parse().is_some()` 过滤"，
        //    结果**变异自证当场抓住它**：往提示词里加一档 `magenta`（parse 不认）时，
        //    那一行被 `filter` 直接丢掉 ⇒ 计数仍是 7 ⇒ **判据照样绿**。
        //    而"提示词教了一档 process 不认的颜色"正是本判据要拦的两个方向之一。
        //    ⇒ 现在按**区块**取（`color` 那行之后、到 `⚠️` 那批注解之前），
        //      取到的每个名字都拿 `parse` 验，**认不出就红**。
        //
        //    ⚠️ 终止条件用 `⚠`（color 块后面紧跟的就是那批注解），**不是 `pos `** ——
        //       第一版写 `pos ` 是因为我以为 color 排在 pos 前面；实际顺序是 **pos 在前、
        //       color 在后** ⇒ `take_while` 一路吃到 shake 那一段里的
        //       `"anchor":"cold_open"` 才停，把 JSON 的字段名当成了颜色档名
        //       （**变异自证当场抓出来**：那次"复原后仍红"就是这么来的）。
        //       ⇒ **区块边界必须照提示词的真实结构写，不能凭记忆。**
        let block: Vec<&str> = prompt
            .lines()
            .skip_while(|l| !l.trim_start().starts_with("color "))
            .take_while(|l| !l.trim_start().starts_with('⚠'))
            .collect();
        assert!(
            !block.is_empty(),
            "提示词里找不到 `color` 那一段（提示词改结构了？这条判据会空转）"
        );
        let names: Vec<&str> = block
            .iter()
            .filter(|l| l.trim_start().starts_with("* `"))
            .filter_map(|l| l.split('`').nth(1))
            .collect();

        for c in ElementColor::ALL {
            assert!(
                names.contains(&c.name()),
                "提示词里没有教颜色档 `{}` —— LLM 永远写不出它，产出恒为 0（且全链路零报错）",
                c.name()
            );
        }
        // 反向：提示词里出现的**每一个**名字，`parse` 都必须认。
        for n in &names {
            assert!(
                ElementColor::parse(n).is_some(),
                "提示词教了颜色档 `{n}`，而 `parse` 不认它 —— \
                 LLM 照着提示词写，process 会判「认不出」并**静默退回默认色**"
            );
        }
        // 档数也要对上（两个方向都堵住）。
        assert_eq!(
            names.len(),
            ElementColor::ALL.len(),
            "提示词里教了 {} 档颜色，而真源有 {} 档 —— 多出来的那些 LLM 会照写、process 会报认不出：{names:?}",
            names.len(),
            ElementColor::ALL.len()
        );
    }

    /// 提示词必须把**特写**这一档教给 LLM，且教的那个词在**消费侧真源**里是独立一档。
    ///
    /// 批0.1 实测（`docs/plan/llm-production-rate.md`）：改之前连着跑三次真实 LLM，
    /// `zoom="特写"` 命中 **0 / 0 / 0**，而同三次里 `zoom` 任意值有 **20 / 19 / 21** 条 ——
    /// LLM 明明在写 zoom，只是**永远不写「特写」**。这正是 §18-A：
    /// **词不存在 ⇒ 产出恒为 0，且全链路零报错**。
    ///
    /// ⚠️ 判据刻意**不写 `"特写"` 字面量**：档位名从提示词里抠出来，
    /// 倍率喂给 `project::CameraStyle::factor`（**运镜倍率的唯一真源**）解析 ——
    /// 这样"提示词与消费侧漂移"会被直接抓住（两边各写一份字面量的判据拦不住漂移）。
    #[test]
    fn prompt_teaches_a_zoom_level_the_consumer_resolves_distinctly() {
        let prompt = build_events_system_prompt(&[], &[], "");
        let style = crate::project::CameraStyle::default();

        // 从枚举行 `zoom = "远"/"中"/"近"/"特写"（…）` 里抠出全部档位名
        let enum_line = prompt
            .lines()
            .find(|l| l.contains("zoom ="))
            .expect("提示词里必须有 zoom 的枚举行");
        let levels: Vec<&str> = enum_line.split('"').skip(1).step_by(2).collect();
        assert!(
            levels.len() >= 4,
            "zoom 枚举行至少要列出四档；解析到 {levels:?}，枚举行 = {enum_line}"
        );

        // 真源侧：近/中 各是多少（`factor` 认不出的档位会兜底成中景）
        let near = style.factor("近");
        let mid = style.factor("中");
        // 提示词里必须有**至少一档**比近景更近 ⇒ 那颗词就是"特写"，
        // 不管它叫什么、也不用在测试里重复写一遍它的名字
        let closer: Vec<(&str, f64)> = levels
            .iter()
            .map(|l| (*l, style.factor(l)))
            .filter(|(_, f)| *f > near)
            .collect();
        assert!(
            !closer.is_empty(),
            "提示词必须列出一档比近景({near})更近的景别，否则 LLM 永远写不出特写；             实测各档 = {levels:?} -> {:?}",
            levels.iter().map(|l| (*l, style.factor(l))).collect::<Vec<_>>()
        );
        assert_eq!(
            closer.len(),
            1,
            "比近景更近的档只该有一个（特写）；实得 {closer:?}"
        );
        let (closeup_label, closeup_factor) = closer[0];
        assert!(
            closeup_factor != mid,
            "特写档({closeup_factor}) 不能与中景({mid}) 同值"
        );
        // 其余档位一个都不许被 `factor` 的兜底 `_ => mid` 吞掉
        // （吞掉 = 提示词教了一个消费侧不认的词，症状正是批0.1 的"产出恒为 0"）
        for l in &levels {
            if *l == closeup_label {
                continue;
            }
            assert!(
                *l == "中" || style.factor(l) != mid,
                "档位 `{l}` 被 factor 的兜底分支吞成了中景 —— 提示词教了一个消费侧不认的词"
            );
        }

        // 示例：批0.1 的旁证是"camera 有示例 ⇒ 产出 20/19/21 次"，示例是教 LLM 用词的有效手段
        let example = prompt
            .lines()
            .find(|l| l.contains(&format!(r#""zoom":"{closeup_label}""#)))
            .expect("必须给一条特写的示例 —— 有示例的词才会被 LLM 真正用起来");
        assert!(
            example.contains(r#""anchor":"liver""#),
            "特写示例必须带 \"anchor\":\"liver\" —— 焦点守卫会把别的焦点改回主播，             教 LLM 写一个必被系统改掉的值等于白写；示例行 = {example}"
        );
        // 与近景的区别要说清（否则 LLM 会把它当 `近` 的同义词，等于没加）
        assert!(
            prompt.contains("特写比近景再近一档"),
            "提示词要说清特写与近景的区别（{closeup_factor} / {near} 是两个独立档位）"
        );
    }
}
