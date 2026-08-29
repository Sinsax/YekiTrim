//! 本地贴纸索引: 扫描 `sticker_dir()` 根目录, 归一化分组, 评分匹配。
//!
//! 数据来源: 程序目录 `stickers/` 下的贴纸文件 (EmoteLab 导出经独立脚本重命名) +
//! 可选清单 `stickers.json` (为每组提供 tags/aliases, 用于中英同义匹配)。
//!
//! 命名解析 (三级结构):
//! - **组** = 括号前主名相同: `呆.gif` / `呆_1.gif` / `呆(贴纸)_1.gif` → 组 `呆`
//! - **变体** = 括号内容 (无括号为基础变体 ""): `呆` 组含基础变体 + 「贴纸」变体
//! - **帧** = 变体内末尾 `_数字`: `呆(贴纸)_1/2/3.gif` → 「贴纸」变体的 3 帧
//! 例: `打字(恼怒).gif` → 组 `打字`, 变体「恼怒」; `爱心_1.gif` → 组 `爱心`, 基础变体帧

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 一个贴纸变体 (如「呆」组的「贴纸」变体, 含多帧)
#[derive(Debug, Clone)]
pub struct StickerVariant {
    /// 变体词: 基础变体为 "" (无括号), 否则为括号内容 (如 "贴纸" / "恼怒")
    pub name: String,
    /// 该变体的帧文件 (相对 sticker_dir, 保序)
    pub files: Vec<PathBuf>,
}

/// 一个贴纸组: 括号前主名相同的所有变体
#[derive(Debug, Clone)]
pub struct StickerGroup {
    /// 主名 (去括号, 如 "呆" / "爱心" / "打字")
    pub name: String,
    /// 变体列表 (含基础变体 "")
    pub variants: Vec<StickerVariant>,
    /// 清单 tags (可为空)
    pub tags: Vec<String>,
    /// 清单 aliases (可为空)
    pub aliases: Vec<String>,
}

impl StickerGroup {
    /// 组内全部文件 (所有变体平铺)
    pub fn all_files(&self) -> Vec<&PathBuf> {
        self.variants.iter().flat_map(|v| v.files.iter()).collect()
    }

    /// 变体词列表 (含 "" 基础变体)
    pub fn variant_names(&self) -> Vec<&str> {
        self.variants.iter().map(|v| v.name.as_str()).collect()
    }
}

/// keyword → 组的匹配结果
#[derive(Debug, Clone)]
pub struct MatchResult {
    pub group_index: usize,
    pub score: u32,
    /// 命中的字段 (用于日志/诊断)
    pub via: String,
    /// 命中的变体词 (如 keyword=生气 命中 打字(生气) → Some("生气")), 用于选帧
    pub preferred_variant: Option<String>,
}

/// 贴纸索引: 扫描一次, 供匹配/诊断/复制共用
pub struct StickerIndex {
    root: PathBuf,
    groups: Vec<StickerGroup>,
}

