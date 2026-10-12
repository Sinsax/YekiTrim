//! §5 E4a —— 文字元素动效。
//!
//! 规格见 `docs/plan/elements-spec.md` §4 的 T1–T6 表。本模块负责
//! **T2 打字机**（typewriter：字逐个出现）+ **T1 弹入 / T4 抖动 / T6 底衬
//! 的配置形状**（2026-10-11 接上；真正的逐帧求值在**底座**，
//! 见 Dhampir 仓的 `crates/dhampir-timeline/src/text_motion.rs`）。
//!
//! # 底座字幕轨能力：2026-10-11 起**四项已就绪**
//!
//! 下表是 2026-10-10 那次勘察的结论（当时后四项都缺），**已过期**，保留是为了
//! 说清"为什么当初只做了 T2"：
//!
//! | 动效 | 需要什么 | 底座现状（2026-10-11） |
//! |---|---|---|
//! | T2 打字机 | 逐帧改**显示几个字** | ✅ 本仓把一条 cue 按帧拆开即可（见下） |
//! | T1 弹入 | `font_ratio` **逐帧缩放** | ✅ `scale_in_from` / `scale_in_peak` / `scale_in_ms` |
//! | T4 抖动 | **持续**逐帧偏移 | ✅ `shake_px` / `shake_hz` / `shake_phase`（纯 `t` 的函数） |
//! | T6 底衬 | 文字背后的色块 | ✅ `plate_color` / `plate_pad_px` |
//! | T3 强调放大 | 逐条放大 | ⚠️ **底座通道已就绪**（`Cue.scale`）—— 但**本仓不接**，见下 |
//!
//! # ⚠️⚠️ T3 强调放大：**底座通道已就绪，本仓无触发源 —— 故意留空**
//!
//! **不是没做，是没得接。** 写清楚免得下一个人再花一遍时间查（我自己查了一轮）：
//!
//! * **底座的通道是好的**：`Cue.scale: Option<f32>`（`None` = 1.0），
//!   乘法合成 `line.scale × enter_scale × cue.scale`，判据齐、变异自证过
//!   （底座 `ac97b70`）。
//! * **但它是"逐条"的，而本仓这一档没有"哪一条要放大"这个信息。**
//!   底座那个字段的文档写着"哪几条命中是**宿主**知道的（按 `<span class="hl">` 标记判）"。
//!   实测本仓这一条路**从来不产这个标记**：
//!   ```
//!   tools/polish-to-dhampir.mjs:3371  const textBody = String(event.text ?? '');  ← 原样透传
//!   tools/polish-to-dhampir.mjs:3385  text: textBody,                             ← 直进 cue
//!   process/mod.rs 里 'class="hl"'                                     → 0 命中
//!   .hl 的生产者只有 cover.rs（封面）/ takumi.rs（封面测试）           → 都不在这条路
//!   ```
//!   ⇒ 文字元素这条路**从头到尾不存在"哪几个字是关键词"**。
//!
//! * ⚠️ **不许拿 `filter.highlight_keywords` 顶替。** 那是**封面**的字段
//!   （`cover.rs` 消费），借来的后果是"**用户改了封面高亮设置，成片里的文字动效跟着变**"——
//!   一个字段两处语义，而且**症状是静默的**（没人会往"我刚改的是封面"那想）。
//!   这属于本仓最贵的那类错，"跑得通、不报错、结果不对"。
//!
//! * **真要做 = 新功能，不是接线**：要同时定"关键词从哪来"+ 改 `process`
//!   （把标记写进 `event.text` 或新字段）+ 提示词 + 前端，**单开一批**。
//!
//! ⇒ 所以本结构体里**没有** `emphasize_*` 字段，转译器也**不写** `cue.scale`。
//!   底盘那份 `Cue.scale` 通道空转着，等产品决策。
//!
//! # ⚠️ 为什么 T2 拆 cue、而不是 ASS 的 `\k` 覆盖标签
//!
//! 底座解析字幕时会把 `{...}` 覆盖标签**整块剥掉**（`subtitle.rs::ass_text_to_plain`，
//! 只留颜色与 `\move`）。⇒ `\k`（libass 原生卡拉OK）**根本到不了 libass**，
//! 那条路是死的。拆 cue 是唯一可行的：**不改底座一行**，且逐帧精确
//! （成片与预览读的是同一份 cue 表，不会分叉）。
//!
//! # ⚠️ 硬约束：逐帧计算，不许 CSS transition / 状态机
//!
//! 规格 §4.1 明写。否则 `seek` 到第 100 帧与"播到第 100 帧"长得不一样
//! ⇒ 预览与导出分叉。本模块的 [`split_typewriter`] 是**纯函数**
//! （输入 cue + 帧率 ⇒ 输出 cue 列表），无累积状态 —— 与 `handheld_sway` 同源。

