//! §10 情绪轨道：从弹幕反推「观众情绪」。
//!
//! 规格见 `docs/plan/emotion-narrative-engine.md` §2。
//!
//! ⚠️ **这里只做 L1（词表）+ L2（颜色/模式）**：零成本、可解释、可单测。
//! L3（LLM 批量打标）是**可选增强**，故意不在这里 —— 规格明写"必须可选"。
//!
//! ⚠️ **无弹幕时退到字幕**（本模块不做那件事，调用方做），
//! 且**必须在日志里明说用了哪条判据**。

use serde::{Deserialize, Serialize};

use super::parser::DanmakuEntry;

/// 四种情绪维度（规格 §2.1 的表）。
///
/// ⚠️ 四个**不是"热度"的分解**，是**互不相同的判断**：
/// `shock` 与 `laugh` 都是"很热"，但该配的画面**完全相反**
/// （震惊要静音+闪白，爆笑要抖动+音效）。
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Emotion {
    /// 喜 / 笑。
    pub joy: f64,
    /// 惊 / 震撼。
    pub shock: f64,
    /// 笑（弹幕的"哈哈"类，与 `joy` 分开：`joy` 是"开心"，`laugh` 是"笑出来"）。
    pub laugh: f64,
    /// 叹 / 感动 / 震撼的美感。
    pub awe: f64,
}

impl Emotion {
    /// 四个维度的**最大者**与它的名字 —— 演出脚本按它选配方。
    ///
    /// ⚠️ 平局时按**固定顺序**取（`shock` > `laugh` > `awe` > `joy`）：
    /// 用浮点比较的"任意"顺序会让同一份工程两次跑出不同结果。
    pub fn dominant(&self) -> (&'static str, f64) {
        let cands = [
            ("shock", self.shock),
            ("laugh", self.laugh),
            ("awe", self.awe),
            ("joy", self.joy),
        ];
        let mut best = cands[0];
        for c in &cands[1..] {
            if c.1 > best.1 {
                best = *c;
            }
        }
        best
    }

    /// 整体强度（四维的**最大值**，不是和 —— 和会让"四样都有一点"看起来比"一样很强"更强）。
    pub fn intensity(&self) -> f64 {
        self.dominant().1
    }

    /// 是否**弱到不值得演出**。
    ///
    /// ⚠️ 规格 §2.3 的硬要求：信号弱时要**降低演出强度**，不是"硬演"。
    /// 阈值 0.15 的由来：L1 命中一个词通常给 0.6~1.0，
    /// 而"随机噪声"（一个无关弹幕）给 0 —— 0.15 是"至少有一条真的命中了情绪词"。
    pub fn is_weak(&self) -> bool {
        self.intensity() < 0.15
    }

    /// 逐维取最大（**不是相加**）—— 两个窗口合成一个时用。
    ///
    /// ⚠️ 相加会让"两个都中等"胜过"一个极强"，而演出的判据是"这里够不够强"。
    pub fn max_with(&self, other: &Self) -> Self {
        Self {
            joy: self.joy.max(other.joy),
            shock: self.shock.max(other.shock),
            laugh: self.laugh.max(other.laugh),
            awe: self.awe.max(other.awe),
        }
    }
}

/// 一个窗口上的情绪（规格 §2.1 的曲线一行）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmotionWindow {
    /// 窗口起点（秒）。
    pub start: f64,
    /// 该窗口的情绪向量。
    pub emotion: Emotion,
    /// 该窗口里参与判定的弹幕条数（**用于解释**："这个判断基于几条"）。
    pub count: u64,
    /// 命中的关键词（**用于解释** —— 用户要能问"凭什么说这里是震惊"）。
    ///
    /// ⚠️ 保留它**不是调试便利**：用户看到时间轴上"这里被判成震惊"必须有可查的答案
    /// （哪个词、几条弹幕、强度多少）。只留一个数字的话，用户只能怀疑是 bug。
    ///
    /// ⚠️⚠️ **更正（2026-10）**：这里原写「情绪轨道会驱动画面（静音/闪白/大字）」——
    /// **那是设想，不是事实**。全仓 `emotion` 的消费者只有两个：`GET /api/polish/emotion`
    /// 与时间轴那条**只读观察轨**；出片链路（转译器 / `templates/index.html` / `engine.js`）
    /// 对它**零命中**，配置里也没有任何 emotion → 画面的开关。
    /// 写着一句没兑现的承诺，会让下一个人以为"这里已经会驱动画面了"而不去接。
    pub hits: Vec<String>,
}

