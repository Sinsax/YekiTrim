//! **B4：prompt ↔ parser 契约**（规格 `docs/plan/polish-refactor-spec-2026-10-19.md` §4 B4）。
//!
//! 形状：**提示词里出现的每一个字段与每一个取值** → 构造一条"LLM 真会那么写"的事件 →
//! 过**生产解析路径**（`EventGenTask::parse_output`：`extract_json_array` → `normalize_segments_json`
//! → `upgrade_event_times` → `serde_json::from_value::<Vec<Event>>`）→ 断言字段**逐值都还在**。
//!
//! # 为什么这条判据必须存在
//!
//! `Event` 是 serde tagged enum，而且**没有** `deny_unknown_fields`：字段名写错、或者
//! 类型推断把事件归错了变体，代价都是**静默丢字段**（解析成功、值没了、全链路零报错）。
//! 历史实证：提示词教 `color`、求值器只认 `colors` ⇒ 4 个项目 **8/8 覆盖层全都没画出来**。
//!
//! 每一条 [`Case`] 都写着它在提示词里的出处（`quote`），`needles` 会在同一次遍历里断言那些
//! 字面量**真的还在提示词里** —— 提示词删掉一个字段/取值（LLM 就再也写不出来、产出恒为 0
//! 且不报错）时，这里立刻红。

use serde_json::{Map, Value};

use super::events::EventGenTask;
use super::segment::{build_events_system_prompt, normalize_segments_json};
use crate::project::{Event, Segment};
use yekitrim_llm::agent::AgentTask;

// ---------------------------------------------------------------- 解析入口

fn segment() -> Segment {
    Segment {
        start: 0.0,
        end: 60.0,
        fx: "high".into(),
        note: String::new(),
        role: String::new(),
        events: Vec::new(),
        extra: Default::default(),
    }
}

/// 走**生产路径**：`EventGenTask::parse_output`。
///
/// ⚠️ 刻意不在这里抄一遍解析链：抄一份就等于这份判据有一天会和 `events.rs` 漂移，
/// 而它守的恰恰是"LLM 的合法输出不会被吞掉"。
fn parse(content: &str) -> anyhow::Result<Vec<Event>> {
    let task = EventGenTask::new(
        vec![segment()],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        String::new(),
    );
    task.parse_output(content, &[])
}

/// 模型输出 = 一个事件数组（事件提示词的原文要求）。
fn parse_events(events_json: &str) -> Vec<Event> {
    let content = format!("[{}]", events_json);
    parse(&content).unwrap_or_else(|e| {
        panic!("合法输出被解析器判死（比丢字段更糟）：{e}\n输入 = {content}")
    })
}

fn parse_one(event_json: &str) -> Event {
    let mut evs = parse_events(event_json);
    assert_eq!(evs.len(), 1, "应当恰好一条事件：{event_json}");
    evs.remove(0)
}

/// 序列化回 JSON 看字段 —— 与"落盘/转译器看到的"同一个视角。
fn obj(ev: &Event) -> Map<String, Value> {
    serde_json::to_value(ev)
        .expect("事件必须可序列化")
        .as_object()
        .expect("事件序列化出来必须是对象")
        .clone()
}

fn expected(literal: &str) -> Value {
    serde_json::from_str(literal).unwrap_or_else(|e| panic!("判据表里的期望值不是合法 JSON：{literal} ({e})"))
}

// ---------------------------------------------------------------- 契约表

/// 一条契约：提示词教了什么 → 模型会怎么写 → 解析后必须还剩什么。
struct Case {
    /// 提示词里教这一条的**那一句**（人读的锚点；红了的时候直接看它）
    quote: &'static str,
    /// 提示词里必须出现的字面量（字段名 / 取值）。缺一个 ⇒ LLM 写不出它 ⇒ 产出恒为 0
    needles: &'static [&'static str],
    /// 模型会写的事件（照提示词抄）
    json: &'static str,
    /// 期望的 `type` 标签（serde tag）
    tag: &'static str,
    /// 必须逐值保住的字段：`(字段名, 期望 JSON 字面量)`
    fields: &'static [(&'static str, &'static str)],
}

