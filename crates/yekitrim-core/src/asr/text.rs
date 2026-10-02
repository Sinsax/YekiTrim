//! 字幕文本后处理（断句**之后**、落盘**之前**）。
//!
//! 两条规则都只改**外观**，既不改断句也不改时间：
//!
//! 1. **行尾只留带情绪的符号**：`？！!?` 留着；`。` `.` 以及 `，、；：,;:` 从行尾去掉
//!    —— 字幕本来就短，行尾一个句号只是噪声，而问号/感叹号带着情绪，删了意思就变。
//!    `…` `～` 保留（语气/拖长音，不是中性标点）。
//! 2. **转述句式去引号**：`xx说：“xxxx”` → `xx说，xxxx`。引号在字幕里只占宽度、不增加理解；
//!    引号前的冒号跟着变逗号（后面不再是"原文引用"了）。
//!
//! 入口只有一个：`normalize`。**别在别处再写一份** —— 步骤顺序错了就会留下 `。，` 这种叠写
//! （引号被删掉后，原本包在引号外的逗号会顶到句号后面）。
//!
//! 真实用例（`media/2026-09-02_20-53-08-367_最鱿鱼之人/output/transcript.json:1584`）：
//! `他他说：“他说没有。”，他说：“他是小朋友。”` → `他他说，他说没有。他说，他是小朋友`

/// 行尾**保留**的符号：带情绪的，删了意思就变。
const EMOTIVE_TAIL: [char; 4] = ['！', '？', '!', '?'];

/// 行尾**去掉**的中性标点。`，、；：` 也在内：字幕行以它们结尾看着像没说完。
const NEUTRAL_TAIL: [char; 8] = ['。', '.', '，', ',', '、', '；', ';', '：'];

/// 引号类字符，一律去掉。
///
/// 不含 ASCII 单引号：英文缩写（`don't`）在内容里，删了会改词。
const QUOTES: [char; 9] = ['“', '”', '‘', '’', '「', '」', '『', '』', '"'];

/// 冒号：后面紧跟引号时改写成逗号。
const COLONS: [char; 2] = ['：', ':'];

/// 字幕文本规范化。
///
/// 空/纯符号的输入原样返回 —— **绝不产出空字幕**（只有引号的条目删完就成了空串，
/// 那条字数里的内容就丢了）。
pub fn normalize(text: &str) -> String {
    let out = strip_neutral_tail(&rewrite_dialogue(text));
    if out.trim().is_empty() {
        return text.trim().to_string();
    }
    out
}

/// 截掉生成标记之后的内容：**在第一个 `<\|` 处断开**。
///
/// 为什么要单独一步、而且要在**切分之前**做：qwen3 喂到音乐/噪声段时会吐出
/// `<|endoftext|>` 然后**接着胡说**，实测形态
/// `我都孤。<|endoftext|>Human Rights Watch（人权观察）是美国一家非政府组织…`（71 字、0.3 秒）。
/// 不截的话那串百科文会变成一条独立字幕；实测 4 个项目里 3 个命中，
/// 而且已经漏进成片（`clips/clip-29`、`clips/clip-35` 的 `clip.srt`）。
///
/// 用 `<|` 而不是只认 `endoftext`：那是所有特殊 token 的共同前缀，而这两个字符
/// 在真实语音转写里几乎不可能出现。
pub fn strip_generation_markers(text: &str) -> String {
    match text.find("<|") {
        Some(idx) => text[..idx].trim_end().to_string(),
        None => text.trim().to_string(),
    }
}