impl StickerIndex {
    /// 扫描贴纸目录根目录 (非递归)。目录不存在时返回空索引。
    pub fn scan(root: &Path) -> Self {
        let metas = load_sticker_meta(root);
        let mut groups: Vec<StickerGroup> = Vec::new();
        let mut by_name: HashMap<String, usize> = HashMap::new();

        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue; // 跳过子目录 (backup/ 等)
                }
                let fname = match path.file_name().and_then(|s| s.to_str()) {
                    Some(n) => n,
                    None => continue,
                };
                if fname == "stickers.json" {
                    continue;
                }
                let Some((main, variant)) = parse_filename(fname) else { continue };
                let gi = match by_name.get(&main) {
                    Some(i) => *i,
                    None => {
                        let g = StickerGroup {
                            name: main.clone(),
                            variants: vec![],
                            tags: vec![],
                            aliases: vec![],
                        };
                        groups.push(g);
                        by_name.insert(main.clone(), groups.len() - 1);
                        groups.len() - 1
                    }
                };
                let g = &mut groups[gi];
                match g.variants.iter_mut().find(|v| v.name == variant) {
                    Some(v) => v.files.push(path),
                    None => g.variants.push(StickerVariant { name: variant, files: vec![path] }),
                }
            }
        }

        // 变体排序: 基础变体 "" 在前 (字典序最小)
        for g in &mut groups {
            g.variants.sort_by(|a, b| a.name.cmp(&b.name));
        }

        // 附着清单 tags/aliases (按 name 匹配)
        let meta_by_name: HashMap<&str, &StickerMeta> =
            metas.iter().map(|m| (m.name.as_str(), m)).collect();
        for g in &mut groups {
            if let Some(m) = meta_by_name.get(g.name.as_str()) {
                g.tags = m.tags.clone();
                g.aliases = m.aliases.clone();
            }
        }
        for m in &metas {
            if !meta_by_name.contains_key(m.name.as_str()) {
                // 清单里的组在磁盘上无文件: 登记为空组, 匹配后由调用方跳过
                groups.push(StickerGroup {
                    name: m.name.clone(),
                    variants: vec![],
                    tags: m.tags.clone(),
                    aliases: m.aliases.clone(),
                });
            }
        }

        Self { root: root.to_path_buf(), groups }
    }

    pub fn groups(&self) -> &[StickerGroup] {
        &self.groups
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// keyword → 得分最高的组。同分按「同义词词典顺序 → 名称字典序」确定性选择。
    pub fn match_keyword(&self, keyword: &str) -> Option<MatchResult> {
        let kw = normalize_keyword(keyword);
        if kw.is_empty() {
            return None;
        }
        let syns = synonyms(&kw);
        let mut best: Option<(u32, usize, Option<usize>, Option<String>, String)> = None;
        for (i, g) in self.groups.iter().enumerate() {
            if g.all_files().is_empty() && g.tags.is_empty() && g.aliases.is_empty() {
                continue; // 无文件且无清单信息的组不参与
            }
            if let Some((score, syn_idx, via, prefer)) = score_group(&kw, g, &syns) {
                let better = match &best {
                    None => true,
                    Some((bs, bi, bsi, _, _)) => {
                        score > *bs
                            || (score == *bs && syn_idx < *bsi)
                            || (score == *bs && syn_idx == *bsi
                                && g.name < self.groups[*bi].name)
                    }
                };
                if better {
                    best = Some((score, i, syn_idx, prefer, via));
                }
            }
        }
        best.map(|(score, i, _, prefer, via)| MatchResult {
            group_index: i,
            score,
            via,
            preferred_variant: prefer,
        })
    }

    /// 按分数降序返回前 n 个候选 (诊断/测试用)。同分按词典顺序 → 名称字典序。
    pub fn candidates(&self, keyword: &str, n: usize) -> Vec<(u32, usize, String)> {
        let kw = normalize_keyword(keyword);
        let syns = synonyms(&kw);
        let mut scored: Vec<(u32, usize, Option<usize>, String)> = self
            .groups
            .iter()
            .enumerate()
            .filter_map(|(i, g)| {
                if g.all_files().is_empty() && g.tags.is_empty() && g.aliases.is_empty() {
                    return None;
                }
                score_group(&kw, g, &syns).map(|(s, si, via, _)| (s, i, si, via))
            })
            .collect();
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.2.cmp(&b.2))
                .then_with(|| self.groups[a.1].name.cmp(&self.groups[b.1].name))
        });
        scored.truncate(n);
        scored.into_iter().map(|(s, i, _, via)| (s, i, via)).collect()
    }

    /// 匹配命中的组是否可落地 (有实际文件)
    pub fn has_files(&self, group_index: usize) -> bool {
        !self.groups[group_index].all_files().is_empty()
    }

    /// 确定性变体/帧选择:
    /// 1. `prefer` 指定变体 (keyword 命中括号变体词) → 优先在该变体的帧中选
    /// 2. 否则基础变体 (无括号) 存在时优先 (keyword 只命中主名, 选基础形态更贴合)
    /// 3. 否则组内全部文件平铺选
    /// 同一 (group, seed) 结果恒定 (渲染可复现), 不同 seed 尽量选不同帧。
    pub fn pick_variant(&self, group_index: usize, seed: f64, prefer: Option<&str>) -> Option<String> {
        let g = &self.groups[group_index];
        if let Some(p) = prefer {
            if let Some(v) = g.variants.iter().find(|v| v.name == p) {
                if let Some(f) = pick_from(&v.files, seed) {
                    return Some(f);
                }
            }
        }
        if let Some(base) = g.variants.iter().find(|v| v.name.is_empty()) {
            if let Some(f) = pick_from(&base.files, seed) {
                return Some(f);
            }
        }
        pick_from(&g.all_files().into_iter().cloned().collect::<Vec<_>>(), seed)
    }
}

/// 在文件列表中按 seed 确定性选一个, 返回文件 stem
fn pick_from(files: &[PathBuf], seed: f64) -> Option<String> {
    if files.is_empty() {
        return None;
    }
    let idx = if files.len() == 1 {
        0
    } else {
        let frac = (seed.fract() + 1.0).abs();
        ((frac * 1000003.0) as usize) % files.len()
    };
    files[idx].file_stem().and_then(|s| s.to_str()).map(|s| s.to_string())
}

// ============================================================
// 命名解析
// ============================================================