/// 相机事件：zoom / anchor / anchor_to / cut / speed / handheld / tilt / 意图 anchor。
const CAMERA: &[Case] = &[
    Case {
        quote: "zoom = \"远\"/\"中\"/\"近\"/\"特写\"（远=画面缩小成画中画）",
        needles: &["zoom", "远"],
        json: r##"{"time":1.0,"end":3.0,"zoom":"远","anchor":"center"}"##,
        tag: "camera",
        fields: &[("time", "1.0"), ("end", "3.0"), ("zoom", "\"远\""), ("anchor", "\"center\"")],
    },
    Case {
        quote: "zoom = \"远\"/\"中\"/\"近\"/\"特写\"（远=画面缩小成画中画）",
        needles: &["zoom", "中"],
        json: r##"{"time":1.0,"end":3.0,"zoom":"中","anchor":"liver"}"##,
        tag: "camera",
        fields: &[("time", "1.0"), ("end", "3.0"), ("zoom", "\"中\""), ("anchor", "\"liver\"")],
    },
    Case {
        quote: "zoom = \"远\"/\"中\"/\"近\"/\"特写\"（远=画面缩小成画中画）",
        needles: &["zoom", "近"],
        json: r##"{"time":1.0,"end":3.0,"zoom":"近","anchor":"liver"}"##,
        tag: "camera",
        fields: &[("time", "1.0"), ("end", "3.0"), ("zoom", "\"近\""), ("anchor", "\"liver\"")],
    },
    Case {
        quote: "zoom = \"远\"/\"中\"/\"近\"/\"特写\"（远=画面缩小成画中画）",
        needles: &["zoom", "特写"],
        json: r##"{"time":1.0,"end":3.0,"zoom":"特写","anchor":"liver"}"##,
        tag: "camera",
        fields: &[("time", "1.0"), ("end", "3.0"), ("zoom", "\"特写\""), ("anchor", "\"liver\"")],
    },

    // anchor = **起点**（四个取值）—— 提示词: `anchor = 这一块的**起点**…: "liver"(主播) / "danmaku"(弹幕) / ...`
    Case {
        quote: "anchor = 这一块的**起点**…: liver(主播) / danmaku(弹幕) / sticker(贴纸) / center(画面中央)",
        needles: &["anchor", "liver"],
        json: r##"{"time":1.0,"end":2.0,"anchor":"liver"}"##,
        tag: "camera",
        fields: &[("anchor", "\"liver\"")],
    },
    Case {
        quote: "anchor = 这一块的**起点**…: liver(主播) / danmaku(弹幕) / sticker(贴纸) / center(画面中央)",
        needles: &["anchor", "danmaku"],
        json: r##"{"time":1.0,"end":2.0,"anchor":"danmaku"}"##,
        tag: "camera",
        fields: &[("anchor", "\"danmaku\"")],
    },
    Case {
        quote: "anchor = 这一块的**起点**…: liver(主播) / danmaku(弹幕) / sticker(贴纸) / center(画面中央)",
        needles: &["anchor", "sticker"],
        json: r##"{"time":1.0,"end":2.0,"anchor":"sticker"}"##,
        tag: "camera",
        fields: &[("anchor", "\"sticker\"")],
    },
    Case {
        quote: "anchor = 这一块的**起点**…: liver(主播) / danmaku(弹幕) / sticker(贴纸) / center(画面中央)",
        needles: &["anchor", "center"],
        json: r##"{"time":1.0,"end":2.0,"anchor":"center"}"##,
        tag: "camera",
        fields: &[("anchor", "\"center\"")],
    },

    // anchor_to —— 提示词: `anchor_to = 这一块的**终点**(…; 不写、或写得跟起点一样 ⇒ 静止不动)`
    Case {
        quote: "anchor_to = 这一块的**终点**…; 不写、或写得跟起点一样 ⇒ 这一块**静止不动**",
        needles: &["anchor_to"],
        json: r##"{"time":5.0,"end":9.0,"zoom":"中","anchor":"liver","anchor_to":"danmaku"}"##,
        tag: "camera",
        fields: &[("anchor", "\"liver\""), ("anchor_to", "\"danmaku\"")],
    },
    // cut —— 提示词: `想硬切(切镜, 瞬间换构图、没有推移动画)再加 "cut":true`
    Case {
        quote: "想硬切(切镜: 这一块瞬间到位、没有推移动画)再加 cut:true",
        needles: &["cut"],
        json: r##"{"time":1.0,"end":2.0,"zoom":"近","cut":true}"##,
        tag: "camera",
        fields: &[("cut", "true")],
    },

    // speed 四档 —— 提示词: `想控制推进快慢加 "speed":"snap"/"fast"/"med"/"slow"`
    Case {
        quote: "想控制推进快慢加 speed:\"snap\"/\"fast\"/\"med\"/\"slow\"",
        needles: &["speed", "snap"],
        json: r##"{"time":30.0,"end":34.0,"zoom":"近","anchor":"liver","speed":"snap"}"##,
        tag: "camera",
        fields: &[("speed", "\"snap\"")],
    },
    Case {
        quote: "想控制推进快慢加 speed:\"snap\"/\"fast\"/\"med\"/\"slow\"",
        needles: &["speed", "fast"],
        json: r##"{"time":30.0,"end":34.0,"zoom":"近","anchor":"liver","speed":"fast"}"##,
        tag: "camera",
        fields: &[("speed", "\"fast\"")],
    },
    Case {
        quote: "想控制推进快慢加 speed:\"snap\"/\"fast\"/\"med\"/\"slow\"",
        needles: &["speed", "med"],
        json: r##"{"time":30.0,"end":34.0,"zoom":"近","anchor":"liver","speed":"med"}"##,
        tag: "camera",
        fields: &[("speed", "\"med\"")],
    },
    Case {
        quote: "想控制推进快慢加 speed:\"snap\"/\"fast\"/\"med\"/\"slow\"",
        needles: &["speed", "slow"],
        json: r##"{"time":30.0,"end":34.0,"zoom":"近","anchor":"liver","speed":"slow"}"##,
        tag: "camera",
        fields: &[("speed", "\"slow\"")],
    },

    // handheld —— 提示词: `近景想要"手持相机"的临场晃动感, 再加 "handheld":true`
    Case {
        quote: "近景想要手持相机的临场晃动感, 再加 handheld:true",
        needles: &["handheld"],
        json: r##"{"time":20.0,"end":24.0,"zoom":"近","anchor":"liver","handheld":true}"##,
        tag: "camera",
        fields: &[("handheld", "true")],
    },
    // tilt —— 提示词: `写 "tilt":4.5 这样一个小数(度, 正=顺时针, 负=逆时针)`
    Case {
        quote: "写 tilt:4.5 这样一个小数(度, 正=顺时针, 负=逆时针)",
        needles: &["tilt"],
        json: r##"{"time":30.0,"end":34.0,"tilt":4.5}"##,
        tag: "camera",
        fields: &[("tilt", "4.5")],
    },
    Case {
        quote: "负=逆时针",
        needles: &["tilt"],
        json: r##"{"time":30.0,"end":34.0,"tilt":-4.5}"##,
        tag: "camera",
        fields: &[("tilt", "-4.5")],
    },

    // 意图 anchor（冷开场 / 回忆回溯）—— 提示词: `"anchor":"cold_open"` / `"anchor":"recall"`
    Case {
        quote: "anchor:cold_open —— 爆点提前播一次",
        needles: &["cold_open"],
        json: r##"{"time":1.0,"end":3.0,"zoom":"中","anchor":"cold_open"}"##,
        tag: "camera",
        fields: &[("anchor", "\"cold_open\"")],
    },
    Case {
        quote: "anchor:recall —— 爆点后面重提",
        needles: &["recall"],
        json: r##"{"time":1.0,"end":3.0,"zoom":"中","anchor":"recall"}"##,
        tag: "camera",
        fields: &[("anchor", "\"recall\"")],
    },
];