/// 去掉引号；引号前的冒号改成逗号；删引号后露出来的叠写标点收敛掉。
fn rewrite_dialogue(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if QUOTES.contains(&c) {
            i += 1;
            continue;
        }
        // `说：“…”` → `说，` + 引号（引号在下一轮被跳过）
        if COLONS.contains(&c) {
            let mut j = i + 1;
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            if j < chars.len() && QUOTES.contains(&chars[j]) {
                out.push('，');
                i = j;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    collapse_punct(&out)
}

/// 收敛标点叠写：
/// - 句末标点后面再跟逗号/顿号/分号（`。”，` 删掉引号后的样子）→ 只留句末标点；
/// - 相邻的同一个标点（`。。`、`，，`）并成一个。
fn collapse_punct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev: Option<char> = None;
    for c in s.chars() {
        if let Some(p) = prev {
            let sentence_end = EMOTIVE_TAIL.contains(&p) || p == '。' || p == '.';
            if sentence_end && matches!(c, '，' | ',' | '、' | '；' | ';' | '：' | ':') {
                continue;
            }
            if p == c && matches!(c, '，' | '。' | '！' | '？' | '、' | '；' | '：') {
                continue;
            }
        }
        out.push(c);
        prev = Some(c);
    }
    out
}

/// 去掉行尾的中性标点（`EMOTIVE_TAIL` 一个都不碰）。
fn strip_neutral_tail(s: &str) -> String {
    let mut t = s.trim_end();
    while let Some(c) = t.chars().last() {
        if NEUTRAL_TAIL.contains(&c) {
            t = &t[..t.len() - c.len_utf8()];
        } else {
            break;
        }
    }
    t.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 行尾中性标点去掉，情绪符号留下。用例取自真实转写文本。
    #[test]
    fn strips_neutral_tail_keeps_emotive() {
        assert_eq!(normalize("不太像，不太有那种V的感觉。"), "不太像，不太有那种V的感觉");
        assert_eq!(normalize("你们不用上班的吗？"), "你们不用上班的吗？");
        assert_eq!(normalize("盛况啊！"), "盛况啊！");
        assert_eq!(normalize("鄱阳湖太坏了，为什么？"), "鄱阳湖太坏了，为什么？");
        // 连续的中性标点一起去掉
        assert_eq!(normalize("那你就别去了，"), "那你就别去了");
        // 情绪符号 + 中性标点：只去掉中性的那个
        assert_eq!(normalize("真的吗？！"), "真的吗？！");
        assert_eq!(normalize("想当算女人啊！开什么玩笑？"), "想当算女人啊！开什么玩笑？");
        // 语气/拖长音保留（不是中性标点）
        assert_eq!(normalize("我就是这么想的…"), "我就是这么想的…");
        assert_eq!(normalize("好久不见～"), "好久不见～");
    }

    /// 转述句式：去引号 + 冒号变逗号 + 收敛叠写。
    #[test]
    fn rewrites_quoted_speech() {
        assert_eq!(
            normalize("他他说：“他说没有。”，他说：“他是小朋友。”"),
            "他他说，他说没有。他说，他是小朋友"
        );
        assert_eq!(normalize("他说：“我今天不去了。”"), "他说，我今天不去了");
        assert_eq!(normalize("妈问：“吃了吗？”"), "妈问，吃了吗？");
        // 英文引号同样处理
        assert_eq!(normalize("他说:\"你好\"。"), "他说，你好");
        // 引号里没有情绪符号时不改变判定
        assert_eq!(normalize("“你懂什么？”"), "你懂什么？");
    }

    /// 空/纯符号的输入不能被变成空字幕，内容也不能丢。
    #[test]
    fn never_produces_empty_text() {
        assert_eq!(normalize("“”"), "“”");
        assert_eq!(normalize("。"), "。");
        assert_eq!(normalize(""), "");
        assert_eq!(normalize("   "), "");
    }

    /// 只动行尾与引号：正文里的标点一个都不许碰（切分是在原文标点上做的）。
    #[test]
    fn leaves_inner_punctuation_alone() {
        assert_eq!(
            normalize("那个直播间帅不人丑？你们平时到底哪那么多时间看直播的"),
            "那个直播间帅不人丑？你们平时到底哪那么多时间看直播的"
        );
        assert_eq!(normalize("一、二、三"), "一、二、三");
        assert_eq!(normalize("3.14"), "3.14");
        assert_eq!(normalize("我是真靠自己画画赚钱的，不是这种"), "我是真靠自己画画赚钱的，不是这种");
    }

    /// 英文缩写里的单引号不动。
    #[test]
    fn keeps_apostrophes() {
        assert_eq!(normalize("I don't know."), "I don't know");
    }

    /// 生成标记之后的内容必须截掉。用例取自真实转写（`media/3d`、`media/[好困]`）。
    #[test]
    fn strips_generation_markers() {
        assert_eq!(
            strip_generation_markers(
                "我都孤。<|endoftext|>Human Rights Watch（人权观察）是美国一家非政府组织，致力于保护人权。"
            ),
            "我都孤。"
        );
        assert_eq!(
            strip_generation_markers("这个老贵的地方，没有去，其实我太。<|endoftext|>Human Rights Watch（人权观察）")
                ,
            "这个老贵的地方，没有去，其实我太。"
        );
        // 没有标记就只是去掉首尾空白
        assert_eq!(strip_generation_markers("  你好  "), "你好");
        // 标记在开头 → 整条为空（调用方据此跳过，不能产出空字幕）
        assert_eq!(strip_generation_markers("<|endoftext|>胡说"), "");
        // 其它特殊 token 同样截断
        assert_eq!(strip_generation_markers("你好<|im_end|>世界"), "你好");
    }

    /// 在**真实转写**上看规范化效果（不需要重跑 ASR）。
    ///
    /// `VT_TRANSCRIPT=<transcript.json> cargo test -p yekitrim-core --lib diagnose_text_normalize -- --ignored --nocapture`
    #[test]
    #[ignore = "需要真实 transcript.json（VT_TRANSCRIPT）"]
    fn diagnose_text_normalize() {
        let path = std::env::var("VT_TRANSCRIPT").expect("需要 VT_TRANSCRIPT=<transcript.json 绝对路径>");
        let raw = std::fs::read_to_string(&path).expect("读取 transcript.json 失败");
        let v: serde_json::Value = serde_json::from_str(&raw).expect("解析 transcript.json 失败");
        let segs = v["segments"].as_array().expect("没有 segments");

        let (mut tail_stripped, mut quotes_removed) = (0usize, 0usize);
        let mut shown = 0usize;
        for s in segs {
            let text = s["text"].as_str().unwrap_or("");
            let out = normalize(text);
            if out == text {
                continue;
            }
            if text.trim_end_matches(|c: char| NEUTRAL_TAIL.contains(&c)).trim_end() != out.trim_end() {
                quotes_removed += 1;
            } else {
                tail_stripped += 1;
            }
            if shown < 15 {
                println!("  {text:?}\n→ {out:?}");
                shown += 1;
            }
        }
        println!(
            "\n共 {} 条：行尾去标点 {} 条 / 去引号 {} 条（含去引号后再去行尾）",
            segs.len(),
            tail_stripped,
            quotes_removed
        );
    }
}