/// L1 词表：一个情绪维度的关键词。
///
/// ⚠️ 词表**按"命中即算"**（不要求全体匹配）—— 弹幕很短（"草""？？？"），
/// 要求多词共现会让绝大多数真实弹幕被判成无关。
fn lexicon(kind: &str) -> &'static [&'static str] {
    match kind {
        // 笑：中文 + 网络用语 + 颜文字 + 拟声。
        "laugh" => &[
            "哈哈", "笑死", "草", "233", "hhh", "嘻嘻", "乐", "绷不住", "乐死",
            "wwww", "笑不活", "哈哈哈哈", "嘿嘿",
        ],
        // 惊：问号与"卧槽"类。
        "shock" => &[
            "？？？", "???", "？？", "卧槽", "我去", "天哪", "天啊", "啊？", "啥？",
            "什么鬼", "离谱", "震惊", "吓", "妈呀", "!!", "！！",
        ],
        // 叹 / 感动。
        "awe" => &[
            "泪目", "感动", "哭了", "牛", "牛逼", "强", "厉害", "绝了", "太强",
            "神了", "帅", "美", "舒服", "值了",
        ],
        // 喜 / 认同。
        "joy" => &[
            "哈哈", "好耶", "喜欢", "爱了", "可爱", "开心", "太好了", "赞", "棒",
            "确实", "+1", "同意", "支持",
        ],
        _ => &[],
    }
}

/// 单条弹幕的情绪打分（L1 + L2）。
///
/// 返回 `(Emotion, hits)`。`hits` 是命中的词/加权理由，供调用方解释。
fn score_entry(entry: &DanmakuEntry) -> (Emotion, Vec<String>) {
    let text = entry.text.to_lowercase();
    let mut e = Emotion::default();
    let mut hits = Vec::new();

    for (kind, weight) in [("laugh", 0.9), ("shock", 0.95), ("awe", 0.8), ("joy", 0.6)] {
        for kw in lexicon(kind) {
            if text.contains(kw) {
                let cur = match kind {
                    "laugh" => &mut e.laugh,
                    "shock" => &mut e.shock,
                    "awe" => &mut e.awe,
                    _ => &mut e.joy,
                };
                *cur = cur.max(weight);
                hits.push((*kw).to_string());
                // 一个词表里命中一个就够（继续找只是多记几个 hits）。
            }
        }
    }

    // ---- L2：颜色（B站彩色弹幕要花钱 ⇒ 是**付费的强调**）----
    //
    // ⚠️ 彩色用**饱和度**判定而不是"是不是白色"：白弹幕是 `#ffffff`，
    // 而浅灰（`#f0f0f0`）也是"没花钱"的默认色系 —— 只看"!= 白"会把
    // 抗锯齿/色偏的白判成彩色，于是**每一条弹幕都"花了钱"**，权重等于没加。
    if let Some((r, g, b)) = parse_hex(&entry.color) {
        let mx = r.max(g).max(b);
        let mn = r.min(g).min(b);
        // 饱和度（max-min）/max，避开 0 除。
        let sat = if mx > 0.0 { (mx - mn) / mx } else { 0.0 };
        if sat > 0.35 && mx > 0.3 {
            // 彩色弹幕 = **更强的强调** —— 给当前最强的那一维加成。
            let (dom, _) = e.dominant();
            if e.intensity() > 0.0 {
                match dom {
                    "laugh" => e.laugh = (e.laugh + 0.15).min(1.0),
                    "shock" => e.shock = (e.shock + 0.15).min(1.0),
                    "awe" => e.awe = (e.awe + 0.15).min(1.0),
                    _ => e.joy = (e.joy + 0.15).min(1.0),
                }
                hits.push("彩".to_string());
            } else {
                // 有颜色但没命中词 ⇒ 不能凭空造情绪（那会让"随便一条彩色弹幕"变成演出）。
                // ⚠️ 这条**刻意不记 hit**：记了会让"为什么这里强"无从解释。
            }
        }
    }

    (e, hits)
}