/// 文件名 → (主名, 变体词)。
/// 变换顺序: 去扩展名 → 空格转 `_` → 剥离末尾 `_数字` 帧号 → 拆括号。
/// `呆(贴纸)_1.gif` → ("呆", "贴纸"); `爱心_1.gif` → ("爱心", ""); `被击中(爱心).gif` → ("被击中", "爱心")
fn parse_filename(fname: &str) -> Option<(String, String)> {
    let stem = Path::new(fname).file_stem()?.to_str()?.trim().to_string();
    if stem.is_empty() {
        return None;
    }
    let underscored = stem.replace(' ', "_");
    let no_frame = strip_frame_suffix(&underscored);
    match no_frame.find('(') {
        Some(pos) => {
            let head = no_frame[..pos].trim_end_matches('_').to_string();
            if head.is_empty() {
                return None;
            }
            let variant = match no_frame[pos + 1..].find(')') {
                Some(end) => no_frame[pos + 1..pos + 1 + end].trim().to_string(),
                None => String::new(),
            };
            Some((head, variant))
        }
        None => Some((no_frame, String::new())),
    }
}

/// 剥离末尾 `_数字` 帧号后缀: `爱心_1` → `爱心`; `呆(贴纸)_1` → `呆(贴纸)`
fn strip_frame_suffix(s: &str) -> String {
    if let Some(pos) = s.rfind('_') {
        let (head, tail) = s.split_at(pos);
        let tail = &tail[1..];
        if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) {
            return head.to_string();
        }
    }
    s.to_string()
}

/// query 归一化: 小写 + 去空格
fn normalize_keyword(kw: &str) -> String {
    kw.trim().to_lowercase().replace(' ', "_")
}

// ============================================================
// 评分匹配
// ============================================================

/// 对单个组计算匹配分。返回 (score, 同义词词典索引, via, 命中的变体词)。
/// 所有比较大小写不敏感。
fn score_group(
    kw: &str,
    g: &StickerGroup,
    syns: &[(usize, &str)],
) -> Option<(u32, Option<usize>, String, Option<String>)> {
    let name_l = g.name.to_lowercase();
    let var_names: Vec<String> = g.variants.iter().map(|v| v.name.to_lowercase()).collect();

    // 1. 直接命中: query == name (100) > variant (95) > tag/alias (100)
    if name_l == kw {
        return Some((100, None, "name".into(), None));
    }
    if let Some(v) = var_names.iter().find(|v| !v.is_empty() && **v == kw) {
        return Some((95, None, "variant".into(), Some(v.clone())));
    }
    if g.tags.iter().any(|t| t.to_lowercase() == kw) || g.aliases.iter().any(|a| a.to_lowercase() == kw) {
        return Some((100, None, "tag/alias".into(), None));
    }

    // 1.5. tag/alias 词干+子串模糊: chat ↔ tag "chatting"/"chat", 短边 ≥2 字符
    let kw_stem = stem(kw);
    for t in g.tags.iter().chain(g.aliases.iter()) {
        let tl = t.to_lowercase();
        let t_stem = stem(&tl);
        if kw_stem == t_stem && !kw_stem.is_empty() {
            return Some((70, None, format!("tag-stem:{t}"), None));
        }
        let (short, long) = if tl.len() <= kw.len() {
            (tl.as_str(), kw)
        } else {
            (kw, tl.as_str())
        };
        if short.chars().count() >= 2 && long.contains(short) {
            return Some((70, None, format!("tag-contains:{t}"), None));
        }
    }

    // 2. 同义词命中: 词典词 (按词典顺序优先) + 清单 tag/alias (低优先)
    //    name==syn 90 > variant==syn 85 > 包含 70
    let mut syn_items: Vec<(usize, &str)> = syns.to_vec();
    for t in g.tags.iter().chain(g.aliases.iter()) {
        syn_items.push((usize::MAX, t.as_str()));
    }
    let mut best_syn: Option<(usize, u32, String, Option<String>)> = None;
    for (idx, syn) in syn_items {
        let s = syn.to_lowercase();
        let hit: Option<(u32, Option<String>)> = if name_l == s {
            Some((90, None))
        } else if let Some(v) = var_names.iter().find(|v| !v.is_empty() && **v == s) {
            Some((85, Some(v.clone())))
        } else if name_l.contains(&s) || s.contains(&name_l) {
            Some((70, None))
        } else {
            None
        };
        let Some((score, prefer)) = hit else { continue };
        let better = match &best_syn {
            None => true,
            Some((bi, bs, _, _)) => score > *bs || (score == *bs && idx < *bi),
        };
        if better {
            best_syn = Some((idx, score, format!("synonym:{syn}"), prefer));
        }
    }
    if let Some((si, sc, via, prefer)) = best_syn {
        return Some((sc, Some(si), via, prefer));
    }

    // 3. 包含命中
    if name_l.contains(kw) || kw.contains(&name_l) {
        return Some((60, None, "contains".into(), None));
    }

    // 4. 字符重叠 (中英混合时几乎用不上, 兜底)
    let overlap = common_chars(&g.name, kw);
    if !g.name.is_empty() && overlap as f64 / g.name.chars().count().max(1) as f64 > 0.5 {
        return Some((30, None, "overlap".into(), None));
    }

    None
}