/// 贴纸 / 文字元素（`element`）/ 瞬时特效 / 覆盖层 / 纯音效。
const OTHERS: &[Case] = &[
    // sticker —— 提示词: `{"type":"sticker",...,"keyword":"ysyk","sticker_tag":"害怕","x":"auto","y":"auto"}`
    Case {
        quote: "sticker: keyword + sticker_tag + x/y 始终写 auto",
        needles: &["sticker", "keyword", "sticker_tag", "x", "y"],
        json: r##"{"type":"sticker","time":10.0,"end":12.0,"keyword":"ysyk","sticker_tag":"害怕","x":"auto","y":"auto"}"##,
        tag: "sticker",
        fields: &[
            ("time", "10.0"),
            ("end", "12.0"),
            ("keyword", "\"ysyk\""),
            ("sticker_tag", "\"害怕\""),
            ("x", "\"auto\""),
            ("y", "\"auto\""),
        ],
    },
    // 只写 keyword（漏 type）—— 提示词那句"不要写 duration"旁边的老坑：模型常漏 type
    Case {
        quote: "（漏写 type 的兜底：keyword 只可能是贴纸）",
        needles: &["keyword"],
        json: r##"{"time":14.0,"end":16.0,"keyword":"laugh","x":"auto","y":"auto"}"##,
        tag: "sticker",
        fields: &[("keyword", "\"laugh\"")],
    },
    // element：kind=text + text + style + pos + color —— 提示词 e.g. hook/pos/color 那几段
    Case {
        quote: "element 例: {kind:text,...,text:名场面,style:hook,pos:top,color:pink}",
        needles: &["element", "kind", "text", "style", "pos", "color"],
        json: r##"{"type":"element","kind":"text","time":30.0,"end":32.4,"text":"名场面","style":"hook","pos":"top","color":"pink"}"##,
        tag: "element",
        fields: &[
            ("kind", "\"text\""),
            ("text", "\"名场面\""),
            ("style", "\"hook\""),
            ("pos", "\"top\""),
            ("color", "\"pink\""),
        ],
    },
    // element 的四个 style —— 提示词: `style 四选一: hook / chapter / comment / count`
    Case {
        quote: "style 四选一: hook / chapter / comment / count",
        needles: &["hook"],
        json: r##"{"kind":"text","time":30.0,"end":32.0,"text":"x","style":"hook"}"##,
        tag: "element",
        fields: &[("style", "\"hook\"")],
    },
    Case {
        quote: "style 四选一: hook / chapter / comment / count",
        needles: &["chapter"],
        json: r##"{"kind":"text","time":30.0,"end":32.0,"text":"x","style":"chapter"}"##,
        tag: "element",
        fields: &[("style", "\"chapter\"")],
    },
    Case {
        quote: "style 四选一: hook / chapter / comment / count",
        needles: &["comment"],
        json: r##"{"kind":"text","time":30.0,"end":32.0,"text":"x","style":"comment"}"##,
        tag: "element",
        fields: &[("style", "\"comment\"")],
    },
    Case {
        quote: "style 四选一: hook / chapter / comment / count",
        needles: &["count"],
        json: r##"{"kind":"text","time":30.0,"end":32.0,"text":"x","style":"count"}"##,
        tag: "element",
        fields: &[("style", "\"count\"")],
    },
    // element 的三个 pos —— 提示词: `pos 三选一: top / center / bottom(不写默认 center)`
    Case {
        quote: "pos 三选一: top / center / bottom(不写默认 center)",
        needles: &["top"],
        json: r##"{"kind":"text","time":30.0,"end":32.0,"text":"x","pos":"top"}"##,
        tag: "element",
        fields: &[("pos", "\"top\"")],
    },
    Case {
        quote: "pos 三选一: top / center / bottom(不写默认 center)",
        needles: &["center"],
        json: r##"{"kind":"text","time":30.0,"end":32.0,"text":"x","pos":"center"}"##,
        tag: "element",
        fields: &[("pos", "\"center\"")],
    },
    Case {
        quote: "pos 三选一: top / center / bottom(不写默认 center)",
        needles: &["bottom"],
        json: r##"{"kind":"text","time":30.0,"end":32.0,"text":"x","pos":"bottom"}"##,
        tag: "element",
        fields: &[("pos", "\"bottom\"")],
    },
    // element 的七档 color —— 提示词: `color **颜色(可选)**, 七选一`
    Case {
        quote: "color 七选一 —— 按这句话的情绪选",
        needles: &["color", "white"],
        json: r##"{"kind":"text","time":30.0,"end":32.0,"text":"x","color":"white"}"##,
        tag: "element",
        fields: &[("color", "\"white\"")],
    },
    Case {
        quote: "color 七选一 —— 按这句话的情绪选",
        needles: &["color", "pink"],
        json: r##"{"kind":"text","time":30.0,"end":32.0,"text":"x","color":"pink"}"##,
        tag: "element",
        fields: &[("color", "\"pink\"")],
    },
    Case {
        quote: "color 七选一 —— 按这句话的情绪选",
        needles: &["color", "red"],
        json: r##"{"kind":"text","time":30.0,"end":32.0,"text":"x","color":"red"}"##,
        tag: "element",
        fields: &[("color", "\"red\"")],
    },
    Case {
        quote: "color 七选一 —— 按这句话的情绪选",
        needles: &["color", "yellow"],
        json: r##"{"kind":"text","time":30.0,"end":32.0,"text":"x","color":"yellow"}"##,
        tag: "element",
        fields: &[("color", "\"yellow\"")],
    },
    Case {
        quote: "color 七选一 —— 按这句话的情绪选",
        needles: &["color", "cyan"],
        json: r##"{"kind":"text","time":30.0,"end":32.0,"text":"x","color":"cyan"}"##,
        tag: "element",
        fields: &[("color", "\"cyan\"")],
    },
    Case {
        quote: "color 七选一 —— 按这句话的情绪选",
        needles: &["color", "green"],
        json: r##"{"kind":"text","time":30.0,"end":32.0,"text":"x","color":"green"}"##,
        tag: "element",
        fields: &[("color", "\"green\"")],
    },
    Case {
        quote: "color 七选一 —— 按这句话的情绪选",
        needles: &["color", "black"],
        json: r##"{"kind":"text","time":30.0,"end":32.0,"text":"x","color":"black"}"##,
        tag: "element",
        fields: &[("color", "\"black\"")],
    },
    // element（kind 漏写，只有 text+style）—— 提示词里 kind 与 text 挨着，模型常只抄一个
    Case {
        quote: "（漏写 kind 的兜底：text + style|pos 只可能是文字元素）",
        needles: &["text"],
        json: r##"{"time":14.0,"end":16.0,"text":"漏写了 kind","style":"comment"}"##,
        tag: "element",
        fields: &[("text", "\"漏写了 kind\""), ("kind", "\"\"")],
    },
];