/// `#rrggbb` -> `(r, g, b)`，各归一化到 0..1。解析不出给 `None`。
fn parse_hex(s: &str) -> Option<(f64, f64, f64)> {
    let h = s.trim().trim_start_matches('#');
    // ⚠️ 只认 6 位：3 位简写要展开，而**展开错了会静默给一个错颜色**
    // （`#f0f` 展开成 `#ff00ff` vs 不展开被当成 `#f0f000`）。宁可不判。
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some((
        ((v >> 16) & 0xff) as f64 / 255.0,
        ((v >> 8) & 0xff) as f64 / 255.0,
        (v & 0xff) as f64 / 255.0,
    ))
}

/// 把一串弹幕按窗口聚成情绪曲线（**L1 + L2**）。
///
/// * `entries`：弹幕（**空 ⇒ 返回空**，调用方据此退到字幕）；
/// * `window_secs`：窗口长度；
/// * `total_duration`：总时长（决定窗口数）。
///
/// ⚠️ 一条弹幕**按中点归窗**（与 `analyze` 的密度口径一致）——
/// 用起点会让"长弹幕"跨窗时被算两次。
pub fn track(
    entries: &[DanmakuEntry],
    window_secs: f64,
    total_duration: f64,
) -> Vec<EmotionWindow> {
    if entries.is_empty() || window_secs <= 0.0 || total_duration <= 0.0 {
        return Vec::new();
    }
    let num_windows = (total_duration / window_secs).ceil() as usize;
    let mut out: Vec<EmotionWindow> = (0..num_windows)
        .map(|i| EmotionWindow {
            start: i as f64 * window_secs,
            emotion: Emotion::default(),
            count: 0,
            hits: Vec::new(),
        })
        .collect();

    for entry in entries {
        let mid = (entry.start + entry.end) / 2.0;
        let idx = (mid / window_secs) as usize;
        let Some(w) = out.get_mut(idx) else { continue };
        let (e, hits) = score_entry(entry);
        // ⚠️ 窗口内的合成用 **max** 而不是平均：
        // 情绪是"这里发生了没有"，不是"这里的平均心情" ——
        // 100 条无关弹幕里夹 1 条"？？？"，平均会把它冲淡到看不出来，
        // 而那 1 条恰恰是"震惊"的证据。
        w.emotion = w.emotion.max_with(&e);
        w.count += 1;
        w.hits.extend(hits);
    }

    out
}