/// 两个字符串的公共字符数 (去重计数)
fn common_chars(a: &str, b: &str) -> usize {
    let sa: std::collections::HashSet<char> = a.chars().collect();
    let sb: std::collections::HashSet<char> = b.chars().collect();
    sa.intersection(&sb).count()
}

// ============================================================
// 中英同义词词典 (英文 LLM keyword → 中文贴纸名词候选)
// ============================================================

/// 英文词干剥离: 去掉 -ing/-s/-ed/-es 后缀 + 双辅音折叠 (chatting→chat, running→run)
/// 仅 ASCII, 用于 chat/chatting 类变体匹配
fn stem(word: &str) -> &str {
    if !word.is_ascii() {
        return word;
    }
    let mut result = word;
    for suffix in ["ing", "es", "ed", "s"] {
        if let Some(stripped) = result.strip_suffix(suffix) {
            if stripped.len() >= 3 {
                result = stripped;
                // 双辅音折叠: chatting→chatt→chat, running→runn→run
                let chars: Vec<char> = result.chars().collect();
                if chars.len() >= 3
                    && chars[chars.len() - 1] == chars[chars.len() - 2]
                    && chars[chars.len() - 1].is_ascii_alphabetic()
                    && !"aeiou".contains(chars[chars.len() - 1])
                {
                    result = &result[..result.len() - 1];
                }
                break;
            }
        }
    }
    result
}

/// 概念兜底词典: 常见英文概念词 → 中文贴纸候选
/// (词典未覆盖的关键词, 不依赖 stickers.json tags; 优先级低于 SYNONYMS)
const CONCEPT_MAP: &[(&str, &[&str])] = &[
    ("chat", &["说话"]),
    ("talk", &["说话"]),
    ("speak", &["说话"]),
    ("say", &["说话"]),
    ("discuss", &["说话"]),
    ("converse", &["说话"]),
    ("sigh", &["呆"]),
    ("exhale", &["呆"]),
    ("bath", &["汗", "喝"]),
    ("wash", &["汗"]),
    ("phone", &["通知", "扩音器"]),
    ("notification", &["通知"]),
    ("message", &["通知"]),
    ("bored", &["呆", "睡觉"]),
    ("silent", &["静音"]),
    ("quiet", &["静音"]),
    ("song", &["唱歌", "吉他"]),
    ("sing", &["唱歌", "吉他"]),
    ("dance", &["跳舞", "舞蹈"]),
    ("walk", &["走路", "脚步"]),
    ("run", &["跑"]),
    ("angry", &["生气", "恼怒"]),
    ("mad", &["生气"]),
    ("sad", &["哭", "难过"]),
    ("cry", &["哭"]),
    ("laugh", &["笑"]),
    ("smile", &["笑"]),
    ("shock", &["惊吓", "呆"]),
    ("scared", &["害怕", "惊吓"]),
    ("fear", &["害怕"]),
    ("happy", &["庆祝", "加油", "干杯"]),
    ("excited", &["期待", "跳舞"]),
    ("love", &["爱心", "玫瑰"]),
    ("gift", &["礼物", "红包"]),
    ("money", &["钱", "刷卡"]),
    ("food", &["吃", "馋", "蛋糕"]),
    ("eat", &["吃", "馋"]),
    ("drink", &["喝", "干杯"]),
    ("sleep", &["睡觉", "催眠"]),
    ("tired", &["睡觉", "困"]),
    ("work", &["工作", "打字"]),
    ("type", &["打字"]),
    ("computer", &["打字", "工作"]),
    ("game", &["打游戏"]),
    ("play", &["打游戏", "跳舞"]),
    ("music", &["唱歌", "吉他"]),
    ("guitar", &["吉他"]),
    ("magic", &["魔法"]),
    ("spell", &["魔法"]),
    ("fire", &["烧"]),
    ("hot", &["烧", "汗"]),
    ("cold", &["冰", "冷"]),
    ("snow", &["冰"]),
    ("rain", &["汗"]),
    ("water", &["喝"]),
    ("swim", &["喝"]),
    ("cook", &["吃"]),
    ("kitchen", &["吃"]),
    ("door", &["到达"]),
    ("open", &["到达"]),
    ("hello", &["打招呼"]),
    ("hi", &["打招呼"]),
    ("bye", &["打招呼"]),
    ("wave", &["打招呼"]),
    ("clap", &["鼓掌"]),
    ("applause", &["鼓掌"]),
    ("point", &["指"]),
    ("button", &["按钮"]),
    ("push", &["按钮"]),
    ("ok", &["一切都好", "点赞"]),
    ("yes", &["点赞", "一切都好"]),
    ("no", &["摇头"]),
    ("think", &["思考", "主意"]),
    ("idea", &["主意"]),
    ("question", &["问号"]),
    ("confused", &["问号", "呆"]),
    ("dizzy", &["头晕"]),
    ("nervous", &["紧张", "害羞"]),
    ("shy", &["害羞"]),
    ("cute", &["可爱", "害羞"]),
    ("heart", &["爱心"]),
    ("kiss", &["舔舔"]),
    ("pet", &["摸头", "舔舔"]),
    ("head", &["摸头", "敲头"]),
    ("hit", &["被击中"]),
    ("attack", &["被击中"]),
    ("gun", &["枪"]),
    ("knife", &["刀"]),
    ("sword", &["刀"]),
    ("kill", &["死亡"]),
    ("die", &["死亡"]),
    ("ghost", &["惊吓", "害怕"]),
    ("monster", &["害怕"]),
    ("pray", &["自我安慰"]),
    ("comfort", &["自我安慰", "摸头"]),
    ("record", &["记录", "抓拍"]),
    ("photo", &["抓拍"]),
    ("camera", &["抓拍"]),
    ("bell", &["摇铃"]),
    ("shake", &["摇铃"]),
    ("cola", &["摇可乐"]),
    ("celebration", &["庆祝", "干杯"]),
    ("party", &["庆祝", "荧光棒"]),
    ("cheer", &["加油", "荧光棒"]),
    ("raise", &["荧光棒", "举手"]),
    ("trash", &["垃圾桶"]),
    ("fly", &["拍蝇"]),
    ("spray", &["喷剂"]),
    ("tape", &["胶带"]),
    ("board", &["画板"]),
    ("draw", &["画板"]),
    ("egg", &["复活节", "蛋糕"]),
    ("prison", &["坐牢"]),
    ("jail", &["坐牢"]),
];