/// 七个瞬时特效 + 纯音效 + 覆盖层（含两个踩过的坑）。
///
/// 七个 fx 的 id 由 `fx_type_list()` 从 `spec/effects.json` 生成，提示词里出现在
/// `- 视觉特效: shake/flash/…` 那一行；这里钉的是"照那一行写的事件能解析回同名变体 + 字段不丢"。
const FX_SFX_OVERLAY: &[Case] = &[
    // shake —— 提示词: `{"type":"shake","time":8.0,"end":8.3,"intensity":14,"sfx":"tag:爆発"}`
    Case {
        quote: "视觉特效例: shake + intensity(像素) + sfx:tag:爆発",
        needles: &["shake", "intensity", "sfx"],
        json: r##"{"type":"shake","time":8.0,"end":8.3,"intensity":14,"sfx":"tag:爆発"}"##,
        tag: "shake",
        fields: &[("intensity", "14.0"), ("sfx", "\"tag:爆発\"")],
    },
    Case {
        quote: "音效 name/sfx 只能从候选里选具体 id, 或写 tag:xxx 由系统匹配",
        needles: &["sfx"],
        json: r##"{"type":"shake","time":8.0,"end":8.3,"intensity":14,"sfx":"explosion"}"##,
        tag: "shake",
        fields: &[("sfx", "\"explosion\"")],
    },
    Case {
        quote: "视觉特效: flash（颜色可选）",
        needles: &["flash"],
        json: r##"{"type":"flash","time":1.0,"end":1.4,"color":"#ffffff"}"##,
        tag: "flash",
        fields: &[("color", "\"#ffffff\"")],
    },
    Case {
        quote: "视觉特效: blur（只用于瞬间失焦/眩晕/被击中/闪回）",
        needles: &["blur"],
        json: r##"{"type":"blur","time":1.0,"end":1.4}"##,
        tag: "blur",
        fields: &[("time", "1.0"), ("end", "1.4")],
    },
    Case {
        quote: "视觉特效: vignette",
        needles: &["vignette"],
        json: r##"{"type":"vignette","time":1.0,"end":1.4}"##,
        tag: "vignette",
        fields: &[("time", "1.0"), ("end", "1.4")],
    },
    Case {
        quote: "视觉特效: hue_shift（颜色可选）",
        needles: &["hue_shift"],
        json: r##"{"type":"hue_shift","time":1.0,"end":1.4,"color":"#00ff00"}"##,
        tag: "hue_shift",
        fields: &[("color", "\"#00ff00\"")],
    },
    Case {
        quote: "视觉特效: noise",
        needles: &["noise"],
        json: r##"{"type":"noise","time":1.0,"end":1.4}"##,
        tag: "noise",
        fields: &[("time", "1.0"), ("end", "1.4")],
    },
    Case {
        quote: "视觉特效: pulse",
        needles: &["pulse"],
        json: r##"{"type":"pulse","time":1.0,"end":1.4}"##,
        tag: "pulse",
        fields: &[("time", "1.0"), ("end", "1.4")],
    },
    // 纯音效 —— 提示词: `{"type":"sfx","time":3.0,"end":4.0,"name":"explosion","volume":0.5}`
    Case {
        quote: "sfx 例: name:explosion + volume:0.5",
        needles: &["sfx", "name", "volume"],
        json: r##"{"type":"sfx","time":3.0,"end":4.0,"name":"explosion","volume":0.5}"##,
        tag: "sfx",
        fields: &[("name", "\"explosion\""), ("volume", "0.5")],
    },
    // 漏 type 的兜底（`name` 只可能是音效）
    Case {
        quote: "音效也能写 tag:xxx（漏 type 时按 name 兜底）",
        needles: &["tag:"],
        json: r##"{"time":3.0,"end":4.0,"name":"tag:爆発"}"##,
        tag: "sfx",
        fields: &[("name", "\"tag:爆発\"")],
    },
    // 覆盖层：colors 数组（踩过的坑 #1 —— 提示词曾只教 color，四处求值器只认 colors，实测 8/8 没画出来）
    Case {
        quote: "overlay 渐变例: overlay_type:gradient + colors 数组 + shape + opacity",
        needles: &["overlay", "overlay_type", "gradient", "colors", "shape", "opacity"],
        json: r##"{"type":"overlay","time":0.0,"end":5.0,"overlay_type":"gradient","colors":["#ff0000","#000000"],"shape":"linear","opacity":0.3}"##,
        tag: "overlay",
        fields: &[
            ("overlay_type", "\"gradient\""),
            ("colors", "[\"#ff0000\",\"#000000\"]"),
            ("shape", "\"linear\""),
            ("opacity", "0.3"),
        ],
    },
    // 覆盖层：纯色写法（坑 #1 的另一半 —— 提示词明确写了"纯色写 color"）
    Case {
        quote: "overlay 纯色例: overlay_type:color + color + opacity",
        needles: &["color", "0.25"],
        json: r##"{"type":"overlay","time":8.0,"end":10.0,"overlay_type":"color","color":"#ff0000","opacity":0.25}"##,
        tag: "overlay",
        fields: &[
            ("overlay_type", "\"color\""),
            ("color", "\"#ff0000\""),
            ("opacity", "0.25"),
        ],
    },
];