/// 字幕情绪（**无弹幕时的主判据**，规格 §2.3 的硬要求）。
///
/// ⚠️ 用的信号与弹幕那套**不同**：主播的情绪体现在
/// **语气词 / 感叹号 / 重复标点 / 句长**，而不是网络用语
/// （主播不会说"笑死"来标记自己好笑）。
pub fn score_subtitle(text: &str) -> Emotion {
    let mut e = Emotion::default();
    let t = text;

    // 感叹句：感叹号（中文全角也算）。
    let exclaims = t.matches('！').count() + t.matches('!').count();
    if exclaims > 0 {
        e.shock = e.shock.max((0.4 + 0.2 * exclaims as f64).min(0.9));
    }
    // 疑问：问号堆叠 = 惊讶/不解。
    let qs = t.matches('？').count() + t.matches('?').count();
    if qs >= 2 {
        e.shock = e.shock.max(0.7);
    } else if qs == 1 {
        e.shock = e.shock.max(0.3);
    }
    // 语气词 / 笑声拟声（主播"哈哈"）。
    for kw in ["哈哈", "嘿嘿", "呵", "笑"] {
        if t.contains(kw) {
            e.laugh = e.laugh.max(0.75);
        }
    }
    for kw in ["哇", "天哪", "不是吧", "真的假的", "我靠", "啊"] {
        if t.contains(kw) {
            e.shock = e.shock.max(0.7);
        }
    }
    for kw in ["谢谢", "感谢", "喜欢", "好看", "可爱"] {
        if t.contains(kw) {
            e.joy = e.joy.max(0.5);
        }
    }

    e
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ⚠️ **`end` 决定落在哪个窗口**（按中点归窗）：`end = start + 4.0` 会把
    /// 一条 `start=0` 的弹幕放到**窗口 2**，而下面几条用例读的是 `w[0]`。
    /// 第一版就是这么写错的 —— 测试报"应当判成 laugh，实得 shock（0）"，
    /// 看起来像词表串了，实际是**读错了窗口**（`w[0]` 本来就是空的）。
    /// 用一个零长变体，让中点正好落在起点那个窗口。
    fn entry_at(text: &str, color: &str, start: f64) -> DanmakuEntry {
        DanmakuEntry {
            start,
            end: start,
            text: text.to_string(),
            color: color.to_string(),
            y: 0.0,
            travel_duration: 4.0,
        }
    }

    fn entry(text: &str, color: &str, start: f64) -> DanmakuEntry {
        DanmakuEntry { end: start + 4.0, ..entry_at(text, color, start) }
    }


    #[test]
    fn 空白弹幕返回空轨() {
        assert!(track(&[], 1.0, 60.0).is_empty());
        // 时长或窗口非法也返回空（不给"除零/死循环"留后门）。
        assert!(track(&[entry_at("哈哈", "#fff", 0.0)], 0.0, 60.0).is_empty());
        assert!(track(&[entry_at("哈哈", "#fff", 0.0)], 1.0, 0.0).is_empty());
    }

    #[test]
    fn 关键词识别到对应维度() {
        let cases = [
            ("笑死我了", "laugh"),
            ("？？？", "shock"),
            ("泪目", "awe"),
            ("好耶", "joy"),
        ];
        for (text, want) in cases {
            // ⚠️ 用 `entry_at`（**零长**）：`entry` 的 `end = start + 4.0` 会把中点
            // 放到窗口 2，而这里读的是 `w[0]` —— 第一版就是这么写错的，
            // 报出来像"词表串了"（"笑死我了"判成 shock(0)），其实是**读错了窗口**。
            let w = track(&[entry_at(text, "#ffffff", 0.0)], 1.0, 10.0);
            assert_eq!(w.len(), 10);
            let (dom, v) = w[0].emotion.dominant();
            assert_eq!(dom, want, "「{text}」应当判成 {want}，实得 {dom}（{v}）");
            assert!(v > 0.0, "「{text}」命中了却强度为 0");
            assert!(!w[0].hits.is_empty(), "「{text}」没有留下可解释的 hits");
        }
    }

    #[test]
    fn 无关弹幕不强加情绪() {
        // ⚠️ 这条防的是"随便一条弹幕都变成演出"：
        // 没有命中词、颜色也是默认白 ⇒ 强度必须是 0。
        let w = track(&[entry_at("今天天气不错", "#ffffff", 0.0)], 1.0, 10.0);
        assert!(
            w[0].emotion.intensity() == 0.0,
            "无关弹幕被判出了情绪 {} —— 那会让整条片子都在演",
            w[0].emotion.intensity()
        );
        assert!(w[0].emotion.is_weak(), "无关弹幕应当算'信号弱'");
    }

    #[test]
    fn 彩色弹幕给已命中的维度加成() {
        let plain = track(&[entry_at("笑死", "#ffffff", 0.0)], 1.0, 10.0);
        let colored = track(&[entry_at("笑死", "#ff0000", 0.0)], 1.0, 10.0);
        assert!(
            colored[0].emotion.laugh > plain[0].emotion.laugh,
            "彩色弹幕没有加成：{} vs {}",
            colored[0].emotion.laugh,
            plain[0].emotion.laugh
        );
        // ⚠️ 另一头：**彩色但没命中词**不许凭空造情绪。
        let colored_only = track(&[entry_at("今天天气不错", "#ff0000", 0.0)], 1.0, 10.0);
        assert_eq!(
            colored_only[0].emotion.intensity(),
            0.0,
            "彩色但无情绪词的弹幕被造出了情绪 —— 那会让'随便一条彩色弹幕'触发演出"
        );
    }

    /// ⚠️ **浅灰/近白不算彩色** —— 只看"!= 白"会把抗锯齿的白判成彩色，
    /// 于是每一条弹幕都"花了钱"，L2 权重等于没加。
    #[test]
    fn 近白弹幕不算彩色() {
        for c in ["#f0f0f0", "#fefefe", "#eeeeee"] {
            let a = track(&[entry_at("笑死", c, 0.0)], 1.0, 10.0);
            let b = track(&[entry_at("笑死", "#ffffff", 0.0)], 1.0, 10.0);
            assert_eq!(
                a[0].emotion.laugh, b[0].emotion.laugh,
                "{c} 被判成彩色了（强度 {} vs 纯白 {}）—— 几乎每条白弹幕都会被加成",
                a[0].emotion.laugh, b[0].emotion.laugh
            );
        }
    }

    #[test]
    fn 窗口内取最大而不是平均() {
        // 99 条无关 + 1 条"？？？" ⇒ 必须是**震惊**（平均会冲到 ~0.01）。
        let mut es: Vec<DanmakuEntry> = (0..99)
            .map(|i| entry_at("今天天气不错", "#ffffff", i as f64 * 0.01))
            .collect();
        es.push(entry_at("？？？", "#ffffff", 0.5));
        let w = track(&es, 1.0, 10.0);
        assert!(
            w[0].emotion.shock > 0.5,
            "99 条无关弹幕把 1 条'？？？'冲淡了（shock={}）—— 窗口合成了平均而不是最大",
            w[0].emotion.shock
        );
    }

    #[test]
    fn 按中点归窗() {
        // start=0, end=4 ⇒ 中点 2 ⇒ 落在第 2 个窗口（窗口 1s）。
        // ⚠️ 这条**故意**用 4 秒窗口的 `entry`（不是零长的 `entry_at`）——
        // 它验的就是"按中点归窗"这件事本身。
        let w = track(&[entry("笑死", "#ffffff", 0.0)], 1.0, 10.0);
        assert_eq!(w[0].emotion.laugh, 0.0, "起点那个窗口不该有它");
        assert!(w[2].emotion.laugh > 0.0, "中点所在的窗口 2 应当有它");
    }

    #[test]
    fn 主导维度平局时顺序固定() {
        // 同一个词表可能同时命中两维（"哈哈"既在 laugh 也在 joy）。
        let e = Emotion { joy: 0.6, shock: 0.6, laugh: 0.6, awe: 0.6 };
        // 固定顺序 shock > laugh > awe > joy ⇒ 四次都必须是 shock。
        for _ in 0..4 {
            assert_eq!(e.dominant().0, "shock", "平局时顺序不固定 ⇒ 同一工程两次跑出不同结果");
        }
    }

    #[test]
    fn 字幕情绪用于无弹幕情形() {
        // 感叹句 → 震惊。
        assert!(score_subtitle("这也太厉害了吧！！！").shock > 0.5);
        // 叠问号 → 震惊。
        assert!(score_subtitle("啊？？").shock > 0.5);
        // 主播自己笑 → laugh。
        assert!(score_subtitle("哈哈哈哈").laugh > 0.5);
        // 无标记的平铺直叙 → 弱（**这是硬要求：不许硬演**）。
        assert!(score_subtitle("今天我来说一下这个东西").is_weak());
    }

    #[test]
    fn 十六进制颜色解析() {
        assert!(parse_hex("#ffffff").is_some());
        assert!(parse_hex("ff0000").is_some());
        // 3 位简写**故意不认**（展开错了会静默给一个错颜色）。
        assert!(parse_hex("#fff").is_none());
        assert!(parse_hex("#zzzzzz").is_none());
        assert!(parse_hex("").is_none());
    }
}