/// 由 keyword 查同义词候选 (已归一化小写)。返回 (全局词序索引, 词)。
/// 全局词序 = SYNONYMS 展平后的顺序, 同条目内词序即优先级 (love → 爱心 先于 情书)。
/// 词干变体 (chatting→chat) 与 CONCEPT_MAP 兜底排在 SYNONYMS 之后 (低优先)。
fn synonyms(kw: &str) -> Vec<(usize, &'static str)> {
    let mut global = 0usize;
    let mut out = Vec::new();
    for (k, v) in SYNONYMS {
        for s in *v {
            if *k == kw {
                out.push((global, *s));
            }
            global += 1;
        }
    }
    // 词干变体: 同一词典再查一次 (chatting → chat)
    let kw_stem = stem(kw);
    if kw_stem != kw && !kw_stem.is_empty() {
        for (k, v) in SYNONYMS {
            for s in *v {
                if *k == kw_stem {
                    out.push((global, *s));
                }
                global += 1;
            }
        }
    }
    // CONCEPT_MAP 兜底 (原词 + 词干)
    let mut concept_global = global;
    for (k, v) in CONCEPT_MAP {
        for s in *v {
            if *k == kw || *k == kw_stem {
                out.push((concept_global, *s));
            }
            concept_global += 1;
        }
    }
    out
}