// ---------------------------------------------------------------- 遍历判据

/// **契约表全量遍历**：提示词里有出处 → 模型照写 → 解析后逐值还在。
///
/// 这是 B4 的主判据。任何一条 `Case` 红了，都意味着"提示词教了一个 LLM 会写、而解析器会丢的东西"。
#[test]
fn every_documented_field_and_value_survives_the_parser() {
    let prompt = build_events_system_prompt(&[], &[], "");
    let mut checked = 0usize;
    for case in CAMERA.iter().chain(OTHERS).chain(FX_SFX_OVERLAY) {
        // ⚠️ 文字层退役（2026-10-20）：提示词不再教 kind=text 及其 style/pos/color。
        if case.json.contains("\"kind\":\"text\"") || case.quote.contains("element") {
            continue;
        }
        // ⚠️ 已退役的瞬时特效（pulse / hue_shift，2026-10-20 并入 overlay）：提示词不再教，
        // 老工程靠读时迁移/丢弃；解析器仍认得，只是不再有提示词契约。
        if case.tag == "pulse" || case.tag == "hue_shift" {
            continue;
        }
        // ⚠️ **文字层已退役**（2026-10-20 用户口径：「算了，删除文字层吧」）：提示词不再教
        // `kind="text"`（及其 `style`/`pos`/`color` 三段），表里那几条随之跳过 ——
        // 它们断言的是"提示词里还教着这一档"，而那一档已经删了。
        // ⚠️ 枚举变体仍在（老文件解析），所以**解析器**那半边没有被删，只是不再有提示词契约。
        if case.json.contains("\"kind\":\"text\"") || case.quote.contains("element") {
            continue;
        }
        assert!(!case.quote.is_empty(), "每条契约都要写明提示词出处：{}", case.json);
        assert!(!case.needles.is_empty(), "每条契约都要有提示词锚点：{}", case.json);
        for n in case.needles {
            assert!(
                prompt.contains(n),
                "提示词里已经没有 `{n}`（出处：{}）—— LLM 再也写不出它 ⇒ 产出恒为 0 且全链路零报错",
                case.quote
            );
        }
        let ev = parse_one(case.json);
        let o = obj(&ev);
        assert_eq!(
            o.get("type").and_then(|v| v.as_str()),
            Some(case.tag),
            "变体归错了（出处：{}）：{}\n解析后 = {}",
            case.quote,
            case.json,
            Value::Object(o.clone())
        );
        for (k, want) in case.fields {
            assert_eq!(
                o.get(*k),
                Some(&expected(want)),
                "字段 `{k}` 丢了或被改了（出处：{}）：{}\n解析后 = {}",
                case.quote,
                case.json,
                Value::Object(o.clone())
            );
        }
        checked += 1;
    }
    // 45 → **34**：文字层退役时表里 `kind="text"` 那 11 条不再参与（2026-10-20）。
    // ⚠️ 下限仍然要有 —— 表被删空时这条会红（"判据空转"的兜底）。
    // 34 -> 32: pulse / hue_shift 也已退役并入 overlay（2026-10-20），那两条不再有提示词契约。
    assert!(checked >= 32, "契约条目只有 {checked} 条 —— 表被删空了？");
}