use serde::{Deserialize, Serialize};

/// 文字元素动效的配置（住在 `polish.toml` 的 `[elements]`）。
///
/// ⚠️ **默认全关** ⇒ 老工程零迁移（R4：存量逐字节不变）。
/// 不写这个段 ⇔ 行为与今天逐字节相同 —— 与 §11 BGM 同一套做法。
///
/// ⚠️⚠️ **`deny_unknown_fields` 是必须的**（2026-10-11 实测发现）：
/// 少了它，`chars_per_sec` 拼成 `char_per_sec` 会被 **serde 静默丢掉** ——
/// 不报错、不打日志、`typewriter` 照旧 `true`，**唯独速度不是用户写的那个**。
/// 实测：写 `char_per_sec_typo = 99` 跑完 `process`，日志一切正常、
/// 段里只剩 `chars_per_sec = 14.0`（默认值）。
/// ★ 这正是本仓反复踩的那一类："**跑得通、日志正常、结果不对**"。
/// 顶层 `ProjectFile` 本来就是 `deny_unknown_fields`，同段加它**不新增口径**。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElementsConfig {
    /// 打字机总开关。**默认关**。
    #[serde(default)]
    pub typewriter: bool,

    /// 每秒打几个字。默认 **14**。
    ///
    /// ⚠️ 这个数不是随便定的：太慢（<8）会让 12 个字的标题打 1.5 秒，
    /// 观众已经看到字幕了、标题还在慢慢冒；太快（>25）与"直接出现"没区别，
    /// 白拆一堆 cue。14 字/秒 ≈ 一个字 71ms，与参照的"逐字出现"节奏接近。
    #[serde(default = "default_chars_per_sec")]
    pub chars_per_sec: f64,

    /// 打完之后的**静止尾巴**（秒）。默认 **0.6**。
    ///
    /// ⚠️ 必须留：打完了立刻消失，观众来不及读完整句。
    /// 它也是"这条 cue 总时长够不够打完"的判据输入。
    #[serde(default = "default_hold_secs")]
    pub hold_secs: f64,

    // =================================================================
    // T1 弹入 / T4 抖动 / T6 底衬（2026-10-11）
    //
    // ⚠️⚠️ **全部 `Option<T>` + `skip_serializing_if`，不用裸值 + "等值就跳过"。**
    //
    // 裸 `f64` + `skip_serializing_if = "等于默认值"` 能满足"老工程一个键都不多"，
    // 但会让**用户显式写的默认值被静默吃掉** —— 读回来仍是默认、渲染逐像素一样，
    // 所以**永远不会有症状**，而"显式写了却读不回来"是本仓最恨的那类静默。
    // `Option` 一举两得：**不写键 = 默认行为**，**写了就是写的那个值**。
    //
    // 底座侧的字段名与口径见 Dhampir 仓的 `docs/text-motion-integration.md`
    // （T1/T3/T4）与 `docs/plan/t6-plate-interface.md`（T6）——
    // **转译器逐字照抄那些名字**，本结构体只是"本仓这一侧的输入形状"。
    // =================================================================
    /// **T1 弹入**：起始缩放倍率。`None` = 1.0（不缩）。参照 `0.7`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_in_from: Option<f64>,

    /// **T1 弹入**：曲线的**终点**倍率。`None` = 1.0（不过冲）。参照 `1.15`。
    ///
    /// ⚠️ **它是曲线端点，不是实际峰值** —— 底座那条 `back_out3`（s=3）在中点
    /// 冲到 1.25，所以实际最大是 `from + (peak - from) × 1.25`
    /// （参照值下 = `0.7 + 0.45 × 1.25 = 1.2625`，实测取整后 1.245）。
    /// 想改"过冲多少"要同时动两个端点。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_in_peak: Option<f64>,

    /// **T1 弹入**：窗口时长（毫秒）。`None` = 0 = **关**（不缩）。
    ///
    /// ⚠️ 三个 T1 字段里**只有它**决定开不开：`from` / `peak` 都给了但它是 0
    /// ⇒ 不缩（"窗口多长"才定义得了"缩到什么程度"，另两个只是端点）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_in_ms: Option<u64>,

    /// **T4 抖动**：幅度（**文档像素**）。`None` = 0 = **关**。
    ///
    /// 与 `rise_in_px` / `transform.x/y` 同一套坐标系（文档像素），不是目标像素。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shake_px: Option<f64>,

    /// **T4 抖动**：基频（Hz）。`None` = 底座默认 18.0。只在 `shake_px > 0` 时有意义。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shake_hz: Option<f64>,

    /// **T6 底衬**：颜色（RGBA）。`None` = **不铺底衬**（一个像素都不多画）。参照 `[0,0,0,140]`。
    ///
    /// ⚠️ alpha 会**乘上**这一帧的不透明度 —— 别在这里再乘一次淡入系数（那会淡两次）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plate_color: Option<[u8; 4]>,

    /// **T6 底衬**：从**字形墨迹**四周各外扩多少（**目标像素**）。`None` ⇒ 底座按 12.0 算。
    ///
    /// ⚠️ 两个 T6 字段的关系是**单向**的：`plate_color = None` 时
    /// `plate_pad_px` **读都不会被读到** ⇒ 只写它是**没有任何效果**的。
    ///
    /// ⚠️ 口径是"从**字形墨迹**外扩"，不是"从行盒外扩"（底座第一版按行盒算，
    /// 实测下侧留白被吃掉 4px，已修）。本仓只管写字段，这个区别只在
    /// "预览里也要画底衬"时才要本仓自己复刻 —— 那时**必须按墨迹算**。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plate_pad_px: Option<f64>,

    // =================================================================
    // T7 摇摆（2026-10-13）
    //
    // 规格：`docs/plan/text-styles-emotion-base-mapping.md` §1.2 ——
    // 「摇摆 | 笑 / 得意；慢摆 = 醉、眩晕 | **角度 + 周期**」。
    // 两个旋钮各一个字段，与那条规格**逐字对应**（别自己发明第三个）。
    //
    // ⚠️⚠️ **它要的"往复"只能靠关键帧** —— 静态 `rotation` 做不出摆动。
    // 底座原先 `evaluate_overlay` **不求值关键帧** ⇒ 写了会被**静默忽略**，
    // 所以规格里这条一直标着 ❌「摇摆做不到」。
    // 2026-10-13 底座 `bcff0a5` 把求值接通了，这里才**第一次**能真做。
    //
    // ⚠️ 仍然 `Option<T>`：`sway_deg = None` 或 `sway_ms = None` = **关**
    //（两个都要给才动得了 —— 只给角度没周期 = 不知道摆多快）。
    // =================================================================
    /// **T7 摇摆**：摆幅（**度**，单边）。`None` = 0 = **关**。
    ///
    /// 写 **8** ⇒ 在 `−8° … +8°` 之间往复（单边 8°，所以峰谷差 16°）。
    /// ⚠️ 口径是**单边幅度**、不是全程 —— 写 8 得到的是 ±8，不是 0…8。
    /// 与 [`Self::sway_ms`] 的关系是**双向**的：两个都给才动。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sway_deg: Option<f64>,

    /// **T7 摇摆**：**一次往复**的周期（毫秒）。`None` = **关**。
    ///
    /// 写 **900** ⇒ 从一端摆到另一端再摆回来用 0.9 秒（一个完整来回）。
    ///
    /// ⚠️ 是**完整来回**，不是半程 —— "摆过去"只有 `sway_ms / 2`。
    /// 规格里"快摆 = 笑 / 慢摆 = 醉"说的就是它：快摆 ~600ms、慢摆 ~1600ms。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sway_ms: Option<u64>,

    // =================================================================
    // T5 真卡拉OK（2026-10-11）
    // =================================================================
    /// **T5 真卡拉OK**：按 **ASR 字级时间戳**逐字切 **SRT 字幕轨**的 cue。**默认关**。
    ///
    /// ⚠️⚠️ **它切的是字幕轨，不是文字元素轨** —— 与 [`Self::typewriter`] **分工不同**：
    ///
    /// | | 切哪条轨 | 切的依据 |
    /// |---|---|---|
    /// | [`Self::typewriter`]（T2） | **文字元素轨** | 等速（`chars_per_sec`，**近似**） |
    /// | `karaoke`（T5） | **SRT 字幕轨** | 字级时间戳（ASR 实测，**硬依据**） |
    ///
    /// ❌ **为什么 T5 不去切文字元素轨**（免得下一个人照着 T2 推）：
    /// 元素文本是 **LLM 另写的标题/吐槽/计数**，提示词**明确要求"别和字幕重复"**
    /// （`llm/segment.rs:110`），而字级时间戳来自**人说的话** ⇒ 两者**压根不是同一个东西**，
    /// 元素文本与 ASR 时间轴**天然对不齐**（不是"命中率低"，是"没关系"）。
    /// 硬切只会让质量闸把**每一条**都作废，而报告里写着"退了 N 条"、
    /// 用户以为开了、实际什么都没发生。元素轨也**没有**字级时间来源
    /// （LLM 编的句子**没人念过**）：想给它逐字只能"按字数均分" —— 那是假的，不做。
    ///
    /// ⚠️ **两者同时开着 ⇒ 各管各的轨，不冲突、也不叠加**（2026-10-11 核对后澄清）。
    ///
    /// 一开始想做成"`karaoke` 赢、T2 让位"，核对之后发现**根本没有冲突**：
    /// 上表已经写明它们切的是**两条不同的轨**（字幕轨 vs 元素轨），
    /// 素材、样式、时间来源都不同 ⇒ **同时开着是合理的组合**
    /// （字幕逐字跟着人声走 + 大字标题按等速打字机出现）。
    /// ⚠️ 但转译器**必须报一条 NOTE** 告诉用户"各自作用在哪条轨"——
    /// 否则用户会以为"叠了两种效果"。
    ///
    /// ⚠️ **缺 `data/karaoke-cues.json`**（老工程、或没跑过 `process`）⇒
    /// **明说降级**（报告里一行），**不产出逐字 cue** ——
    /// **不许静默变回普通字幕**：那会让用户"以为开着、其实没开"。
    ///
    /// ⚠️ 为什么是**裸 `bool`** 而不是 `Option<bool>`（与上面 T1/T4/T6 那批不同）：
    /// `bool` 的"没写"与"写了 `false`"**行为等价**（都是关），所以它**不触发**那条
    /// "用户显式写的默认值被等值 skip 吃掉"的坑 —— 与同段的 `typewriter` 保持一致。
    ///
    /// ⚠️ **`is_empty()` 必须一起判它** —— 见那里的说明。
    #[serde(default)]
    pub karaoke: bool,
}