static SYNONYMS: &[(&str, &[&str])] = &[
    ("love", &["爱心", "喜欢", "情书", "玫瑰"]),
    ("heart", &["爱心", "情书"]),
    ("laugh", &["笑", "小丑", "搞笑"]),
    ("happy", &["庆祝", "加油", "干杯", "一切都好", "期待", "开心"]),
    ("cheer", &["加油", "庆祝", "干杯", "荧光棒"]),
    ("excited", &["期待", "跳舞", "荧光棒", "开心"]),
    ("cry", &["哭", "流泪"]),
    ("sad", &["哭", "难过", "低落"]),
    ("angry", &["生气", "恼怒", "愤怒"]),
    ("mad", &["生气", "恼怒"]),
    ("sleep", &["睡觉", "催眠", "困"]),
    ("sleepy", &["睡觉", "催眠", "工作(小睡)"]),
    ("tired", &["工作(疲倦)", "睡觉", "困"]),
    ("dream", &["睡觉", "催眠", "困"]),
    ("nightmare", &["惊吓", "害怕", "死亡"]),
    ("relief", &["一切都好", "笑"]),
    ("calm", &["一切都好"]),
    ("bored", &["呆", "睡觉", "困"]),
    ("waiting", &["呆", "到达", "期待"]),
    ("wait", &["呆", "到达"]),
    ("panic", &["惊吓", "害怕", "叹号"]),
    ("wow", &["呆", "叹号", "惊吓"]),
    ("amazing", &["呆", "叹号"]),
    ("eat", &["吃", "馋", "蛋糕"]),
    ("eating", &["吃", "馋", "蛋糕"]),
    ("drink", &["喝", "干杯", "饮料"]),
    ("hungry", &["吃", "馋", "舔舔"]),
    ("game", &["打游戏", "游戏", "电风扇"]),
    ("gaming", &["打游戏", "游戏"]),
    ("play", &["打游戏", "跳舞", "摇铃"]),
    ("hit", &["被击中", "敲头", "拍蝇"]),
    ("attack", &["被击中", "枪", "刀"]),
    ("shock", &["惊吓", "害怕", "呆", "叹号"]),
    ("surprise", &["惊吓", "惊讶", "叹号"]),
    ("scared", &["害怕", "惊吓", "紧张"]),
    ("fear", &["害怕", "惊吓", "紧张"]),
    ("afraid", &["害怕", "惊吓"]),
    ("nervous", &["紧张", "害羞", "害怕"]),
    ("dance", &["跳舞", "舞蹈"]),
    ("sing", &["唱歌", "吉他"]),
    ("music", &["唱歌", "吉他", "扩音器"]),
    ("song", &["唱歌", "音乐"]),
    ("guitar", &["吉他", "唱歌"]),
    ("gift", &["礼物", "红包", "蛋糕"]),
    ("present", &["礼物", "红包"]),
    ("money", &["钱", "拿走我的钱", "刷卡", "要米"]),
    ("cash", &["钱", "刷卡"]),
    ("hello", &["打招呼", "到达"]),
    ("hi", &["打招呼", "到达"]),
    ("bye", &["打招呼", "挥手", "到达"]),
    ("welcome", &["打招呼", "干杯"]),
    ("yes", &["点赞", "点头", "一切都好"]),
    ("ok", &["点赞", "一切都好", "点头"]),
    ("good", &["一切都好", "点赞", "加油"]),
    ("no", &["摇头", "反向点赞"]),
    ("nope", &["摇头", "反向点赞"]),
    ("think", &["思考", "主意", "问号"]),
    ("idea", &["主意", "思考", "叹号"]),
    ("question", &["问号", "思考"]),
    ("confused", &["问号", "呆", "头晕"]),
    ("dizzy", &["头晕", "呆"]),
    ("work", &["工作", "打字", "带薪拉屎"]),
    ("typing", &["打字", "工作"]),
    ("dead", &["死亡", "停止工作"]),
    ("die", &["死亡", "停止工作"]),
    ("kill", &["死亡", "枪", "刀"]),
    ("weapon", &["枪", "刀", "撬棍", "喷剂"]),
    ("gun", &["枪", "刀"]),
    ("knife", &["刀", "枪"]),
    ("magic", &["魔法", "复活节"]),
    ("cool", &["墨镜反光", "墨镜循环", "像素墨镜反光", "摘掉墨镜"]),
    ("sunglasses", &["墨镜反光", "墨镜循环", "像素墨镜反光", "摘掉墨镜"]),
    ("hot", &["烧", "汗"]),
    ("sweat", &["汗", "烧"]),
    ("shy", &["害羞", "害羞"]),
    ("cute", &["害羞", "可爱", "爱心"]),
    ("kiss", &["舔舔", "情书"]),
    ("record", &["记录", "抓拍"]),
    ("camera", &["抓拍", "记录"]),
    ("photo", &["抓拍", "记录"]),
    ("trash", &["垃圾桶", "停止工作"]),
    ("bug", &["bug", "bug"]),
    ("attention", &["通知", "叹号", "扩音器"]),
    ("notice", &["通知", "提示"]),
    ("coin", &["硬币", "得分"]),
    ("score", &["得分", "记录"]),
    ("sticker", &["贴纸"]),
    ("point", &["指", "荧光棒"]),
    ("raise", &["荧光棒", "加油", "举手"]),
    ("clap", &["鼓掌", "拍手"]),
    ("wave", &["打招呼", "挥手", "荧光棒"]),
    ("hand", &["指", "摸头", "抓拍"]),
    ("head", &["摸头", "敲头", "摇头"]),
    ("sleep_prep", &["准备阶段1", "准备阶段2"]),
    ("smile", &["笑", "一切都好"]),
    ("wink", &["眨眼", "笑"]),
    ("shut", &["静音", "停止工作"]),
    ("mute", &["静音", "扩音器"]),
    ("loud", &["扩音器", "唱歌"]),
    ("bubble", &["冒泡", "思考"]),
    ("pet", &["摸头", "舔舔"]),
    ("push", &["按钮", "按"]),
    ("button", &["按钮"]),
    ("egg", &["复活节", "蛋糕"]),
    ("easter", &["复活节"]),
    ("jail", &["坐牢"]),
    ("prison", &["坐牢"]),
    ("flip", &["反转", "反转"]),
    ("reverse", &["反转", "反向点赞"]),
    ("crown", &["小丑", "庆祝"]),
    ("clown", &["小丑"]),
    ("shower", &["洗澡", "汗"]),
    ("rain", &["下雨", "汗"]),
    ("sunglasses_pop", &["像素墨镜反光", "墨镜反光"]),
    ("shake", &["摇可乐", "摇铃", "摇头"]),
    ("bell", &["摇铃", "通知"]),
    ("cola", &["摇可乐", "喝"]),
    ("drive", &["驾驶", "到达"]),
    ("car", &["驾驶", "到达"]),
    ("tape", &["胶带", "胶带"]),
    ("board", &["画板", "记录"]),
    ("draw", &["画板", "记录"]),
    ("cheese", &["奶酪糊脸", "吃"]),
    ("fly", &["拍蝇", "到达"]),
    ("spray", &["喷剂", "喷"]),
    ("mop", &["拖把", "清洁"]),
    ("self", &["自我安慰", "自我"]),
    ("comfort", &["自我安慰", "摸头"]),
    ("destiny", &["命运", "六七"]),
    ("six", &["六七"]),
    ("glow", &["荧光棒", "冒泡"]),
    ("fan", &["电风扇", "摇扇"]),
    ("swing", &["折扇", "摇扇"]),
    ("explosion", &["爆炸", "叹号"]),
    ("boom", &["爆炸", "叹号", "烧"]),
    ("fire", &["烧", "叹号", "爆炸"]),
    ("burn", &["烧", "死亡"]),
    ("melt", &["烧", "融化"]),
    ("ice", &["冰", "冷"]),
    ("cold", &["冷", "冰"]),
    ("alien", &["外星人", "小丑"]),
    ("ufo", &["外星人", "飞行"]),
    ("tea", &["喝茶", "喝"]),
    ("coffee", &["咖啡", "喝"]),
];