/// 提示词里列出的**每一个效果 id** 都必须能解析成同名变体。
///
/// 方向是"提示词教了它 ⇒ 解析器必须认"：教了一个解析器不认的 id，症状是**整条窗口解析失败**
/// （`apply_outputs` 跳过 ⇒ 这一段一条事件都没有），而日志里只有一句 `unknown variant`。
#[test]
fn prompt_effect_lists_only_contain_ids_the_parser_knows() {
    let prompt = build_events_system_prompt(&[], &[], "");
    let mut ids: Vec<String> = Vec::new();
    for line in prompt.lines() {
        let l = line.trim_start();
        // `- 视觉特效: shake/flash/…`、`* 瞬时特效 …`、`* 持续效果 …`
        // ⚠️ 这几行的**标签眼下是错位的**（`- 视觉特效:` 打印 fx 清单；`* 瞬时特效` 那一行
        //    打印的却是 durable 清单）—— 那是提示词正文的问题（B4 只报告、不改正文，见交付摘要）；
        //    这里按"提示词里出现过的 id"取，不按标签取。
        if l.starts_with("- 视觉特效:") || l.starts_with("* 瞬时特效") || l.starts_with("* 持续效果") {
            // 只认**斜杠列表**（`a/b/c`）：这三行里还散着别的英文词（`end`、`overlay`…），
            // 按"含 / 的词"取才不会把说明文字当成 id。
            // ⚠️ 清单的位置两行不一样：`- 视觉特效:` 在冒号**后**，`* 瞬时特效 …:` 在冒号**前** ——
            //    所以按空白切、不看冒号。
            for word in l.split_whitespace().filter(|w| w.contains('/')) {
                for id in word.split('/') {
                    let id = id.trim().trim_end_matches(':').trim();
                    if !id.is_empty() && !ids.iter().any(|x| x == id) {
                        ids.push(id.to_string());
                    }
                }
            }
        }
    }
    assert!(ids.len() >= 10, "提示词里的效果 id 少于 10 个（扫描失效了？）：{ids:?}");
    for id in &ids {
        let payload = minimal_event(id);
        let ev = parse_one(&payload);
        assert_eq!(
            obj(&ev).get("type").and_then(|v| v.as_str()),
            Some(id.as_str()),
            "提示词教了 `{id}`，解析出来的却是别的变体（整窗会解析失败）：{payload}"
        );
    }
}