fn default_chars_per_sec() -> f64 {
    14.0
}
fn default_hold_secs() -> f64 {
    0.6
}

impl Default for ElementsConfig {
    fn default() -> Self {
        Self {
            typewriter: false,
            chars_per_sec: default_chars_per_sec(),
            hold_secs: default_hold_secs(),
            // T1 / T4 / T6：全部 `None` = 全关（老工程零迁移）。
            scale_in_from: None,
            scale_in_peak: None,
            scale_in_ms: None,
            shake_px: None,
            shake_hz: None,
            plate_color: None,
            plate_pad_px: None,
            // T7 摇摆：默认关（老工程零迁移）。
            sway_deg: None,
            sway_ms: None,
            // T5：默认关（老工程零迁移）。
            karaoke: false,
        }
    }
}

impl ElementsConfig {
    /// 空配置（与 `skip_serializing_if` 配套：空 ⇔ 不写这个键 ⇔ 老行为）。
    ///
    /// ⚠️⚠️ **新增字段必须一起判**（2026-10-11）—— 这里曾经**只**判 `!self.typewriter`。
    ///
    /// 加了 T1/T4/T6 那七个字段之后，只判打字机的话：
    /// 「配了 `shake_px = 14`、没开 `typewriter`」的工程会被判成**空** ⇒
    /// `[elements]` 整段**不写出去** ⇒ 用户写的 `shake_px` **静默丢失**
    /// （`process` 日志一切正常、`polish.toml` 里那行还在，唯独成片不抖）。
    /// ★ 与同段那条 `deny_unknown_fields` 是**同族**的错：
    ///   "用户写了配置、程序当他没写"。
    ///
    /// 判据：`默认的_elements_不写进_toml_r4`（全默认才不写）+ checkTextElementLane
    /// 的「只配抖动、不开打字机」那一条（**必须写出去**）。
    ///
    /// ⚠️⚠️ **T5 的 `karaoke` 也必须在这里判**（2026-10-11，加它时同一个坑又摆了一次）：
    /// 漏了它 ⇒「只配了 `karaoke = true`」的工程被判成**空** ⇒ `[elements]` 整段不写
    /// ⇒ **用户开的卡拉OK静默消失**。这正是上面那段说的同一个病，加新字段时**逐条过一遍**。
    pub fn is_empty(&self) -> bool {
        !self.typewriter
            && !self.karaoke
            && self.scale_in_from.is_none()
            && self.scale_in_peak.is_none()
            && self.scale_in_ms.is_none()
            && self.shake_px.is_none()
            && self.shake_hz.is_none()
            && self.plate_color.is_none()
            && self.plate_pad_px.is_none()
            // ⚠️ T7 摇摆（2026-10-13）—— **上面那段警告说的就是这个坑**：
            //    漏判这两个字段 ⇒ 「只配了摆动、没开打字机」的工程被判成**空**
            //    ⇒ `[elements]` 整段不写出去 ⇒ 用户写的 `sway_deg` **静默丢失**
            //    （日志正常、`polish.toml` 里那行还在，唯独字不摆）。
            && self.sway_deg.is_none()
            && self.sway_ms.is_none()
    }