// ============================================================
// stickers.json 清单 (可选)
// ============================================================

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StickerMeta {
    pub name: String,
    /// 组内文件 (可选, 空则按 name 自动分组)
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
}

fn load_sticker_meta(root: &Path) -> Vec<StickerMeta> {
    let p = root.join("stickers.json");
    let raw = match std::fs::read_to_string(&p) {
        Ok(r) => r,
        Err(_) => return vec![],
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

// ============================================================
// 测试
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn group_names(idx: &StickerIndex) -> Vec<String> {
        idx.groups().iter().map(|g| g.name.clone()).collect()
    }

    /// 构造临时贴纸目录
    fn temp_sticker_dir() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "爱心.gif", "爱心_1.gif", "爱心_2.gif",
            "呆.gif", "呆_1.gif",
            "呆(贴纸).gif", "呆(贴纸)_1.gif",
            "被击中(爱心).gif",
            "被击中(拖鞋).gif",
            "打字(恼怒).gif", "打字(普通).gif", "打字(生气).gif",
            "睡觉.gif", "睡觉_1.gif",
            "问号.gif",
            "说话.gif",
            "Bug.gif",
        ] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        let p = dir.path().to_path_buf();
        (dir, p)
    }

    #[test]
    fn groups_by_name() {
        let (_d, root) = temp_sticker_dir();
        let idx = StickerIndex::scan(&root);
        let names = group_names(&idx);
        assert!(names.contains(&"爱心".to_string()));
        assert!(names.contains(&"呆".to_string()));
        assert!(names.contains(&"被击中".to_string()));
        assert!(names.contains(&"打字".to_string()));
        assert!(names.contains(&"Bug".to_string()));
        // 爱心 3 帧 → 1 组 3 文件 (基础变体)
        let love = names.iter().position(|n| n == "爱心").unwrap();
        assert_eq!(idx.groups()[love].all_files().len(), 3);
        // 呆 与 呆(贴纸) 合并: 基础 2 帧 + 贴纸变体 2 帧
        let dai = names.iter().position(|n| n == "呆").unwrap();
        assert_eq!(idx.groups()[dai].all_files().len(), 4);
        assert_eq!(idx.groups()[dai].variant_names(), vec!["", "贴纸"]);
        // 打字 3 变体 (无基础文件, 无 "" 变体)
        let typing = names.iter().position(|n| n == "打字").unwrap();
        assert_eq!(idx.groups()[typing].variant_names(), vec!["恼怒", "普通", "生气"]);
        // 被击中 2 变体 (爱心/拖鞋)
        let hit = names.iter().position(|n| n == "被击中").unwrap();
        assert_eq!(idx.groups()[hit].variant_names(), vec!["拖鞋", "爱心"]);
    }

    #[test]
    fn match_scoring() {
        let (_d, root) = temp_sticker_dir();
        let idx = StickerIndex::scan(&root);

        // 主名精确命中
        let m = idx.match_keyword("爱心").unwrap();
        assert_eq!(m.score, 100);
        assert_eq!(idx.groups()[m.group_index].name, "爱心");
        // 变体精确命中 → 95 + preferred_variant (name 命中优先于 variant 命中)
        let m = idx.match_keyword("生气").unwrap();
        assert_eq!(m.score, 95);
        assert_eq!(idx.groups()[m.group_index].name, "打字");
        assert_eq!(m.preferred_variant.as_deref(), Some("生气"));
        // name 命中优先于 variant 命中: 呆 → 呆组 (不是 六七(呆) 变体)
        let m = idx.match_keyword("呆").unwrap();
        assert_eq!(m.score, 100);
        assert_eq!(idx.groups()[m.group_index].name, "呆");
        // 同义词: love → 爱心 (90)
        let m = idx.match_keyword("love").unwrap();
        assert_eq!(m.score, 90);
        assert_eq!(idx.groups()[m.group_index].name, "爱心");
        // 同义词命中变体: angry → 生气 → 打字(生气) (85 + prefer)
        let m = idx.match_keyword("angry").unwrap();
        assert_eq!(m.score, 85);
        assert_eq!(m.preferred_variant.as_deref(), Some("生气"));
        // 英文直接命中: bug → Bug
        let m = idx.match_keyword("bug").unwrap();
        assert_eq!(m.score, 100);
        // 新同义词: dream → 睡觉
        let m = idx.match_keyword("dream").unwrap();
        assert_eq!(idx.groups()[m.group_index].name, "睡觉");
        // question → 问号 (词典顺序: 问号 先于 思考)
        let m = idx.match_keyword("question").unwrap();
        assert_eq!(idx.groups()[m.group_index].name, "问号");
        // 概念兜底: chat → 说话, sigh → 呆
        let m = idx.match_keyword("chat").unwrap();
        assert_eq!(idx.groups()[m.group_index].name, "说话");
        let m = idx.match_keyword("sigh").unwrap();
        assert_eq!(idx.groups()[m.group_index].name, "呆");
        // 词干变体: chatting → chat → 说话
        let m = idx.match_keyword("chatting").unwrap();
        assert_eq!(idx.groups()[m.group_index].name, "说话");
        // 无命中
        assert!(idx.match_keyword("zzz_nothing").is_none());
    }

    #[test]
    fn pick_variant_prefers_variant() {
        let (_d, root) = temp_sticker_dir();
        let idx = StickerIndex::scan(&root);
        let dai = idx.groups().iter().position(|g| g.name == "呆").unwrap();
        // prefer=贴纸 → 选 呆(贴纸) 帧
        let s = idx.pick_variant(dai, 1.234, Some("贴纸")).unwrap();
        assert!(s.starts_with("呆(贴纸)"), "got {}", s);
        // 无 prefer → 基础变体帧 (呆/呆_1)
        let s2 = idx.pick_variant(dai, 1.234, None).unwrap();
        assert!(s2 == "呆" || s2 == "呆_1", "got {}", s2);
        // 确定性
        assert_eq!(idx.pick_variant(dai, 1.234, Some("贴纸")).unwrap(), s);
    }

    #[test]
    fn frame_suffix_stripped() {
        assert_eq!(strip_frame_suffix("爱心_1"), "爱心");
        assert_eq!(strip_frame_suffix("呆(贴纸)_1"), "呆(贴纸)");
        assert_eq!(strip_frame_suffix("通知_提示_爱心_喜欢"), "通知_提示_爱心_喜欢");
        assert_eq!(strip_frame_suffix("爱心"), "爱心");
    }

    #[test]
    fn parse_filename_breaks_paren() {
        assert_eq!(parse_filename("呆(贴纸)_1.gif"), Some(("呆".into(), "贴纸".into())));
        assert_eq!(parse_filename("爱心_1.gif"), Some(("爱心".into(), "".into())));
        assert_eq!(parse_filename("被击中(爱心).gif"), Some(("被击中".into(), "爱心".into())));
        assert_eq!(parse_filename("打字(恼怒).gif"), Some(("打字".into(), "恼怒".into())));
        assert_eq!(parse_filename("通知_提示_爱心_喜欢.gif"), Some(("通知_提示_爱心_喜欢".into(), "".into())));
        assert_eq!(parse_filename("睡觉_(UU).gif"), Some(("睡觉".into(), "UU".into())));
    }
}