/// 各变体的**必填字段**（serde 没有 default 的那些）。
fn minimal_event(id: &str) -> String {
    match id {
        // shake: intensity 无 default
        "shake" => r##"{"type":"shake","time":1.0,"end":1.4,"intensity":14}"##.to_string(),
        // sticker: keyword/x/y 无 default
        "sticker" => {
            r##"{"type":"sticker","time":1.0,"end":1.4,"keyword":"laugh","x":"auto","y":"auto"}"##.to_string()
        }
        // element: text 无 default
        "element" => r##"{"type":"element","kind":"text","time":1.0,"end":1.4,"text":"x"}"##.to_string(),
        // overlay: overlay_type 无 default
        "overlay" => {
            r##"{"type":"overlay","time":1.0,"end":1.4,"overlay_type":"color","color":"#ff0000"}"##.to_string()
        }
        other => format!(r##"{{"type":"{other}","time":1.0,"end":1.4}}"##),
    }
}

// --------------------------------------------- 另一条路：分段（Step1）的形状

/// `normalize_segments_json` 对**分段形状**（`[{"events":[…]}]`）的类型推断 —— 与上面那条
/// 平铺事件的路**不是同一个分支**，两条都要有判据（只钉一条的话，另一条烂掉没人知道）。
#[test]
fn segment_shaped_payload_also_infers_types() {
    let norm = normalize_segments_json(
        r##"[{"start":0.0,"end":60.0,"fx":"high","note":"","events":[
            {"time":1.0,"end":2.0,"zoom":"近","anchor":"liver"},
            {"time":3.0,"end":4.0,"keyword":"laugh","x":"auto","y":"auto"},
            {"time":5.0,"end":6.0,"kind":"text","text":"名场面","style":"hook"},
            {"time":7.0,"end":8.0,"colors":["#ff0000","#000000"],"overlay_type":"gradient"}
          ]}]"##,
    );
    let segs: Vec<Segment> = serde_json::from_str(&norm).expect("分段形状必须能解析");
    let tags: Vec<String> = segs[0]
        .events
        .iter()
        .map(|e| {
            obj(e)
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    assert_eq!(tags, vec!["camera", "sticker", "element", "overlay"], "推断结果 = {tags:?}");
}

// ------------------------------------------------- 踩过的坑：覆盖层三件事

/// **坑 #2：`overlay_type` 漏写时的推断**（写 `colors` 的那一支）。
///
/// 提示词要求写 `overlay_type`，但模型漏写是常态；漏写时 `Overlay` 变体**没有 default**，
/// 直接反序列化会报 `missing field overlay_type` ⇒ **整窗事件全丢**（不是少一条）。
#[test]
fn overlay_with_colors_but_no_overlay_type_is_still_an_overlay() {
    let ev = parse_one(r##"{"time":0.0,"end":5.0,"colors":["#ff0000","#000000"],"opacity":0.3}"##);
    let o = obj(&ev);
    assert_eq!(
        o.get("type").and_then(|v| v.as_str()),
        Some("overlay"),
        "带 colors 的必须是覆盖层：{o:?}"
    );
    assert_eq!(o.get("colors"), Some(&expected(r##"["#ff0000","#000000"]"##)), "colors 不许丢：{o:?}");
    assert!(
        o.get("overlay_type").and_then(|v| v.as_str()).is_some_and(|t| !t.is_empty()),
        "漏写 overlay_type 时必须补一个合法档位（否则整窗解析失败）：{o:?}"
    );
}

/// **坑 #2 的另一半：纯色覆盖层**（提示词"纯色写 color"那一句）。
///
/// ⚠️ 这一支最阴：`color` 是 shake/flash/hue_shift 也有的字段名，而 `Shake` **根本没有 color 字段**
/// ⇒ 归成 shake 的话 `color` 会被 serde **静默丢掉**（没有 `deny_unknown_fields`），
/// 用户在编辑器里看到一条"没有颜色的抖动"，而日志一个字都没有。
/// 判据因此同时钉"必须是 overlay"与"颜色值还在"。
#[test]
fn solid_color_overlay_without_overlay_type_keeps_its_color() {
    let ev = parse_one(r##"{"time":8.0,"end":10.0,"color":"#ff0000","opacity":0.25}"##);
    let o = obj(&ev);
    assert_eq!(
        o.get("type").and_then(|v| v.as_str()),
        Some("overlay"),
        "只写 color+opacity 的应当是覆盖层（归成 shake 会静默丢 color）：{o:?}"
    );
    assert_eq!(o.get("color"), Some(&expected(r##""#ff0000""##)), "颜色不许丢：{o:?}");
    assert_eq!(o.get("opacity"), Some(&expected("0.25")), "opacity 不许丢：{o:?}");
}

/// **坑 #2 的第三支：只写了 `colors`（连 `opacity` 都没有）**。
///
/// ⚠️ 这条是**变异自证逼出来的**：上面那条 `colors+opacity` 的用例会被"opacity 也归 overlay"
/// 那条规则兜住 ⇒ **单独删掉 `colors` 分支它照样绿**（实测：那一处变异 0 个测试变红）。
/// 所以这一条**故意只写 colors** —— 只要 `colors` 那一支没了，它必红。
#[test]
fn overlay_with_colors_only_is_inferred() {
    let ev = parse_one(r##"{"time":0.0,"end":5.0,"colors":["#ff0000","#000000"]}"##);
    let o = obj(&ev);
    assert_eq!(
        o.get("type").and_then(|v| v.as_str()),
        Some("overlay"),
        "带 colors 的必须是覆盖层：{o:?}"
    );
    assert_eq!(o.get("colors"), Some(&expected(r##"["#ff0000","#000000"]"##)), "colors 不许丢：{o:?}");
    assert_eq!(
        o.get("overlay_type").and_then(|v| v.as_str()),
        Some("gradient"),
        "有 colors 时缺省档位应当是 gradient：{o:?}"
    );
}

/// **坑 #3：`fade_in` / `fade_out` 不铺色档**。
///
/// 提示词: `fade_in/fade_out(黑场淡入/淡出, 整块时长=渐变时长)只用于片头片尾`。
/// 这两档的黑是**渲染器按 overlay_type 自己画的**（整窗线性斜坡），不来自 `color`/`colors` ——
/// 所以解析侧**不许**给它们"补"一个颜色档（补了就等于把黑场变成一个色块）。
/// 反过来，用户/模型**显式**写了颜色也要原样保留（那是数据，不是解析器该删的）。
#[test]
fn fade_overlays_do_not_get_color_stops_invented() {
    for kind in ["fade_in", "fade_out"] {
        let ev = parse_one(&format!(
            r##"{{"type":"overlay","time":0.0,"end":2.0,"overlay_type":"{kind}"}}"##
        ));
        let o = obj(&ev);
        assert_eq!(o.get("overlay_type").and_then(|v| v.as_str()), Some(kind), "{o:?}");
        // ⚠️ `Overlay` 的 color/colors **没有** `skip_serializing_if` ⇒ 缺省写成 `null`，
        //    所以判据是"空"（缺键或 null），不是"键不存在"。
        let blank = |o: &Map<String, Value>, k: &str| o.get(k).map_or(true, |v| v.is_null());
        assert!(blank(&o, "colors"), "{kind} 不该被补出 colors：{o:?}");
        assert!(blank(&o, "color"), "{kind} 不该被补出 color：{o:?}");
    }
    // 显式写了颜色 ⇒ 原样保留（解析器不删数据）
    let ev = parse_one(
        r##"{"type":"overlay","time":0.0,"end":2.0,"overlay_type":"fade_in","color":"#101010"}"##,
    );
    assert_eq!(obj(&ev).get("color"), Some(&expected(r##""#101010""##)), "显式颜色被删了");
}

// ------------------------------------------------- 反向：不许把垃圾也吞掉

/// 容错**只针对"提示词教过的字段"**：什么线索都没有的事件必须**报错**，
/// 不许被静默丢掉（丢掉的症状是"这一段少了一条事件"，而日志零输出）。
#[test]
fn event_without_type_and_without_clues_still_errors() {
    let err = parse(r##"[{"time":1.0,"end":2.0}]"##).expect_err("无 type 无线索的事件必须报错");
    assert!(!format!("{err}").is_empty(), "报错信息不能是空的");
}

/// 认不出的 `type` 同理：**不静默**。
#[test]
fn unknown_event_type_still_errors() {
    assert!(parse(r##"[{"type":"nope","time":1.0,"end":2.0}]"##).is_err(), "认不出的 type 必须报错");
}