    /// 每秒字数**夹取到可用范围**。
    ///
    /// ⚠️ 配置是手写的 ⇒ 0 / 负数 / NaN / 无穷都进得来。0 会让下面的除法炸
    /// （`chars_per_sec` 当除数），而"因为一个手滑的数字整条出片跑不出来"
    /// 的代价远大于"这条用了默认速度"。
    pub fn safe_chars_per_sec(&self) -> f64 {
        if self.chars_per_sec.is_finite() && self.chars_per_sec > 0.0 {
            self.chars_per_sec
        } else {
            default_chars_per_sec()
        }
    }

    /// 静止尾巴**夹取**（同样挡 NaN / 负数）。
    pub fn safe_hold_secs(&self) -> f64 {
        if self.hold_secs.is_finite() && self.hold_secs >= 0.0 {
            self.hold_secs
        } else {
            default_hold_secs()
        }
    }
}

/// 一条文字元素 cue（**拆之前**的形状）。
///
/// ⚠️ 字段与转译器那边的 cue 对齐（`start`/`end` 是**帧号**、`text` 是整句）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextCue {
    pub start: u32,
    pub end: u32,
    pub text: String,
}

/// 把一条 cue 拆成"逐字出现"的多条（**T2 打字机**）。
///
/// # 返回
///
/// `Vec<TextCue>`：第 i 条的 `text` 是原句的**前 i 个字**，时间区间首尾相接。
///
/// # 为什么不在这里做"打完之后的静止尾巴"
///
/// 尾巴 = **最后一条** cue 的时长比其它条长。这样"打到第 k 个字"这个状态
/// 与"整句显示完"共用同一条 cue（文本相同），少一条 cue、也少一次栅格化
/// （底座的栅格化按**文本**缓存，见 `cache_hits/cache_misses`）。
///
/// # 边界（保守）
///
/// - 开关关着 ⇒ 原样返回一条（**逐字段等于输入**，存量零回归）；
/// - 文本 ≤ 1 个字 ⇒ 原样一条（没有"逐个"可言，拆了反而多一次栅格化）；
/// - cue 时长不够打完 ⇒ **按实际时长压缩**（不延长 cue —— 延长会盖住下一条），
///   且**至少每字 1 帧**（0 帧的 cue 在底座那边是不可见的）；
/// - `end <= start` ⇒ 原样一条（脏输入不该让这里 panic 或产出空列表）。
pub fn split_typewriter(cue: &TextCue, fps: f64, cfg: &ElementsConfig) -> Vec<TextCue> {
    if !cfg.typewriter {
        return vec![cue.clone()];
    }
    let chars: Vec<char> = cue.text.chars().collect();
    // 没有"逐个"可言。
    if chars.len() <= 1 || cue.end <= cue.start || !fps.is_finite() || fps <= 0.0 {
        return vec![cue.clone()];
    }

    let span = (cue.end - cue.start) as f64;
    let per_char_secs = 1.0 / cfg.safe_chars_per_sec();

    // ⚠️ **每字至少 1 帧**：帧不够时宁可**少打几个字**、也不产出 0 帧的 cue
    //    （0 帧 = 底座那边一帧都不显示 = 那个字**永远不会出现**，
    //     而报告里看着"拆了 N 条"，是"报告与事实相反"的又一种形态）。
    let max_steps = span.floor().max(1.0) as u32;
    let wanted = (span / (per_char_secs * fps)).floor() as u32;
    // 打字阶段占多少帧（至少 1 帧、至多"每字 1 帧"）。
    let typing_frames = wanted.clamp(1, max_steps.saturating_sub(0).max(1));

    // 每字推进多少帧（浮点累积，避免整数除法把总时长吃掉）。
    let mut out: Vec<TextCue> = Vec::new();
    let mut shown = 0usize;
    let mut frame = cue.start;
    let hold_frames = (cfg.safe_hold_secs() * fps).round() as u32;

    // 打字阶段：逐个吐字。
    for i in 0..typing_frames {
        // 这一步应该显示到第几个字（按比例推进，保证最后一步刚好是整句）。
        let target = (((i + 1) as f64 / typing_frames as f64) * chars.len() as f64).ceil() as usize;
        let target = target.clamp(1, chars.len());
        let next_frame = cue.start + ((i + 1) as f64 * span / typing_frames as f64).round() as u32;
        let next_frame = next_frame.max(frame + 1).min(cue.end);
        out.push(TextCue {
            start: frame,
            end: next_frame,
            text: chars[..target].iter().collect(),
        });
        shown = target;
        frame = next_frame;
        if frame >= cue.end {
            break;
        }
    }

    // 静止尾巴：整句保持到 cue 结束。
    if shown < chars.len() || frame < cue.end {
        // ⚠️ 尾巴够长才单独占一条；不够长就并入最后一条（多一条 0 帧 cue 没意义）。
        let tail_start = cue.end.saturating_sub(hold_frames);
        if tail_start > frame {
            out.push(TextCue {
                start: frame,
                end: tail_start,
                text: chars[..shown.max(1)].iter().collect(),
            });
            frame = tail_start;
        }
        if frame < cue.end {
            let full = chars[..].iter().collect::<String>();
            // 与最后一条文本相同时**合并**（不产出内容等价的两条 cue）。
            match out.last_mut() {
                Some(last) if last.text == full && last.end == frame => last.end = cue.end,
                _ => out.push(TextCue {
                    start: frame,
                    end: cue.end,
                    text: full,
                }),
            }
        }
    }

    if out.is_empty() {
        return vec![cue.clone()];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_on() -> ElementsConfig {
        ElementsConfig {
            typewriter: true,
            ..Default::default()
        }
    }

    fn cue(start: u32, end: u32, text: &str) -> TextCue {
        TextCue {
            start,
            end,
            text: text.to_string(),
        }
    }

    /// 开关关着 ⇒ **逐字段等于输入**（R4：存量零回归靠这一条）。
    #[test]
    fn 关着的时候一条都不拆() {
        let c = cue(10, 100, "这句话有八个字");
        let out = split_typewriter(&c, 30.0, &ElementsConfig::default());
        assert_eq!(out, vec![c], "关着开关必须原样返回");
    }

    /// 文本只有 1 个字 ⇒ 不拆（没有"逐个"可言）。
    #[test]
    fn 一个字不拆() {
        let c = cue(0, 30, "好");
        let out = split_typewriter(&c, 30.0, &cfg_on());
        assert_eq!(out, vec![c]);
    }

    /// 拆出来的 cue **首尾相接、不重叠、不掉帧**。
    #[test]
    fn 拆出来的区间首尾相接() {
        let c = cue(0, 90, "一二三四五六");
        let out = split_typewriter(&c, 30.0, &cfg_on());
        assert!(out.len() > 1, "应当拆成多条，实得 {}", out.len());
        assert_eq!(out[0].start, c.start, "第一条必须从原起点开始");
        assert_eq!(out.last().unwrap().end, c.end, "最后一条必须到原终点");
        for w in out.windows(2) {
            assert_eq!(w[0].end, w[1].start, "相邻两条必须首尾相接（不留缝、不重叠）");
        }
    }

    /// 每条的文字都是原句的**前缀**，且**逐条变长**（这就是"打字机"）。
    #[test]
    fn 文字逐条变长且都是前缀() {
        let text = "一二三四五六";
        let c = cue(0, 120, text);
        let out = split_typewriter(&c, 30.0, &cfg_on());
        let mut prev_len = 0usize;
        for (i, c2) in out.iter().enumerate() {
            assert!(
                text.starts_with(&c2.text),
                "第 {i} 条「{}」不是原句前缀",
                c2.text
            );
            assert!(
                c2.text.chars().count() >= prev_len,
                "第 {i} 条比上一条短了 —— 那不是打字机"
            );
            prev_len = c2.text.chars().count();
        }
        assert_eq!(
            out.last().unwrap().text, text,
            "最后一条必须是**整句**（否则结尾少字）"
        );
    }

    /// ⚠️ **不许产出 0 帧的 cue**：0 帧在底座那边一帧都不显示 ⇒ 那个字永不出现。
    #[test]
    fn 不许有零帧的_cue() {
        // 极短的 cue（3 帧）+ 长文本 ⇒ 帧不够打完，也必须每字 ≥1 帧。
        let c = cue(0, 3, "一二三四五六七八");
        let out = split_typewriter(&c, 30.0, &cfg_on());
        for (i, c2) in out.iter().enumerate() {
            assert!(c2.end > c2.start, "第 {i} 条是 0 帧（{}..{}）", c2.start, c2.end);
        }
        assert_eq!(out.first().unwrap().start, 0);
        assert_eq!(out.last().unwrap().end, 3);
    }

    /// 脏输入（`end <= start`）⇒ 原样一条，不 panic、不产出空列表。
    #[test]
    fn 脏区间原样返回() {
        for c in [cue(10, 10, "一二三"), cue(10, 5, "一二三")] {
            let out = split_typewriter(&c, 30.0, &cfg_on());
            assert_eq!(out, vec![c.clone()], "脏区间必须原样返回");
        }
    }

    /// 帧率是脏值 ⇒ 原样一条（不 panic）。
    #[test]
    fn 脏帧率原样返回() {
        let c = cue(0, 60, "一二三");
        for fps in [0.0, -30.0, f64::NAN, f64::INFINITY] {
            let out = split_typewriter(&c, fps, &cfg_on());
            assert_eq!(out, vec![c.clone()], "fps={fps} 应当原样返回");
        }
    }

    /// ⚠️ **同一输入必须逐字段可复现**（纯函数、无累积状态）——
    /// 这是规格 §4.1 那条硬约束的机器判据：seek 到第 N 帧与播到第 N 帧必须一样。
    #[test]
    fn 纯函数可复现() {
        let c = cue(7, 133, "十二个字的钩子标题哦");
        let a = split_typewriter(&c, 30.0, &cfg_on());
        let b = split_typewriter(&c, 30.0, &cfg_on());
        assert_eq!(a, b, "同一输入两次调用必须逐字段相同");
        // 再来一次，确认没有跨调用的隐藏状态。
        let c2 = split_typewriter(&c, 30.0, &cfg_on());
        assert_eq!(a, c2);
    }

    /// 配置夹取：0/负数/NaN 的速度与尾巴都退回默认值（不许让除法炸）。
    #[test]
    fn 脏配置被夹取() {
        let bad = ElementsConfig {
            typewriter: true,
            chars_per_sec: 0.0,
            hold_secs: f64::NAN,
            ..Default::default()
        };
        assert_eq!(bad.safe_chars_per_sec(), default_chars_per_sec());
        assert_eq!(bad.safe_hold_secs(), default_hold_secs());
        // 负数也要挡。
        let neg = ElementsConfig {
            typewriter: true,
            chars_per_sec: -5.0,
            hold_secs: -1.0,
            ..Default::default()
        };
        assert_eq!(neg.safe_chars_per_sec(), default_chars_per_sec());
        assert_eq!(neg.safe_hold_secs(), default_hold_secs());
        // 正常值原样通过。
        let ok = ElementsConfig {
            typewriter: true,
            chars_per_sec: 20.0,
            hold_secs: 1.2,
            ..Default::default()
        };
        assert_eq!(ok.safe_chars_per_sec(), 20.0);
        assert_eq!(ok.safe_hold_secs(), 1.2);
    }

    /// 空配置的判据（与 `skip_serializing_if` 配套）。
    #[test]
    fn 空配置只在关着时成立() {
        assert!(ElementsConfig::default().is_empty(), "默认必须算空（不写进 toml）");
        assert!(
            !ElementsConfig {
                typewriter: true,
                ..Default::default()
            }
            .is_empty(),
            "开了开关就不算空 —— 否则这个键不会被写下来，下次跑就丢失了"
        );
    }

    /// ★ **每一个可选字段单独配了都必须算「非空」**（2026-10-13 补）。
    ///
    /// # 为什么必须逐个过一遍
    ///
    /// `is_empty() == true` ⇒ `[elements]` **整段不写出去**。
    /// 所以**漏判一个字段 = 那个字段静默丢失**：日志正常、`polish.toml` 里那行还在、
    /// 唯独成片里没效果。上面 `is_empty` 的注释记的正是 2026-10-11 那次
    ///（只判 `!self.typewriter`，于是「配了 `shake_px`、没开打字机」整段被丢）。
    ///
    /// ⚠️ 那条只测了 `typewriter` / `karaoke` 两个**开关**，
    /// 而**漏判的从来都是 `Option<T>` 那批**（新增字段时最容易被忘）。
    /// 这里逐个构造"只配这一个"的配置 ⇒ **加新字段忘了判 is_empty 会当场红**。
    #[test]
    fn 任一可选字段配了都不算空() {
        // (字段名, 只配这一个的配置) —— 加字段时**往这里加一行**。
        let cases: [(&str, ElementsConfig); 10] = [
            ("scale_in_from", ElementsConfig { scale_in_from: Some(0.7), ..Default::default() }),
            ("scale_in_peak", ElementsConfig { scale_in_peak: Some(1.15), ..Default::default() }),
            ("scale_in_ms", ElementsConfig { scale_in_ms: Some(240), ..Default::default() }),
            ("shake_px", ElementsConfig { shake_px: Some(14.0), ..Default::default() }),
            ("shake_hz", ElementsConfig { shake_hz: Some(18.0), ..Default::default() }),
            ("plate_color", ElementsConfig { plate_color: Some([0, 0, 0, 140]), ..Default::default() }),
            ("plate_pad_px", ElementsConfig { plate_pad_px: Some(12.0), ..Default::default() }),
            ("sway_deg", ElementsConfig { sway_deg: Some(8.0), ..Default::default() }),
            ("sway_ms", ElementsConfig { sway_ms: Some(900), ..Default::default() }),
            // 开关那两个也一并过（它们曾经就是唯一的判据）。
            ("karaoke", ElementsConfig { karaoke: true, ..Default::default() }),
        ];
        for (name, cfg) in cases {
            assert!(
                !cfg.is_empty(),
                "只配了 `{name}` 却被判成空 ⇒ `[elements]` 整段不写出去 ⇒ \
                 `{name}` **静默丢失**（日志正常、toml 里那行还在、成片里没效果）。\
                 修法：在 `is_empty()` 里把 `{name}` 一起判上"
            );
        }
    }
}
