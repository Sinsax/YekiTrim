//! 渲染核心的输入类型定义。
//!
//! 这些类型从 yekitrim-polish 的 project/config.rs 迁入，现在是 `yekitrim-polish` 的字段类型
//! （`polish.toml` 的序列化格式就是它们）；yekitrim-polish 通过 `pub use` 重导出保持既有代码路径兼容。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ===== 时间轴分段 =====

/// Segment 仅话题分组, 运镜由 Camera event 控制
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Segment {
    pub start: f64,
    pub end: f64,

    /// 情绪阈值: low/med/high, 控制该段反应密度
    #[serde(default = "default_fx")]
    pub fx: String,

    #[serde(default)]
    pub note: String,

    /// **叙事角色**（总方案 §8）：这一段在整条片子里干什么。
    ///
    /// 规格 §3.6 明确要求**先做最小版**：
    /// > 完整的六种 `role` + 配方 + 守卫是个大改动。**最小可用版**：只分
    /// > **`hook` / `punch` / 其余**三类 —— 就能拿到 80% 的"起伏感"，改动面小得多。
    ///
    /// ⇒ 取值只有 `hook` / `punch` / `"`（其余，**空串**）。
    ///
    /// ⚠️ **用 `String` 而不是 enum**：`polish.toml` 是**用户会手改**的文件，
    /// 而且是**跨版本**的。enum 遇到认不出的值会让**整个工程反序列化失败**
    /// （`unknown variant`）—— 那意味着"手改了一个字，整个精修配置读不出来"。
    /// 用 `String` 则认不出的值由 [`Self::role`] 归进"其余"，行为与不写一样。
    ///
    /// ⚠️ `#[serde(default)]` + `skip_serializing_if`：**老工程零迁移**（R4）。
    /// 没写过 `role` 的工程重存时**一个字节都不该多出**。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,

    #[serde(default)]
    pub events: Vec<Event>,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Segment {
    /// **归一化后的叙事角色**（§8 的三档最小版）：`hook` / `punch` / `""`（其余）。
    ///
    /// ⚠️ **认不出的值一律归进"其余"（空串）**，不报错、不启发式猜。
    /// 规格 §3.6 只要求三档，所以 `setup`/`build`/`afterglow`/`cta` 这些
    /// **更细的角色值也归"其余"** —— 那是有意的：它们的行为与"其余"相同，
    /// 而"猜一个近似档"会让手写了细角色的工程拿到**没人设计过**的行为。
    ///
    /// 大小写不敏感 + 去空白：`polish.toml` 是手改的（`Hook` / `hook ` 都该认得），
    /// 而"手改了一个大小写就整档失效"是最难查的一类问题（它不报错）。
    pub fn role(&self) -> &'static str {
        match self.role.trim().to_ascii_lowercase().as_str() {
            "hook" => "hook",
            "punch" => "punch",
            _ => "",
        }
    }

    /// 这一段是不是**钩子**（片头抓人那一段）。
    pub fn is_hook(&self) -> bool {
        self.role() == "hook"
    }

    /// 这一段是不是**爆点**（名场面那一段）。
    pub fn is_punch(&self) -> bool {
        self.role() == "punch"
    }
}

fn default_fx() -> String {
    "low".into()
}

// ===== Tagged Event Enum =====

/// 事件的时间一律是「开始 + 结束」两个绝对秒：`time` = 开始，`end` = 结束。
///
/// `end` 写成 `Option` 是为了容错：LLM 偶尔漏写、老工程写的是 `duration`
/// （由 `yekitrim_polish::project::legacy` 在反序列化前换算成 `end`）。缺失时由
/// `process` 按类型补默认值（`Event::default_end`）——`sfx` 例外，`end` 缺失 =
/// 用文件原始长度，不补。
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    // === 贴图 (统一视觉元素, 来源由 sticker_source 配置决定) ===
    Sticker {
        time: f64,
        /// 结束秒。「保持」时长的终点；入场 0.35 / 浮起 0.4 / 淡出 0.3 另计。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<f64>,
        /// LLM 生成的描述关键词（贴纸包名，如 "ysyk"）
        keyword: String,
        /// Step2a LLM 选出的语义 tag（如 "害怕"）
        #[serde(default)]
        sticker_tag: Option<String>,
        /// process/Step2b 解析后的文件名 (不含扩展名)
        #[serde(default)]
        name: Option<String>,
        /// process/Step2b 解析成功后实际使用的来源 (emoji/irasu/local/包名)
        #[serde(default)]
        source: Option<String>,
        /// 落点（盒子**左上角**百分比）。**可省略**（serde default = 空串 = 还没落点）。
        ///
        /// ⚠️ 2026-10-20 P0：这两个字段曾经是**必填**，而落点是 process Step 2b 的活 ——
        /// 模型漏写一个 x 就让**整个 20.5s 窗口**的全部事件作废（missing field x）。
        /// 现在空串 = 还没落点：process Step 2b 会无条件回填真坐标（它不读旧值），
        /// 渲染侧（模板 / engine.js 的 stickerAnchor）对空串/NaN 退到**画面中心**而不是 (0,0)。
        /// 序列化**照写**（R4：有值的文件逐字节不变；空值那种文件以前根本读不进来）。
        #[serde(default)]
        x: String,
        /// 落点（见 x 的说明）：**可省略**，空串 = 还没落点。
        #[serde(default)]
        y: String,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
        /// **手工定过时间**（编辑器里拖过 / 改过开始·结束）：后处理不再把它吸附到字幕边界、
        /// 也不再往后延长尾帧（`align_stickers_to_subtitles` / Step 2c 都跳过）。
        /// 面板上「跟随字幕」清掉它，下一次后处理就恢复自动吸附。缺省 false = 自动。
        /// **手工定过的落点**（`"x% y%"`，**盒子中心** —— 与 `[sticker] position` 同一口径）。
        ///
        /// ⚠️ 它是**逐事件**的（用户口径 2026-10-16：「贴纸和文字层都需要一个可以给每个单独块手动选择位置的功能，
        /// 自动是全局的，手动过后自动无法影响」）。有它 ⇒ 后处理的**自动落点不参与**：
        /// `process` 只把它换算成 `x`/`y`（盒子**左上角**，那才是底座/模板认的那个口径），
        /// 面板上「改落点」写它、「恢复自动」清它。缺省 `None` = 自动 ⇒ 存量工程一个键都不多（R4）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        manual_pos: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        manual: bool,
    },

    // === 运镜 (持续) ===
    Camera {
        time: f64,
        /// 结束秒。普通景别 = 推移到位窗口的终点；`远`（画中画）= 待在画面上的终点。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<f64>,
        /// "远"(画中画 0.55) | "全"(原画 1.0，**缺省**) | "中"(1.3) | "近"(1.5) | "特写"(2.0)。
        /// 缺省是「全」——只写 `anchor` 的事件也应当能解析（2026-10-20 用户口径）。
        #[serde(default = "default_camera_zoom")]
        zoom: String,
        /// 聚焦中心的字面坐标 "x% y%"。由 `anchor` 解析回填，也可直接手写。
        #[serde(default = "default_camera_origin")]
        origin: String,
        /// 起点聚焦目标：`liver` | `danmaku` | `sticker` | `center`。
        /// LLM 只写这个意图，`process` 解析成 `origin`（与贴纸 keyword→name 同一套约定）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<String>,
        /// 终点聚焦目标。写了就是「从这里移到那里」；不写即原地聚焦。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor_to: Option<String>,
        /// 终点坐标（`process` 由 `anchor_to` 解析回填）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to_origin: Option<String>,
        /// 硬切（切镜）：这一条**整档瞬间到位** —— 事件起点直接给到目标 `zoom`/不动点，
        /// 不做 0.6s 缓动、也不从上一个落点推过去（画中画「远」本来就是这条规则，
        /// 现在放开给任意景别）。缺省 false = 平滑运镜。
        #[serde(default, skip_serializing_if = "is_false")]
        cut: bool,
        /// **手持摇摆**（附加变种）：把相机换成「手持」——低频位移 + 轻微倾斜 + 呼吸缩放，
        /// 叠加在当前景别之上（见 [`handheld_sway`]）。
        ///
        /// 与 `zoom` 同一套状态机：**持续状态**，从这条运镜起生效、一直跟到下一条运镜
        /// （把相机交回稳镜头就再写一条不带 `handheld` 的运镜），不是一次性特效；
        /// 起停跟随该事件的过渡窗口（硬切即瞬变）。
        ///
        /// ⚠️ **只有近景（`zoom = "近"`）允许**。它是可选能力，不是自动补的：
        /// `process` 会把非近景上的 `handheld` 直接剔除（`process::fx::cap_handheld_sway`）。
        /// ⚠️ 竖屏（源素材竖）整个运镜都被屏蔽，它跟着一起屏蔽。
        #[serde(default, skip_serializing_if = "is_false")]
        handheld: bool,
        /// **手工定过时间**（见 `Sticker::manual`）：画中画的 `align_pip_shots` 会跳过它。
        #[serde(default, skip_serializing_if = "is_false")]
        manual: bool,
        /// **运镜速度**（规格 `camera-and-noise-spec.md` §1）：`snap` | `fast` | `med` | `slow`。
        ///
        /// 在这之前过渡时长是**写死**的（`td = min(dur × 0.5, 0.6)`）—— 推近/拉远的快慢
        /// 完全一样，LLM 与用户都改不了。
        ///
        /// ⚠️ **缺省必须走原式**（`med` 的算法一个字节不改）：今天所有工程都没写 `speed`，
        /// 存量横屏要**逐字节零回归**（R4）。所以这里是 `Option`、不是带默认值的 `String`：
        /// `None` 与 `Some("med")` 在转译器里走**同一条**分支，但只有前者保证"没写过就不写出"。
        ///
        /// ⚠️ **与 `cut` 互斥**：同时写以 `cut` 为准（硬切 = 过渡 0 秒 = `speed` 无穷快）。
        ///
        /// ⚠️ **`skip_serializing_if` 在这个字段上是冗余的**（2026-10-10 实测）：
        /// `toml` 序列化**本来就跳过 `Option::None`**（写了个探针：`Option<String>` 的 `None`
        /// 不落键，而同结构里的 `bool = false` 照落）。所以"没写过 ⇒ 文件里没有 `speed`"
        /// **由类型本身保证**，去掉这个属性也不会回归 —— 实测拆掉它之后那条零回归用例
        /// **照样绿**：不是判据没锁住，是这件事本来就不靠它。
        /// 留着它有两个理由：① 与同文件的 `manual` / `handheld` 保持一致；
        /// ② 万一哪天字段改成带默认值的 `String`，它就成了唯一的防线。
        /// **别为它写"变异自证"** —— 那个变异永远不会红（这条注释就是那次尝试的记录）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        speed: Option<String>,
        /// **荷兰角 / 倾斜**（规格 `camera-and-noise-spec.md` §4）：整帧绕**聚焦中心**旋转的度数，
        /// 正 = 顺时针。`None` = 不倾斜（**存量工程逐字节零回归**）。
        ///
        /// ⚠️ **它是 `camera` 的字段、不是新事件类型**（规格 §4.5）：荷兰角是**持续状态**
        /// （要停住一段时间），必须跟 `zoom`/`anchor` **同一套状态机** —— 从这条运镜起生效、
        /// 一直跟到下一条运镜。做成"瞬时特效"的话"到点就回正"会变成**闪一下**。
        ///
        /// # ⚠️⚠️ 硬约束：可能与「远」（画中画）**同段出现**
        ///
        /// 整帧旋转 θ 后要盖满画布，需要的缩放是
        /// `max((W·cosθ + H·sinθ)/W, (W·sinθ + H·cosθ)/H)`（画布四角逆旋转后仍落在画面内）。
        /// 实测各档的最大安全角：**`远`(0.55) → 0.00°**、`中`(1.3) → 10.24°、`近`(1.5) → 17.98°。
        ///
        /// ⇒ **「远」一格都转不得**（`scale < 1` 时画面本来就小于画布、左右各有 432px 黑边，
        /// 再转必然露出更多）。而规格 §4.2 的场景表第 3 条恰恰推荐"画中画配合" ——
        /// **那两条互相矛盾**（规格 §4.4b 有完整推导）。按保守方案：
        /// `process::fx::cap_dutch_angle` 会把**与「远」同段**的倾斜**直接剔除**。
        ///
        /// ⚠️ 另一条推论：`scale = 1.0`（不缩放）也转不得 ⇒ 荷兰角**必须自带缩放**，
        /// 由转译器补一个 ≥ 1.181 的缩放（`中` 档的 1.3 已满足）。
        ///
        /// ⚠️ **夹取到 ±6°**（规格 §4.3 的角度上限）：超过就像"拍摄事故"。
        /// 6° 在 `中`/`近` 上都有充足余量（只需 1.1804）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tilt: Option<f64>,
    },

    // === 瞬时特效 (所有 sfx 接受布尔值) ===
    Shake {
        time: f64,
        intensity: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<f64>,
        #[serde(default)]
        anchor: Option<String>,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
    },
    Flash {
        time: f64,
        color: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<f64>,
        #[serde(default)]
        anchor: Option<String>,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
    },
    Blur {
        time: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<f64>,
        #[serde(default)]
        anchor: Option<String>,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
    },
    Vignette {
        time: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<f64>,
        #[serde(default)]
        anchor: Option<String>,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
    },
    HueShift {
        time: f64,
        color: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<f64>,
        #[serde(default)]
        anchor: Option<String>,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
    },
    Noise {
        time: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<f64>,
        #[serde(default)]
        anchor: Option<String>,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
    },
    Pulse {
        time: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<f64>,
        #[serde(default)]
        anchor: Option<String>,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
    },

    // === 覆盖层 (持续) ===
    Overlay {
        time: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<f64>,
        overlay_type: String,
        color: Option<String>,
        opacity: Option<f64>,
        colors: Option<Vec<String>>,
        shape: Option<String>,
        /// 线性渐变的**方向**（CSS 约定：0 = 向上、顺时针增大）。`None` = 参照的缺省方向
        /// （CSS 的 `to bottom right`，即 135°）。
        ///
        /// ⚠️ 与底座那侧的约定**不同**：底座的 `angle` 是度、在 shader 里当 `(cos, sin)` 用
        /// （0 = 向右、90 = 向下），所以转译器要换算（`base = css - 90`）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        angle: Option<f64>,
        #[serde(default, deserialize_with = "flex_sfx")]
        sfx: Option<String>,
        sfx_volume: Option<f64>,
        #[serde(default)]
        anchor: Option<String>,
    },

    // === 纯音效 (无视觉) ===
    Sfx {
        time: f64,
        #[serde(default)]
        name: String,
        #[serde(default)]
        volume: Option<f64>,
        /// 结束秒；不指定则用文件原始长度
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<f64>,
    },

    // === 统一视觉元素（总方案 §5，规格 `elements-spec.md`）===
    //
    // ⚠️ **这一支同时承载 `text` 与 `sticker` 两种 kind**（靠 `kind` 分派），
    // 而不是给文字单开一个 `text` 变体。理由与规格 §2 一致：两者在时间轴上是**同一类东西**
    // （一个有开始/结束的视觉元素，挂在同一条轨道上），差别只在"画什么"。
    // 拆成两个变体的话，配额守卫、贴纸对齐、落点避让、时间轴渲染都得各写两遍。
    //
    // ⚠️ **`elements` 是新写法，`Sticker` 变体仍然在**（§5 的 E2 只是"迁进来"、
    // **不是**把老变体删掉）。老工程里全是 `[[segments.events]] type = "sticker"`，
    // 删变体 = 所有存量工程读不出来（R4）。两个变体长期共存，由 `process` 一并看待。
    Element {
        time: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<f64>,
        /// `"text"` 或 `"sticker"`。**不用 enum**：认不出的值要能给一条像样的报错
        /// （enum 只会说"unknown variant"，而这里要告诉用户"是哪一条元素、写了什么"）。
        ///
        /// ⚠️ **`#[serde(default)]` 不能省**：LLM 会**反复**漏写 `kind`（提示词里
        /// `kind` 与 `text` 挨着，模型很容易只抄一个）。没有 default 时整个工程
        /// **反序列化直接失败** —— 那不是"少一条文字"，而是"这个切片的精修配置读不出来"。
        /// 留空字符串 ⇒ 由 `process` 按"认不出的 kind"处理（当作 `text`，且质量分里
        /// 归到第三类 `element`，能看到异常）。
        #[serde(default)]
        kind: String,
        /// 元素**显示什么**。`kind = "text"` 时是正文；`kind = "sticker"` 时是关键词
        /// （语义同 `Event::Sticker.keyword`，即贴纸包名）。
        text: String,
        /// `text` 的样式预设：`hook` / `chapter` / `comment` / `count`（规格 §2）。
        /// `kind = "sticker"` 时不用。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<String>,
        /// `text` 的**动效档名**（39 档，2026-10-13，见 `docs/plan/text-styles-39-完成计划.md`）。
        ///
        /// 取值是转译器 `ANIM_PRESETS` 里的键（`sway_std` / `slam_squash` / `tilt_sm` …）。
        ///
        /// # ⚠️ 为什么动效**不能**挂在 `[elements]` 上
        ///
        /// `ElementsConfig` 是**全局单例**（`project/config.rs:82`，整片一份），
        /// 而 39 档要**逐条不同**（第 1 条砸入、第 2 条摇摆）——
        /// 全局单例在物理上做不到这件事。所以它必须住**事件**上，与 `style` 并列。
        ///
        /// 与 `style` **正交**：`style` 管字号/描边/颜色，`anim` 管怎么动。
        /// 两者能自由组合（规格 §1.3 的四个情绪族 = 样式 × 动效）。
        ///
        /// ⚠️ **是 `Option<String>` 而不是 enum**（与相邻的 `kind` 同一个理由）：
        /// 认不出的档名要能给一条**说出是哪一条、写了什么**的报错，
        /// 而 enum 只会说 `unknown variant`。转译器把认不出的档名报成 CLAMP。
        ///
        /// ⚠️ **不写 = 没有动效**（不是"用默认档"）⇒ 存量工程逐字节不变（R4）。
        /// 没写时转译器**回落到全局 `[elements]`**（那份降级成了"整片兜底默认"）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anim: Option<String>,
        /// `text` 的**样式档名**（2026-10-13：与 `anim` 并列，两者**可叠加**）。
        ///
        /// # 为什么要与 `anim` 分开
        ///
        /// 上面 `anim` 那一段已经写明「与 `style` **正交** … 两者能自由组合」——
        /// 但**界面**把 40 个档名挤进了一个下拉，用户没法"先定形态、再定怎么动"。
        /// 而底座的模型**本来就是分开的**：`Layer` 上 `transform`（静态形态）/
        /// `keyframes`（时间）/ `per_char`（逐字）是**三个独立字段**。
        ///
        /// ⇒ 这一对字段就是把界面提升到与底座同一个维度。
        ///
        /// # 与 `anim` 的关系
        ///
        /// * 两个新字段**都空**且 `anim` 有值 ⇒ 按转译器 `ANIM_PRESETS` 的
        ///   **参数归属**把 `anim` 拆成这两个（**只读迁移，不写回**）——
        ///   存量工程行为不变（R4）。
        /// * 两个新字段任一有值 ⇒ **以它们为准**，`anim` 被忽略。
        ///
        /// ⚠️ 取值是 `webui/src/components/TimelineEditor.vue::ANIM_KINDS` 里
        /// `axis: 'style'` 那些类的档名（`tilt_std` / `arc_up` / `vtext_bold` …）。
        ///
        /// ⚠️ **不写 = 没设样式**（不是"用默认档"）⇒ 存量工程逐字节不变。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style_anim: Option<String>,
        /// `text` 的**动效档名**（与 `style_anim` 并列，可叠加）。
        ///
        /// 取值是 `axis: 'effect'` 那些类的档名（`shake_std` / `sway_lg` /
        /// `slam_squash` …）。其余语义同 `style_anim`。
        ///
        /// ⚠️ `fade_only` 是**合法**的动效档（它动 opacity），别因为它的
        /// 「参数归属」是 `none` 就在界面上把它藏起来 —— 归属说的是
        /// "它不走 `transform` 那条路"，**不是"它不是动效"**。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effect_anim: Option<String>,
        /// `text` 的**过渡档名**（2026-10-14：入场 / 出场效果，从 `anim` 里**独立出来**）。
        ///
        /// # 为什么要独立成一轴
        ///
        /// 用户口径：「弹出，淡入淡出**独立出来作为过渡模式**（入场出场的效果），
        /// **默认为弹出**」。在此之前 `pop_*` / `shrink_in_*` / `fade_only` 混在
        /// `anim`/`effect_anim` 里，与"抖动/逐字变化"这类**持续型**动效抢同一个下拉
        /// —— 但它们是**两个维度**：过渡管"怎么进来、怎么出去"（一次性、有窗口），
        /// 动效管"在场的时候怎么动"（持续/循环）。两者**本来就可以并存**
        /// （淡入出场 + 全程抖动）。
        ///
        /// # 与 `style_anim` / `effect_anim` 的关系
        ///
        /// 三者**字段互不重叠**（与两轴时代同一条纪律）：一个档名只属于一轴。
        /// 归属由转译器 `ANIM_PRESETS` 与界面 `ANIM_KINDS` 的 `axis` 字段共同表达，
        /// 由 `scripts/check-anim-two-axis.mjs`（已扩成三轴）机器校验。
        ///
        /// ⚠️ **不写 = 不施加过渡**（不是"用弹出档"）⇒ 存量工程逐字节不变（R4）。
        /// 界面上显示成「弹出」只是**视觉默认**，落盘仍写 `None`
        /// —— "没配"与"显式配了弹出"在产物里必须是两个状态，否则 R4 保不住。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        transition_anim: Option<String>,
        /// `text` 的**排版角度**（2026-10-14：倾斜 / 弧形的可调角度，**度**）。
        ///
        /// # 为什么要有它
        ///
        /// 用户口径：「倾斜和弧形**需要有角度参数可调整**」。在此之前角度是
        /// **写死在档位里**的（`ANIM_PRESETS` 里 `tilt_std: { tiltDeg: -6.0 }`、
        /// `arc_up: { perChar: { rotation_to: -14 } }`）—— 用户在界面上只能选
        /// "标准 / 小角度 / 反方向"三选一，**没有任何一处能填一个数**。
        ///
        /// # 语义
        ///
        /// * **正负**：倾斜用（正 = 顺时针；负 = 逆时针，与 `tiltDeg` 同号约定）。
        ///   弧形只用**绝对值**（幅度），符号决定朝上/朝下由档位自己定。
        /// * **缺省**（`None`）= 回落**该档位写死的那个值** ⇒ "没填角度"与改造前
        ///   **逐字节相同**（R4）。
        /// * **范围**：由 `yekitrim-polish` 侧**夹取**（后端是权威，不信前端）。
        ///   ⚠️ 夹取区间必须与前端滑杆**同源**（本仓有 `check-camera-zoom-parity.mjs`
        ///   的先例：一处定义、多处引用）—— 否则"前端能拉到 45、后端夹到 30"
        ///   ⇒ 用户看到 45、成片 30，**且不报错**。
        ///
        /// ⚠️ 用 `f64` 不是 `f32`：与 `tiltDeg` / `squashY` 那套既有精修数值同精度，
        /// 免得同一个角度经两次转换后与档内默认值差出 1e-7（那会让
        /// "没填角度"与"填了等于默认值的角度"产出**不同字节**）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        layout_angle_deg: Option<f64>,
        /// 落点档位：`top` / `center` / `bottom`（`text` 用；`sticker` 走 `x`/`y`）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pos: Option<String>,
        /// `text` 的**颜色命名预设**（2026-10-13）。
        ///
        /// ⚠️⚠️ **颜色链已整条退役**（2026-10-30）。提示词不再教颜色、`ElementColor` 已删除、
        /// process/渲染/转译器都不再消费这个字段。**它留着只有一个理由**：老 `polish.toml`
        /// 里可能有这个键，而本枚举是 `deny_unknown_fields` —— 删字段 = 老工程直接打不开（R4 红线）。
        /// ⇒ **别再给它加消费方**（提示词/面板/转译器解析都不要再读）。判据：
        /// `scripts/check-element-color.mjs`（退役口径 + 老工程可读）。
        ///
        /// 取值：`white` / `pink` / `red` / `yellow` / `cyan` / `green` / `black`。
        ///
        /// # 为什么当初要有它（历史，勿据此复活）
        ///
        /// 在此之前文字颜色**只有** `style` 档位那四个写死的常量（`hook`/`chapter`/`count` 白、
        /// `comment` 暖黄）—— **LLM 完全无法表达颜色**，"这句是愤怒的"只能靠换档位近似。
        /// 这个字段就是给它开的。
        ///
        /// # 为什么是命名枚举而不是 hex 字符串
        ///
        /// 模型写 `#ff80c0` 的出错率很高（少一位、丢 `#`、写成 `rgb()`），每种错法都只在
        /// 出片时才看得出来；而且 hex 对 LLM **没有语义** —— 它推不出 `#FF4040` 是"愤怒"。
        /// 命名两头都解决：名字自带情绪语义、拼错时能报出**具体哪个名字认不出**。
        ///
        /// ⚠️ **认不出的值不许静默**（`process` 会打一行 `⚠` 并退回该档默认色）——
        /// 与 `kind`/`style`/`anim` 同一条纪律。
        ///
        /// ⚠️ **是 `Option<String>` 而不是 enum**（与相邻的 `kind`/`style` 同一个理由）：
        /// 要在报告里说出"**哪一条、写了什么**"，而 enum 只会说 `unknown variant`。
        ///
        /// ⚠️ **不写 = 走 `style` 档位的默认色**（不是"用白色"）⇒ 存量工程逐字节不变（R4）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<String>,
        /// 贴纸语义 tag（`kind = "sticker"`，语义同 `Event::Sticker.sticker_tag`）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sticker_tag: Option<String>,
        /// 落点百分比（`kind = "sticker"`；`"auto"` = 交给避让求解器）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        x: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        y: Option<String>,
        /// 手工定时标记（语义同 `Event::Sticker.manual`：后处理一律不改写）。
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        manual: bool,
        /// 解析后的贴纸文件名 / 来源（由 `process` 填，与 `Event::Sticker` 同语义）
        /// **手工定过的落点**（`"x% y%"`，**盒子中心** —— 与 `[sticker] position` 同一口径）。
        ///
        /// ⚠️ 它是**逐事件**的（用户口径 2026-10-16：「贴纸和文字层都需要一个可以给每个单独块手动选择位置的功能，
        /// 自动是全局的，手动过后自动无法影响」）。有它 ⇒ 后处理的**自动落点不参与**：
        /// `process` 只把它换算成 `x`/`y`（盒子**左上角**，那才是底座/模板认的那个口径），
        /// 面板上「改落点」写它、「恢复自动」清它。缺省 `None` = 自动 ⇒ 存量工程一个键都不多（R4）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        manual_pos: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
    },
}

/// 运镜缺省聚焦中心：画面正中（老行为），有 `anchor` 时由 process 覆盖。
fn default_camera_origin() -> String {
    "50% 50%".to_string()
}

/// 布尔字段「假就序列化时省略」的判定（`skip_serializing_if`）。
/// 少写一行 `cut = false`，老工程的 TOML 不会被重写成噪音。
pub(crate) fn is_false(v: &bool) -> bool {
    !*v
}

/// 运镜缺省档位：**全（原画不缩放，1.0）**。
///
/// 2026-10-20 用户口径：「增加一个默认 1.0 作为默认景别」 ⇒ 缺省从「中」(1.3) 改成「全」(1.0)。
/// ⚠️ **认不出的档位仍按中景处理**（`CameraStyle::factor` 的 `_` 分支），两者不再一致 ——
/// 这是刻意的：写错档位不该静默变成"原画"，而**没写**档位就是用户要的"全"。
fn default_camera_zoom() -> String {
    "全".to_string()
}

impl Event {
    pub fn get_time(&self) -> f64 {
        match self {
            Event::Sticker { time, .. } => *time,
            Event::Camera { time, .. } => *time,
            Event::Shake { time, .. } => *time,
            Event::Flash { time, .. } => *time,
            Event::Blur { time, .. } => *time,
            Event::Vignette { time, .. } => *time,
            Event::HueShift { time, .. } => *time,
            Event::Noise { time, .. } => *time,
            Event::Pulse { time, .. } => *time,
            Event::Overlay { time, .. } => *time,
            Event::Sfx { time, .. } => *time,
            Event::Element { time, .. } => *time,
        }
    }

    /// 写入开始秒。⚠️ **凡是要改事件时间的后处理都走这里**，别在各处 `match` ——
    /// 贴纸有两个载体（`Sticker` / `Element{kind:"sticker"}`），逐处匹配漏一个就是
    /// "这一类事件的时间在界面上改了、后处理却不认"。
    pub fn set_time(&mut self, value: f64) {
        match self {
            Event::Sticker { time, .. } => *time = value,
            Event::Camera { time, .. } => *time = value,
            Event::Shake { time, .. } => *time = value,
            Event::Flash { time, .. } => *time = value,
            Event::Blur { time, .. } => *time = value,
            Event::Vignette { time, .. } => *time = value,
            Event::HueShift { time, .. } => *time = value,
            Event::Noise { time, .. } => *time = value,
            Event::Pulse { time, .. } => *time = value,
            Event::Overlay { time, .. } => *time = value,
            Event::Sfx { time, .. } => *time = value,
            Event::Element { time, .. } => *time = value,
        }
    }

    /// 结束秒（`None` = 没写，由调用方按 `default_end` 或文件长度兜底）。
    pub fn end(&self) -> Option<f64> {
        match self {
            Event::Sticker { end, .. } => *end,
            Event::Camera { end, .. } => *end,
            Event::Shake { end, .. } => *end,
            Event::Flash { end, .. } => *end,
            Event::Blur { end, .. } => *end,
            Event::Vignette { end, .. } => *end,
            Event::HueShift { end, .. } => *end,
            Event::Noise { end, .. } => *end,
            Event::Pulse { end, .. } => *end,
            Event::Overlay { end, .. } => *end,
            Event::Sfx { end, .. } => *end,
            Event::Element { end, .. } => *end,
        }
    }

    /// 写入结束秒（`None` = 清空，回到「按默认值兜底」）。
    pub fn set_end(&mut self, value: Option<f64>) {
        match self {
            Event::Sticker { end, .. } => *end = value,
            Event::Camera { end, .. } => *end = value,
            Event::Shake { end, .. } => *end = value,
            Event::Flash { end, .. } => *end = value,
            Event::Blur { end, .. } => *end = value,
            Event::Vignette { end, .. } => *end = value,
            Event::HueShift { end, .. } => *end = value,
            Event::Noise { end, .. } => *end = value,
            Event::Pulse { end, .. } => *end = value,
            Event::Overlay { end, .. } => *end = value,
            Event::Sfx { end, .. } => *end = value,
            Event::Element { end, .. } => *end = value,
        }
    }

    /// 缺省结束秒 = `time + 该类型的默认时长`。
    /// 与 `process::fx::transient_default_duration` 的瞬时表一致（那份表还管上限，
    /// 这里只在「没写 end」时兜底，且不夹上限 —— 夹取在 `normalize_event_ends`）。
    pub fn default_end(&self) -> f64 {
        let t = self.get_time();
        t + match self {
            Event::Sticker { .. } => 2.0,
            Event::Camera { .. } => 1.0,
            Event::Overlay { .. } => 1.5,
            Event::Shake { .. } => 0.3,
            Event::Flash { .. } => 0.25,
            Event::Blur { .. } => 0.3,
            Event::Vignette { .. } => 0.4,
            Event::HueShift { .. } => 0.35,
            Event::Noise { .. } => 0.3,
            Event::Pulse { .. } => 0.4,
            // 音效不补：`None` 是「用文件原始长度」的合法语义
            Event::Sfx { .. } => 0.0,
            // 元素：文字与贴纸都是**持续类**，缺省 2.0 与 `Sticker` 同档
            // （§5 的元素时长规则另有「最短 1.2s / 最长 5s」的守卫，那在 `process` 里、
            //  不在这里 —— 这里只管「没写 end」时的兜底）。
            Event::Element { .. } => 2.0,
        }
    }

    /// 结束秒（兜底后）；`sfx` 缺 end 时返回 `None`（= 文件原始长度）。
    pub fn end_or_default(&self) -> Option<f64> {
        match self.end() {
            Some(e) => Some(e),
            None => match self {
                Event::Sfx { .. } => None,
                _ => Some(self.default_end()),
            },
        }
    }

    /// 时间是不是**手工定的**（只有贴纸/文字元素与运镜有这个概念，其余类型恒 false）。
    ///
    /// ⚠️⚠️ **`Element` 必须一起认**（2026-10-15 用户报「贴纸无法加入」）：编辑器「贴纸轨」
    /// 新建的贴纸走 `Element{kind:"sticker"}`，它**有** `manual` 字段、编辑器也**打了**标记，
    /// 但这里不认 ⇒ 标记形同不存在 ⇒ `cap_events` 抽稀会把它删掉（"我加的贴纸过两秒自己没了"），
    /// `align_stickers_to_subtitles` 也会把时间吸走。
    pub fn is_time_manual(&self) -> bool {
        match self {
            Event::Sticker { manual, .. } | Event::Camera { manual, .. } => *manual,
            Event::Element { manual, .. } => *manual,
            _ => false,
        }
    }

    /// **手工定过的落点**（`"x% y%"`，**盒子中心**）—— 只有贴纸与文字元素有这个概念，其余类型恒 `None`。
    ///
    /// ⚠️ 消费侧**必须**用它（而不是各自 `match` 变体）：载体有两个（`Event::Sticker` 与
    /// `Event::Element{kind:"sticker"}`），漏一个就是"手动落点在那一半上不生效"（本仓踩过同形状的坑）。
    pub fn manual_pos(&self) -> Option<&str> {
        match self {
            Event::Sticker { manual_pos, .. } => manual_pos.as_deref(),
            Event::Element { manual_pos, .. } => manual_pos.as_deref(),
            _ => None,
        }
    }

    /// 清掉「手工落点」（面板上的「恢复自动」）。返回"原本有没有"。
    pub fn clear_manual_pos(&mut self) -> bool {
        match self {
            Event::Sticker { manual_pos, .. } => manual_pos.take().is_some(),
            Event::Element { manual_pos, .. } => manual_pos.take().is_some(),
            _ => false,
        }
    }
    /// 标记/清除「手工时间」。**只有贴纸与运镜有该字段**，其余类型是 no-op。
    pub fn set_time_manual(&mut self, value: bool) {
        match self {
            Event::Sticker { manual, .. } | Event::Camera { manual, .. } => *manual = value,
            Event::Element { manual, .. } => *manual = value,
            _ => {}
        }
    }
    /// **这个事件是不是"贴纸"**（不管它是哪一代写法）。
    ///
    /// 贴纸有**两个载体**，长期共存：
    /// * `Event::Sticker` —— 老写法（LLM 生成的、老工程里的）；
    /// * `Event::Element { kind: "sticker" }` —— §5 E2 迁进来的，也是**编辑器「贴纸轨」
    ///   新建时产的那一支**（`TimelineEditor::addElementAt('sticker')` 一律产 `element`）。
    ///
    /// ⚠️ **凡是"按贴纸处理"的后处理，都必须同时认这两种** —— 只认 `Event::Sticker`
    /// 的写法会**静默跳过**编辑器新建的贴纸，症状是"手动加的贴纸不显示、LLM 放的正常"。
    /// 这个不对称极难归因（两边各自都正常）。
    pub fn is_sticker(&self) -> bool {
        match self {
            Event::Sticker { .. } => true,
            Event::Element { kind, .. } => kind == "sticker",
            _ => false,
        }
    }

    /// **这个事件是不是文字元素**（`Element { kind: "text" }`）。
    ///
    /// 与 [`Event::is_sticker`] 成对：`Element` 这一支同时装两种 kind，按下标/字段匹配时
    /// 只写一半就会静默漏掉另一边（本仓在贴纸那一半上踩过一次，见上面那段说明）。
    ///
    /// ⚠️ **空 `kind` 不算**：它可能是手改到一半/老数据，`process` 自己会按「认不出的 kind」
    /// 给出提示；这里只认明确写了文字的（`normalize_segments_json` 给漏写 kind 的补的就是 "text"）。
    pub fn is_text_element(&self) -> bool {
        matches!(self, Event::Element { kind, .. } if kind.trim().eq_ignore_ascii_case("text"))
    }

    /// 贴纸的**关键词**（= 贴纸包名 / 来源关键字），非贴纸返回 `None`。
    ///
    /// ⚠️ 字段名按载体不同：`Event::Sticker` 在 `keyword`，`Event::Element` 在 `text`
    /// （见 `Element::text` 的文档注释：`kind = "sticker"` 时它就是关键词）。
    /// 这与前端 `TimelineEditor::onStickerPick` 的"关键词落在哪个字段由载体决定"是同一条口径。
    pub fn sticker_keyword(&self) -> Option<&str> {
        match self {
            Event::Sticker { keyword, .. } => Some(keyword.as_str()),
            Event::Element { kind, text, .. } if kind == "sticker" => Some(text.as_str()),
            _ => None,
        }
    }

    /// 贴纸**解析后的文件名 stem**（`process` Step 2 写回的那个值）。
    /// 贴纸的 tag（「包内哪个语义组」）。两个载体同名，直接复用。
    pub fn sticker_tag(&self) -> Option<&str> {
        match self {
            Event::Sticker { sticker_tag, .. } => sticker_tag.as_deref(),
            Event::Element { sticker_tag, .. } => sticker_tag.as_deref(),
            _ => None,
        }
    }

    /// 写入贴纸的 `name` + `source`（Step 2 / Step 2b 解析结果）。
    /// ⚠️ 读写在**同一处**，两个载体字段不同名，散着写必漏。
    pub fn set_sticker_resolved(&mut self, source: &str, name: &str) {
        match self {
            Event::Sticker { name: n, source: s, .. } => {
                *n = Some(name.to_string());
                *s = Some(source.to_string());
            }
            Event::Element { name: n, source: s, kind, .. } if kind == "sticker" => {
                *n = Some(name.to_string());
                *s = Some(source.to_string());
            }
            _ => {}
        }
    }

    pub fn sticker_name(&self) -> Option<&str> {
        match self {
            Event::Sticker { name, .. } => name.as_deref(),
            Event::Element { kind, name, .. } if kind == "sticker" => name.as_deref(),
            _ => None,
        }
    }

    /// 贴纸**实际使用的来源**（`emoji` / `irasu` / 包名），由 `process` Step 2 解析写回。
    pub fn sticker_source(&self) -> Option<&str> {
        match self {
            Event::Sticker { source, .. } => source.as_deref(),
            Event::Element { kind, source, .. } if kind == "sticker" => source.as_deref(),
            _ => None,
        }
    }

    /// 贴纸的**时间**（`try_resolve_sticker` 按时间做分布均衡时要读）。
    pub fn sticker_time(&self) -> Option<f64> {
        match self {
            Event::Sticker { time, .. } => Some(*time),
            Event::Element { kind, time, .. } if kind == "sticker" => Some(*time),
            _ => None,
        }
    }
    /// 贴纸的**落点 + 时间窗口**：`(x, y, time, end)`。
    ///
    /// 四个值一起取是因为落点求解器（Step 2b）与障碍表都按这一组用，分开取会写出
    /// 四段几乎相同的 `match`。`end` 缺省回落到 `time`（瞬时）。
    /// ⚠️ 返回 `Option<(&str, &str)>` 里的两个坐标**都是 `Option`**：
    /// `Event::Sticker.x/y` 是必填 `String`，而 `Event::Element.x/y` 是**可选** `Option<String>`
    /// （元素轨的落点由 `process` 算，新建时可以不写）。调用方按 `None` 走自己的缺省，
    /// 这里**不替它编一个值** —— 编了就是把"用户没写"和"用户写了 50%"混成一件事。
    pub fn sticker_span(&self) -> Option<(Option<&str>, Option<&str>, f64, f64)> {
        match self {
            Event::Sticker { x, y, time, end, .. } => {
                Some((Some(x.as_str()), Some(y.as_str()), *time, end.unwrap_or(*time)))
            }
            Event::Element { kind, x, y, time, end, .. } if kind == "sticker" => {
                Some((x.as_deref(), y.as_deref(), *time, end.unwrap_or(*time)))
            }
            _ => None,
        }
    }

    /// 贴纸的**结束时间**（`Option`，Step 2c 尾帧延长要改它）。
    pub fn sticker_end_mut(&mut self) -> Option<&mut Option<f64>> {
        match self {
            Event::Sticker { end, .. } => Some(end),
            Event::Element { kind, end, .. } if kind == "sticker" => Some(end),
            _ => None,
        }
    }

    /// 兼容旧调用点（wgpu 的 `timeline.rs`）：结束与开始之差，缺省 0.5s。
    pub fn duration(&self) -> f64 {
        match self.end() {
            Some(e) => (e - self.get_time()).max(0.0),
            None => match self {
                Event::Sticker { .. } | Event::Camera { .. } | Event::Overlay { .. } => 0.5,
                _ => 0.5,
            },
        }
    }
}

/// sfx 字段的灵活反序列化: 接受字符串/null/布尔值
/// "impact-light" → Some("impact-light"), null → None
/// false → None, true → Some("pop-notice")
fn flex_sfx<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::String(s) => Ok(if s.is_empty() { None } else { Some(s) }),
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Bool(b) => Ok(if b { Some("pop-notice".into()) } else { None }),
        _ => Ok(None),
    }
}

// ===== 视觉样式 =====

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct StyleRoot {
    #[serde(default)]
    pub subtitle: SubtitleStyle,
    #[serde(default)]
    pub danmaku: DanmakuStyle,
    #[serde(default)]
    pub camera: CameraStyle,
}

// ===== 运镜样式 =====

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct CameraStyle {
    /// 中景缩放 (camera zoom="中")
    #[serde(default = "default_zoom_mid")]
    pub zoom_mid: f64,
    /// 近景特写缩放 (camera zoom="近")
    #[serde(default = "default_zoom_close")]
    pub zoom_close: f64,
    /// **全**（原画不缩放）= `camera zoom="全"`，**缺省档位**（2026-10-20 用户口径）。
    ///
    /// 1.0 = 不做任何缩放：与"没有运镜"在画面上等价，但它是**显式的一档**（能写进文件、
    /// 能在编辑器里选、能参与过渡）。新增它没有动任何既有档位的值 ⇒ 存量工程零影响。
    #[serde(default = "default_zoom_full")]
    pub zoom_full: f64,
    /// **特写**缩放（`camera zoom="特写"`，规格 `camera-and-noise-spec.md` §3.3 方案 A）。
    ///
    /// 用户拍板 **2.0**（2026-10-20，由 1.8 上调：「改成 2.0 倍率」）。`近`(1.5) 保持不变
    /// ⇒ 存量工程里显式写过的档位零影响（只有"从没写过运镜"的段会跟着缺省档位走）。
    ///
    /// ⚠️ **露边余量比 `近` 大得多**：单侧可平移 `(1.8−1)×1920/2 = 768px`
    /// （`近` 是 480px、`中` 是 288px）。参考基准是那次 `scale=1.3` 的 622px 实测。
    /// ⇒ 特写档下运镜**更"钉"**（平移空间更大是好事，但画面也更满）。
    ///
    /// ⚠️ **特写必须强制 `anchor = liver`**（用户拍板）：放大 1.8 倍后弹幕/贴纸区常被
    /// 挤出画面，"聚焦弹幕"会看不到目标。守卫在 `process::fx::cap_closeup_anchor`。
    #[serde(default = "default_zoom_closeup")]
    pub zoom_closeup: f64,
    /// **微**（`camera zoom="微"`）= **1.06**（2026-10-20 运镜导演化 §一②「句读级微推」）。
    ///
    /// 它不是给人选的第五档，而是 **process 侧逐句读生成**的极轻推拉用的那一档：
    /// 1.06 在 1080p 上是 ~57px —— 肉眼能看出"画面在呼吸"，但不会像 `中`(1.3) 那样变成"推镜头"。
    /// ⚠️ 1.00~1.06 之间**没有别的表达方式**：`zoom` 是标签字符串，写数值会被当成认不出的档
    ///    回落 `中`(1.3)（历史行为）—— 这正是必须新增这一档的原因。
    #[serde(default = "default_zoom_micro")]
    pub zoom_micro: f64,
    /// 画中画缩放 (camera zoom="远")。小于 1 = 画面缩小、四周留黑边
    #[serde(default = "default_zoom_wide")]
    pub zoom_wide: f64,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl CameraStyle {
    /// 档位名 → 缩放系数。认不出的档位按中景处理（与渲染侧的历史行为一致）。
    ///
    /// 四个渲染/求值路径（render-core、模板 canvas、模板 GSAP、engine.js）都必须走同一张表，
    /// 否则「预览看到的不是成片」——1.5/1.8 的硬编码就是这么来的。
    ///
    /// 五档（2026-10-20）：`远`(0.55) / **`全`(1.0，缺省)** / `中`(1.3) / `近`(1.5) / `特写`(2.0)。
    /// **认不出的档位仍按中景**（历史行为，别改成"全"：写错不该静默变原画）。
    pub fn factor(&self, label: &str) -> f64 {
        match label.trim() {
            "特写" => self.zoom_closeup,
            "近" => self.zoom_close,
            "微" => self.zoom_micro,
            "远" => self.zoom_wide,
            "全" => self.zoom_full,
            _ => self.zoom_mid,
        }
    }
}

impl Default for CameraStyle {
    fn default() -> Self {
        Self {
            zoom_full: default_zoom_full(),
            zoom_micro: default_zoom_micro(),
            zoom_mid: default_zoom_mid(),
            zoom_closeup: default_zoom_closeup(),
            zoom_close: default_zoom_close(),
            zoom_wide: default_zoom_wide(),
            extra: HashMap::new(),
        }
    }
}

// ===== 手持摇摆（`Event::Camera::handheld` 的固定参数表）=====

/// 手持摇摆的**固定参数**：一个开关、一套参数 ——
/// 不开放 `[style.camera]` 旋钮、也没有事件级强弱（用户已定）。
///
/// 幅度基准是 **1920×1080 输出像素 / 度**；DOM 预览换成画布占比（百分比）与 px 同构，不用换算。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandheldSway {
    /// 横向位移（输出像素，1920 基准）
    pub dx: f64,
    /// 纵向位移（输出像素，1080 基准）
    pub dy: f64,
    /// 倾斜（度，绕**聚焦中心**，正=顺时针）
    pub rot: f64,
    /// 呼吸缩放倍率（绕聚焦中心，1.0 = 不缩放）
    pub scale: f64,
}

impl HandheldSway {
    /// 静止：未开启、或幅度为 0（竖屏 / 非近景被剔除后）。
    pub const OFF: HandheldSway = HandheldSway {
        dx: 0.0,
        dy: 0.0,
        rot: 0.0,
        scale: 1.0,
    };
}

/// 手持摇摆波形：**纯 `t` 的函数**，无累积状态 ——
/// 所以跳帧求值、并行求值、seek 到任意时刻，结果都一样（与其余瞬时效应的窗口函数同一约定）。
///
/// 四个低频正弦叠加：纵横位移各两路（主次频不成整数比，避免机械的钟摆感）+ 轻微倾斜 + 呼吸缩放。
/// `amp` 是状态机给出的幅度 0~1（与景别同一段缓动窗口，见 `timeline::get_active_camera`）：
/// 1 = 全幅摇摆，0 = 静止，中间值只在起停的 0.6s 过渡里出现。
///
/// ⚠️ 改这里的常数必须同步 `templates/index.html::handheldSway` 与
/// `webui/src/polish/engine.js::handheldSway`（后者由 `webui/scripts/check-handheld.mjs` 对拍，
/// Rust↔JS 由 `timeline.rs` 的 `handheld_sway_matches_js_reference` 钉住）。
#[cfg(test)]
mod camera_zoom_tests {
    use super::*;

    /// **五档表**（2026-10-20 用户口径：「特写 2.0」「增加一个默认 1.0 作为默认景别」）。
    ///
    /// 变异自证：把 factor 里那行「全」删掉 ⇒ 本条红（会落到兜底的中景 1.3）。
    #[test]
    fn factor_covers_all_five_tiers() {
        let cam = CameraStyle::default();
        assert!((cam.factor("远") - 0.55).abs() < 1e-9);
        assert!((cam.factor("全") - 1.0).abs() < 1e-9, "全新档 = 原画不缩放");
        assert!((cam.factor("中") - 1.3).abs() < 1e-9);
        assert!((cam.factor("近") - 1.5).abs() < 1e-9);
        assert!((cam.factor("特写") - 2.0).abs() < 1e-9, "特写 1.8 → 2.0（用户拍板）");
        // 认不出的档位仍按中景（写错不该静默变成原画）
        assert!((cam.factor("瞎写") - 1.3).abs() < 1e-9);
        assert!((cam.factor("  近 ") - 1.5).abs() < 1e-9, "两侧空白要吃掉");
    }

    /// **缺省景别 = 全（1.0）**：模型漏写 zoom 时落到这一档。
    ///
    /// 变异自证：把 default_camera_zoom 改回「中」 ⇒ 本条红。
    #[test]
    fn default_camera_zoom_is_full() {
        assert_eq!(default_camera_zoom(), "全");
        assert!((CameraStyle::default().factor(&default_camera_zoom()) - 1.0).abs() < 1e-9);
    }
}
pub fn handheld_sway(t: f64, amp: f64) -> HandheldSway {
    if amp <= 0.0 {
        return HandheldSway::OFF;
    }
    const TAU: f64 = std::f64::consts::TAU;
    HandheldSway {
        dx: (6.5 * (TAU * 0.47 * t).sin() + 3.0 * (TAU * 0.83 * t + 1.7).sin()) * amp,
        dy: (5.0 * (TAU * 0.59 * t + 0.9).sin() + 2.4 * (TAU * 1.13 * t + 2.4).sin()) * amp,
        rot: (0.32 * (TAU * 0.37 * t + 0.4).sin() + 0.16 * (TAU * 0.91 * t + 2.9).sin()) * amp,
        scale: 1.0 + 0.007 * (TAU * 0.53 * t + 1.1).sin() * amp,
    }
}

/// 中景 / 近景倍率。
///
/// **只放大不缩小就不会露底**：运镜是绕「不动点」做 `scale`，`s ≥ 1` 时四周边缘一律向外扩张
/// （`overflow:hidden` 裁掉多出来的部分），画面上不可能出现背景色/背景层；只有画中画
/// （`zoom_wide < 1`）才刻意留黑边。所以这两个值不存在「推得太近会出框」的问题，只有
/// 「推得够不够近」的观感问题——调大是安全的。
/// 全（原画不缩放）：**缺省档位**（2026-10-20 用户口径「增加一个默认 1.0 作为默认景别」）。
fn default_zoom_full() -> f64 {
    1.0
}
/// **微**：句读级微推用的极轻一档（1.06 = 1080p 上 ~57px）。
fn default_zoom_micro() -> f64 {
    1.06
}
fn default_zoom_mid() -> f64 {
    1.3
}
fn default_zoom_close() -> f64 {
    1.5
}
/// 特写倍率（**2.0**，2026-10-20 用户口径「改成 2.0 倍率」；此前是 1.8）。
///
/// 历史注记（保留，免得再被问一遍）：1.8 时与 `近`(1.5) 的差距是 1.35×，2.0 是 1.33×，
/// 观感上几乎一样，而单侧可平移量从 768px 涨到 960px（画面更满）。用户在此基础上仍然要 2.0
/// —— **以用户口径为准**，别再按旧注记改回去。
fn default_zoom_closeup() -> f64 {
    2.0
}
/// 画中画倍率：0.55 = 画面缩到 55%，四周黑边各 `(1-0.55)/2×1920 = 432px`
/// （0.7 时是 288px）。旧工程里的 0.7 由 `cmd_process` 的迁移改成新默认。
fn default_zoom_wide() -> f64 {
    0.55
}

// ===== 字幕样式 =====

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct SubtitleStyle {
    #[serde(default = "default_font")]
    pub font: String,

    #[serde(default = "default_sub_size")]
    pub size: u32,

    #[serde(default = "default_sub_color")]
    pub color: String,

    #[serde(default = "default_sub_stroke")]
    pub stroke: String,

    /// **外圈描边**：在 `stroke` 那一圈**外面**再加一圈（常态白）。
    ///
    /// 格式与 `stroke` 完全一样：`"4px #ffffff"`（宽度 + 颜色）。
    ///
    /// # 为什么要有它（2026-10-15 用户口径）
    ///
    /// 「给字幕和文字层描边**再增加一层白色描边**」—— 白在外、深在内那套双层描边。
    /// 本仓这一侧只负责把值交给两条消费方：底座（转译器写进 `SubtitleStyle.outer_stroke`）
    /// 与 DOM 预览（4 向 `text-shadow` 兜底）。⚠️ 两边**同一条口径**：
    /// 都是「在 `stroke` 那一圈**之外**再向外这么宽」。
    ///
    /// ⚠️ **默认就是开着**（`4px #ffffff`）—— 用户要的是「加一层白边」这个观感本身，
    /// 不是「提供一个能加白边的开关」。写空串 = 不画。
    #[serde(default = "default_sub_outer_stroke")]
    pub outer_stroke: String,

    #[serde(default = "default_highlight_color")]
    pub highlight_color: String,

    #[serde(default = "default_sub_position")]
    pub position: String,

    /// 字幕**垂直落点**：文字带底边距画布底边的百分比（0~50）。
    ///
    /// ⚠️ **`None` ≠ 默认值，而是"老工程没写过"** —— 这是**存量零回归（R4）的关键**：
    /// - `None`（缺省）⇒ 落点走**历史上那条写死的公式**（`cy = CH - 120 * CKH`），
    ///   与改动前**逐字节相同**；
    /// - `Some(pct)` ⇒ 落点按百分比算。
    ///
    /// 所以这里**故意不给 Default 值**：给了就分不出"用户选了 12%"与"没写过"，
    /// 而 `120/1080 = 11.11%` 与 12% 并不相等 —— 用 % 表达"等于今天"必然留一点偏差。
    /// ⚠️ **不要在 `impl Default` 里给它填 `Some(12.0)`**。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_pct: Option<f64>,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Default for SubtitleStyle {
    fn default() -> Self {
        Self {
            font: default_font(),
            size: default_sub_size(),
            color: default_sub_color(),
            stroke: default_sub_stroke(),
            outer_stroke: default_sub_outer_stroke(),
            highlight_color: default_highlight_color(),
            position: default_sub_position(),
            position_pct: None,
            extra: HashMap::new(),
        }
    }
}

fn default_font() -> String {
    // ⚠️ **2026-10-02 由 `LXGW WenKai` 换成 `Noto Sans SC`**（自带中文那支换了）。
    // 这是**新工程**没选字体时写进 `polish.toml` 的值，所以它与：
    //   * `yekitrim_core::fonts::fallback_chain()` 首项
    //   * `render/server.rs::SYSTEM_FONT_STACK` 首项
    // 三处**必须同名**，否则"新工程默认"与"库里装的那支"对不上（当年 P1-e 那类对不上的老坑）。
    // ⚠️ 老工程不受影响：它们的 `font` 字段已显式写着旧值，读到什么用什么（`family_for` 原文保留）。
    "Noto Sans SC".into()
}
fn default_sub_size() -> u32 {
    72
}
fn default_sub_color() -> String {
    "#dcbda0".into()
}
fn default_sub_stroke() -> String {
    "12px #403c3b".into()
}
/// 外圈描边默认值：**白、`auto`**（单位与 `stroke` 一样是 CSS 居中宽）。
///
/// ⚠️ **`auto` = 粗细跟随第一层描边**（2026-10-16 用户口径：「第二层的白描边…粗细跟随第一层的
///    宽度」）：内圈多宽，白环就多厚。以前是写死的 `4px` —— 用户把第一层从 12px 调到 24px 时
///    白边纹丝不动，看着就是「白边太细、不成比例」。
/// ⚠️ 写具体数字（`"8px #ffffff"`）**仍然按固定宽度走**，老工程与手写的值不被改写。
/// ⚠️ 两处默认值必须同值：这里与转译器的 `DEFAULT_OUTER_STROKE`，判据 `check-text-style.mjs` ⑥d。
fn default_sub_outer_stroke() -> String {
    "auto #ffffff".into()
}
fn default_highlight_color() -> String {
    // A 套「柔和莫兰迪」（2026-10-20）：旧 "#f56b41"（ΔE 11.2，压描边对比度 5.85→6.24）。
    // 封面 hl_color 直接读这一条（cover.rs::cover_frame_vars）—— 改这里两处一起变。
    "#CF8A72".into()
}
fn default_sub_position() -> String {
    "bottom".into()
}

// ===== 弹幕样式 =====

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct DanmakuStyle {
    /// 弹幕字体：字体库里的 id，**空 = 用出厂默认文字字体**（`default_font()` 那支）。
    /// 与 [`SubtitleStyle::font`] 同一套取值规则。
    ///
    /// ⚠️ **2026-10 口径变更：空值不再是「跟随字幕那条字体栈」**（用户拍板）。
    /// 旧口径写着"空 = 跟随字幕"，但那个契约**三条腿里只兑现了两条**，而且在**成立的那两条上
    /// 也是错的**：
    /// * `[style.subtitle] font` 里存的是**族名**（`polish init` 写 `Noto Sans SC`、用户在字体库
    ///   里可以设成任意一支），而弹幕这一栏存的是**字体库 id** —— 两个键的**格式不同**，
    ///   拿字幕那个值当 id 用本来就不对；
    /// * 于是"跟随字幕"的工程里，预览（`engine.js::dmFontFamily`）与出片模板
    ///   （`templates/index.html::dmFontFamily`）会把字幕那串族名塞进弹幕字体栈首 ——
    ///   **弹幕看起来和字幕一模一样**，而用户对"弹幕默认"的预期是**出厂默认**、不是跟着字幕走；
    /// * 更糟的是**真出片通道**（底座 Dhampir）从来不实现这一档：转译器把空值填成 `'Sans'`
    ///   ⇒ 预览是字幕那支、成片是 Sans，**两边不一致且不报错**。
    ///
    /// 三处实现现已在空值上统一退到 [`default_font()`]：`engine.js`、`templates/index.html`、
    /// `tools/polish-to-dhampir.mjs`。**别再退回"跟随字幕"。**
    ///
    /// ⚠️ 它同时决定**文字宽度**（滚动/渐隐时长按宽度算，见 `measureDanmakuWidths`）——
    /// 预览/模板/转译器三处都必须用它量宽，量宽与渲染不一致就是"弹幕没滚到边就消失"。
    #[serde(default)]
    pub font: String,

    #[serde(default = "default_dm_size")]
    pub font_size: u32,

    #[serde(default = "default_dm_color")]
    pub color: String,

    #[serde(default = "default_dm_stroke")]
    pub stroke: String,

    #[serde(default = "default_dm_highlight_color")]
    pub highlight_color: String,

    #[serde(default = "default_dm_speed")]
    pub speed: f64,

    #[serde(default = "default_dm_area")]
    pub area: String,

    #[serde(default = "default_dm_direction")]
    pub scroll_direction: String,

    /// 弹幕时间偏移（秒，负 = 提前出现）。默认 −3.0：录播姬的弹幕时间戳
    /// 与视频成片之间普遍有一个系统性滞后，弹幕普遍比话说完晚几秒才出现，
    /// 整体提前 3 秒是精修时的常用起点。只影响弹幕的显示时刻（渲染/预览侧
    /// 平移 `start`/`end`），不参与分段与贴纸遮挡计算。
    #[serde(default = "default_dm_offset")]
    pub offset_secs: f64,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Default for DanmakuStyle {
    fn default() -> Self {
        Self {
            font: String::new(),
            font_size: default_dm_size(),
            color: default_dm_color(),
            stroke: default_dm_stroke(),
            highlight_color: default_dm_highlight_color(),
            speed: default_dm_speed(),
            area: default_dm_area(),
            scroll_direction: default_dm_direction(),
            offset_secs: default_dm_offset(),
            extra: HashMap::new(),
        }
    }
}

fn default_dm_size() -> u32 {
    42
}
fn default_dm_color() -> String {
    "#ffffff".into()
}
fn default_dm_stroke() -> String {
    "2px #000".into()
}
fn default_dm_highlight_color() -> String {
    "#ff6bcb".into()
}
fn default_dm_speed() -> f64 {
    8.0
}
fn default_dm_area() -> String {
    "danmaku_area".into()
}
fn default_dm_direction() -> String {
    "rtl".into()
}
fn default_dm_offset() -> f64 {
    -3.0
}

// ===== 布局 =====

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    #[serde(default = "default_liver")]
    pub liver: LayoutRegion,

    #[serde(default = "default_danmaku_area")]
    pub danmaku_area: LayoutRegion,

    #[serde(default = "default_sticker_area")]
    pub sticker_area: LayoutRegion,

    #[serde(default)]
    pub reaction_zone: Option<LayoutRegion>,

    #[serde(default)]
    pub game_area: Option<LayoutRegion>,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            liver: default_liver(),
            danmaku_area: default_danmaku_area(),
            sticker_area: default_sticker_area(),
            reaction_zone: None,
            game_area: None,
            extra: HashMap::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Point {
    /// x 百分比字符串，如 "65%"
    pub x: String,
    /// y 百分比字符串，如 "25%"
    pub y: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct LayoutRegion {
    /// 九宫格格子列表（可多个合并成更大区域）；旧版配置可能没有该字段，默认空数组
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grid: Vec<String>,
    /// 运镜聚焦中心（新）：九宫格格子名，如 "mid-right" / "center"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_grid: Option<String>,
    /// 运镜聚焦中心 (旧，格式 "x% y%")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// 旧版布局区域左上角/右下角（新字段为 grid，旧字段保留兼容）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_left: Option<Point>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom_right: Option<Point>,

    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl LayoutRegion {
    /// 是否为空（既没有九宫格格子，也没有自由选框）
    pub fn is_empty(&self) -> bool {
        self.grid.is_empty() && free_rect(self).is_none()
    }
}

fn default_liver() -> LayoutRegion {
    LayoutRegion {
        grid: vec!["mid-right".into()],
        origin_grid: Some("mid-right".into()),
        origin: None,
        top_left: None,
        bottom_right: None,
        extra: HashMap::new(),
    }
}

fn default_danmaku_area() -> LayoutRegion {
    LayoutRegion {
        grid: vec!["top-left".into()],
        origin_grid: None,
        origin: None,
        top_left: None,
        bottom_right: None,
        extra: HashMap::new(),
    }
}

/// 默认表情/插画区域: 优先放在 liver 左侧，覆盖中左 + 下左
fn default_sticker_area() -> LayoutRegion {
    LayoutRegion {
        grid: vec!["mid-left".into(), "bot-left".into()],
        origin_grid: None,
        origin: None,
        top_left: None,
        bottom_right: None,
        extra: HashMap::new(),
    }
}

// ===== 九宫格 =====

/// 返回九宫格格子的标准矩形（百分比字符串）。
pub fn grid_rect(name: &str) -> Option<(Point, Point)> {
    let p = |x: f64, y: f64| Point {
        x: format!("{x:.2}%"),
        y: format!("{y:.2}%"),
    };
    let rect = match name {
        "top-left" => (p(0.0, 0.0), p(33.33, 33.33)),
        "top" => (p(33.33, 0.0), p(66.67, 33.33)),
        "top-right" => (p(66.67, 0.0), p(100.0, 33.33)),
        "mid-left" => (p(0.0, 33.33), p(33.33, 66.67)),
        "center" => (p(33.33, 33.33), p(66.67, 66.67)),
        "mid-right" => (p(66.67, 33.33), p(100.0, 66.67)),
        "bot-left" => (p(0.0, 66.67), p(33.33, 100.0)),
        "bottom" => (p(33.33, 66.67), p(66.67, 100.0)),
        "bot-right" => (p(66.67, 66.67), p(100.0, 100.0)),
        _ => return None,
    };
    Some(rect)
}

/// 解析百分比字符串为数值。
pub fn parse_pct(s: &str) -> f64 {
    s.trim_end_matches('%').trim().parse::<f64>().unwrap_or(0.0)
}

/// 自由选框（新）：「框选布局」得到的精确矩形，落在 `top_left` / `bottom_right` 两个字段上。
///
/// 两个角都存在且面积有效时返回规范化的 (左上, 右下)：
/// 顺序化（拖拽可能左上右下颠倒）、百分比裁剪到 0~100、过小/退化返回 None。
/// 该矩形**优先于** `grid` —— 框选是更精确的表达，`grid` 保留作为清除选框后的回退。
pub fn free_rect(region: &LayoutRegion) -> Option<(Point, Point)> {
    let tl = region.top_left.as_ref()?;
    let br = region.bottom_right.as_ref()?;
    let x0 = parse_pct(&tl.x).clamp(0.0, 100.0);
    let y0 = parse_pct(&tl.y).clamp(0.0, 100.0);
    let x1 = parse_pct(&br.x).clamp(0.0, 100.0);
    let y1 = parse_pct(&br.y).clamp(0.0, 100.0);
    let (x0, x1) = (x0.min(x1), x0.max(x1));
    let (y0, y1) = (y0.min(y1), y0.max(y1));
    if x1 - x0 < 0.5 || y1 - y0 < 0.5 {
        return None;
    }
    let p = |x: f64, y: f64| Point {
        x: format!("{x:.2}%"),
        y: format!("{y:.2}%"),
    };
    Some((p(x0, y0), p(x1, y1)))
}

/// 将 LayoutRegion 解析为矩形列表（不合并包围盒）。
/// 优先级：自由选框（单矩形） > 九宫格格子（各自独立矩形）。
pub fn region_rects(region: &LayoutRegion) -> Vec<(Point, Point)> {
    if let Some(rect) = free_rect(region) {
        return vec![rect];
    }
    let mut out = Vec::new();
    for name in &region.grid {
        if let Some((tl, br)) = grid_rect(name) {
            out.push((tl, br));
        }
    }
    out
}

/// 解析运镜聚焦中心（百分比字符串 "x% y%"）。
/// 优先级：自由选框中心（框选出来的精确位置） > origin_grid（格子中心） >
/// origin（旧百分比） > grid 第一个格子中心。
pub fn region_origin(region: &LayoutRegion) -> Option<String> {
    if let Some((tl, br)) = free_rect(region) {
        let cx = (parse_pct(&tl.x) + parse_pct(&br.x)) / 2.0;
        let cy = (parse_pct(&tl.y) + parse_pct(&br.y)) / 2.0;
        return Some(format!("{cx:.2}% {cy:.2}%"));
    }
    if let Some(g) = &region.origin_grid {
        if let Some((tl, br)) = grid_rect(g) {
            let cx = (parse_pct(&tl.x) + parse_pct(&br.x)) / 2.0;
            let cy = (parse_pct(&tl.y) + parse_pct(&br.y)) / 2.0;
            return Some(format!("{cx:.2}% {cy:.2}%"));
        }
    }
    if let Some(o) = &region.origin {
        return Some(o.clone());
    }
    // 自动取第一个格子的中心
    if let Some(first) = region.grid.first() {
        if let Some((tl, br)) = grid_rect(first) {
            let cx = (parse_pct(&tl.x) + parse_pct(&br.x)) / 2.0;
            let cy = (parse_pct(&tl.y) + parse_pct(&br.y)) / 2.0;
            return Some(format!("{cx:.2}% {cy:.2}%"));
        }
    }
    None
}

// ===== 成片时间线（EDL）=====

/// 成片时间线的一段：`源区间` + `速率`。
///
/// **数组顺序 = 成片播放顺序**（区间可以重复、可以乱序、可以变速）—— 见
/// `docs/plan/edl-retiming-spec.md` §2.1。成片起点（`out_start`）**永远派生、绝不落盘**：
/// 落盘就有两个真源，改一段会让后面全部不同步（§2.2 规则 1）。
///
/// ⚠️ **不加 `deny_unknown_fields`**：这是**文件格式**（`polish.toml` 的 `[[edl.clips]]`），
/// 规格 §11.4 已经预告要加 `source_kind` / `asset_id` / `transition_in`。
/// 加了 deny 会让"新版本写过的工程"在旧版本上**直接读不出来**（而不是忽略那个键）。
///
/// ⚠️ 所有字段 `serde(default)`：老工程没有 `[[edl.clips]]`，一个字节都不该动（R4 存量零回归）。
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct EdlClip {
    /// 源时间轴起点（秒，与 `events` 的 `time` 同一基准）。
    #[serde(default)]
    pub source_start: f64,

    /// 源时间轴终点（秒）。必须 `> source_start`（校验在 `polish::project::validate`）。
    #[serde(default)]
    pub source_end: f64,

    /// 速率：`1.0` = 原速；`0.5` = 慢放；`2.0` = 快放。
    ///
    /// ⚠️ 夹在 `[SPEED_MIN, SPEED_MAX]` = `[0.5, 2.0]` —— 那**不是随便定的**：它是
    /// ffmpeg `atempo` 的**单级**范围（§3.2），超出去要串联多级 `atempo`。
    /// 常量表的真源在 `yekitrim-polish::project::validate`（`SPEED_MIN` / `SPEED_MAX`），
    /// 与 §4.1 第 8 条的"必须与 atempo 一致"是同一条。
    #[serde(default = "default_edl_speed")]
    pub speed: f64,

    /// 语义标签（可选，**仅供 UI/统计**，不参与任何推导）：
    /// `main` | `replay` | `recall` | `intro` | `outro`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,

    /// UI 显示名（可选，仅供 UI）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    /// 这一段**画面从哪来**（规格 §11.4）：`clip`（本条片段，缺省）| `asset`（素材库视频）。
    ///
    /// ⚠️ **`None` = `clip`**（不是"认不出的第三档"）：字段缺席是老工程，而老工程唯一可能的
    /// 来源就是本条片段。用 `Option` 而不是带默认值的枚举，是为了让"用户没写过这个键"
    /// 与"用户显式写了 clip"在**序列化**上也不可区分 —— 老工程重存一次不会多出一个键（R4）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<String>,

    /// 素材 id（**仅 `source_kind = "asset"` 时有意义**）。
    ///
    /// 指认的是**项目目录里那份拷贝**的名字（`<项目>/assets/videos/<文件名>`），
    /// 不是素材库里的绝对路径 —— 库改名/换机器之后老工程照样出得来（`assets/AGENTS.md`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,

    /// 进入这一段时的转场（规格 §11.4，**可选**）：`cut`（缺省）| `flash` | `dissolve`。
    ///
    /// ⚠️ 第 7 批起 **`dissolve` 已被消费**（不再是"只落盘"）：
    /// - **画面**（转译器）：把这一段挪到上面那条视频轨（`v2`），几何 = 本层 `start` 提前
    ///   `DISSOLVE_OVERLAP_SEC`、上一层保持不透明，本层挂 `opacity` 0→1 线性关键帧
    ///   （关键帧是**图层相对**帧号）。重叠跨轨才合法（同轨重叠是 `layer_overlap`）。
    /// - **音轨**（`yekitrim-polish::render::ffmpeg`）：对重叠窗口做出段淡出 + 入段淡入
    ///   （⚠️ 底座**不做**淡化，它把重叠音轨**直接相加**；见 `assemble_audio_from_edl_fades`）。
    /// - 成立条件与重叠帧数由 [`EdlClip::dissolve_overlap_frames`] 定，
    ///   派生结果读 [`ResolvedClip::dissolve_frames`]。
    ///
    /// ⚠️ **`flash` 仍未被消费**（认不出的档位 = 硬切，`transition_kind` 返回 `None`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_in: Option<String>,
}

fn default_edl_speed() -> f64 {
    1.0
}

/// `EdlClip::source_kind` 的取值 —— **`clip` 是缺省**（字段缺席 = 本条片段）。
pub const EDL_SOURCE_CLIP: &str = "clip";
/// `EdlClip::source_kind` 的取值 —— 素材库视频。
pub const EDL_SOURCE_ASSET: &str = "asset";

/// `EdlClip::transition_in` 的取值 —— **`cut` 是缺省**（字段缺席 = 硬切）。
pub const EDL_TRANSITION_CUT: &str = "cut";
/// `EdlClip::transition_in` 的取值 —— 叠化（跨轨重叠 + 上层 `opacity` 0→1 关键帧）。
pub const EDL_TRANSITION_DISSOLVE: &str = "dissolve";

/// `EdlClip::kind` 的取值 —— **片头**（拼在成片最前面的素材段）。
///
/// ⚠️ 与 `process::fx` 的 `EDL_KIND_COLD_OPEN` / `EDL_KIND_RECALL` 同一个命名空间，
/// 但这两个由**用户设置**直接决定（`[intro_outro]`），不经 LLM 候选。
pub const EDL_KIND_INTRO: &str = "intro";
/// `EdlClip::kind` 的取值 —— **片尾**（拼在成片最后面的素材段）。
pub const EDL_KIND_OUTRO: &str = "outro";

/// **叠化的重叠时长（秒）** —— 相邻两段在成片时间轴上重叠这么久。
///
/// ⚠️ **必须与转译器 `polish-to-dhampir.mjs::DISSOLVE_OVERLAP_SEC` 逐值相同**
/// （那里是零依赖的 node 脚本，只能照抄一份）。两边分歧的症状是
/// "**画面叠化 0.5s、声音只淡了 0.25s**"—— 音画不同步这一类**不报错**。
///
/// 取 `0.5` 的理由（探针 `.workbuddy/edl-transition-probe/` 就是用它测的，
/// 规格 §11.3 说"典型 0.3~1.0s"）：0.3s 在 60fps 下只有 18 帧，窗口太短时"中点两层同时可见"
/// 这条判据与硬切的区分度下降；1.0s 又长到"两段各让出半秒"肉眼可见。
pub const DISSOLVE_OVERLAP_SEC: f64 = 0.5;

impl EdlClip {
    /// 这一段的**入场转场**（`None` = 硬切）。
    ///
    /// ⚠️ **判据只此一份**（Rust 侧）：转译器 `edlClipTransition`、前端
    /// `edl.js::edlTransition` 是另两处消费点，必须同口径。
    ///
    /// 口径：`transition_in` 必须是**小写** `"dissolve"` —— `"Dissolve"` / `"DISSOLVE"`
    /// 不认（工程文件是机器写的，宽容只会让"拼错"从"看得出来"变成"看不出来"）；
    /// 其余一切（字段缺席 / `"cut"` / 认不出的档位）⇒ `None` = 硬切。
    pub fn transition_kind(&self) -> Option<&'static str> {
        match self.transition_in.as_deref() {
            Some(EDL_TRANSITION_DISSOLVE) => Some(EDL_TRANSITION_DISSOLVE),
            _ => None,
        }
    }

    /// 这一段**实际发生**的叠化重叠帧数（`0` = 没有叠化）。
    ///
    /// 三条成立条件（与转译器 `mainGeom` 那一段逐条同构）：
    /// ① 自己声明了 `dissolve`；
    /// ② `prev_frames`（**前一段在成片里占的帧数**）与自己的帧数都 `> overlap` ——
    ///    重叠 ≥ 段长会把那一段整段吃掉（那不是叠化，是删段）；
    /// ③ `overlap >= 1`（`fps` 探不出来时是 0 ⇒ 退化成硬切）。
    ///
    /// ⚠️ 调用方拿 `frames` = **这一段自己的**成片帧数（`edl_clip_frames`）。
    /// 第一段没有"前一段"可叠，由调用方传 `prev_frames = 0` 表达。
    pub fn dissolve_overlap_frames(&self, frames: u64, prev_frames: u64, fps: f64) -> u64 {
        if self.transition_kind().is_none() {
            return 0;
        }
        if !(fps > 0.0) || !fps.is_finite() {
            return 0;
        }
        let overlap = (DISSOLVE_OVERLAP_SEC * fps).round() as u64;
        if overlap < 1 || overlap >= frames || overlap >= prev_frames {
            return 0;
        }
        overlap
    }
}

impl EdlClip {
    /// 这一段是不是**素材库来源**（`source_kind = "asset"`）。
    ///
    /// ⚠️ **判据只此一份**：转译器（`polish-to-dhampir.mjs`）、Rust 音轨组装、前端成片轨
    /// （`polish/edl.js::edlSourceKind`）三处必须同口径 —— 分歧的症状是"预览里那段是素材、
    /// 成片里是本条"（或反过来），而**不报错**。前端那处是 JS，只能照抄，靠本函数的
    /// 文档注释 + 守卫对齐。
    ///
    /// 认不出的值（拼错 / 未来档位）按**本条片段**处理，与"字段缺席"同一条路 ——
    /// 素材那一档要额外要求 `asset_id` 存在才有意义（见 [`Self::asset_id_ref`]）。
    pub fn is_asset(&self) -> bool {
        self.source_kind.as_deref() == Some(EDL_SOURCE_ASSET)
    }

    /// 这一段指向的**素材 id**（`source_kind = "asset"` 且 `asset_id` 非空时才有值）。
    ///
    /// ⚠️ 空串 / 全空白**不算 id**：`asset_id = ""` 会静默退化成"素材库里没有这个文件"，
    /// 而那种失败在出片里表现为"这一段用回本条片段" —— 用户看不出区别。所以这里
    /// 把"没有 id"当成**没填**，由调用侧报出来（转译器侧是一条 DROP）。
    pub fn asset_id_ref(&self) -> Option<&str> {
        if !self.is_asset() {
            return None;
        }
        self.asset_id.as_deref().map(str::trim).filter(|s| !s.is_empty())
    }
}

/// 成片时间线：`clips[]`（`polish.toml` 的 `[[edl.clips]]`）。
///
/// ⚠️ **`clips` 为空 = 老工程 = 今天的行为**：由 `mute_ranges` 的补集派生（§2.2 规则 3）。
/// **不是"EDL 为空 ⇒ 什么都不剪"**，也不是"两者相加"—— 有 EDL 时 `mute_ranges` 被**忽略**，
/// 因为 `mute_ranges` 本来就是 EDL 的一个特例（只删、不排、不速）。相加会让
/// "这段到底播不播"产生歧义（§2.2 的 ⚠️）。
///
/// ⚠️ **`skip_serializing_if` 必须有**：空 EDL 不能被写进文件，否则老工程重存一次就多出一个
/// `[edl]` 段（R4 零回归的判据之一）。
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
pub struct Edl {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clips: Vec<EdlClip>,
}

/// `Edl::resolve_clips` 的派生结果：**成片时间轴**上的一段。
///
/// 全部字段都是派生值，**没有一个落盘**（§2.2 规则 1）。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedClip {
    /// 成片起点（秒）= `out_start_frame / fps`。
    pub out_start: f64,
    /// 成片终点（秒）= `out_end_frame / fps`。
    pub out_end: f64,
    /// 成片时长（秒）= `out_end - out_start`。
    pub out_duration: f64,
    /// 成片起点（**帧号**，整数累加的结果本身）。
    pub out_start_frame: u64,
    /// 成片终点（**帧号**）。最后一段的这个值就是 `total_frames`（§2.2 规则 2）。
    pub out_end_frame: u64,
    /// 源时间轴起点（秒，照抄该段的 `source_start`）。
    pub source_start: f64,
    /// 源时间轴终点（秒，照抄该段的 `source_end`）。
    pub source_end: f64,
    /// 该段的速率（照抄该段的 `speed`）。
    pub speed: f64,
    /// 这一段的**入场转场**（第 7 批 / task-10）：`None` = 硬切，`Some("dissolve")` = 叠化。
    ///
    /// 照抄 `EdlClip::transition_in` 的**已归一化**值（认不出的档位在这里就是 `None`，
    /// 与 `EdlClip::transition_kind` 同口径）。
    ///
    /// ⚠️ **`out_start` 不因它而变**（规格 §2.2 规则 1 那条"落盘/派生分界"的延伸）：
    /// 叠化的几何是"画面那一层提前 `DISSOLVE_OVERLAP_SEC` 开演"，
    /// 而 `out_start` 是**成片时间轴**的唯一推导（三处消费点逐值同构，
    /// `check-edl-invariants.mjs` 钉着）。把重叠算进 `out_start` 会让整条下游
    /// （字幕/弹幕/音效重映射、总时长、合流断言）跟着漂 —— 那是第二份推导。
    /// 所以转场只**描述**这一段，改动落在消费侧（画面层提前 / 音轨交叉淡化）。
    pub transition_in: Option<String>,
    /// 这一段与**前一段**实际重叠的帧数（`0` = 没有叠化）。
    ///
    /// 由 [`EdlClip::dissolve_overlap_frames`] 在**本函数里唯一算一次** ——
    /// 消费侧（音轨交叉淡化 / 前端成片轨角标）**只读它，不许自己再推一遍**：
    /// 两份推导分歧的症状是"**画面叠 0.5s、声音只淡了 0.25s**"，而**不报错**。
    pub dissolve_frames: u64,
}

/// 源秒 → 源帧号。**与转译器的 `frameAt` 逐式相同**（`tools/polish-to-dhampir.mjs:1340`：
/// `Math.round(seconds * fps)`）—— 两边不同口径就等于"预览 ≠ 成片"，而那条缝**不报错**。
pub fn edl_frame_at(seconds: f64, fps: f64) -> u64 {
    if !(fps > 0.0) || !seconds.is_finite() {
        return 0;
    }
    (seconds * fps).round().max(0.0) as u64
}

impl Edl {
    /// 是否为空（没有一段）—— **写盘判据**：空 EDL 一个字节都不写（R4 存量零回归）。
    ///
    /// ⚠️ 只给 `clips` 加 `skip_serializing_if` **是不够的**：`Edl` 本身是个表，
    /// toml 照样会写出一个空的 `[edl]` 段头 ⇒ 老工程重存一次就"多了一段"，
    /// 那正是"老工程产物逐字节不变"要挡的东西。所以 `ProjectFile::edl` 那个字段上
    /// 也要挂 `skip_serializing_if = "Edl::is_empty"`。两条缺一不可。
    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }

    /// **成片时间线的唯一推导**（规格 §4.1 第 3 条）。
    ///
    /// 只做"clips → 成片时间轴"这一段**纯数学**；"空 clips 时拿 `mute_ranges` 的补集兜底"
    /// 是 `ProjectFile` 那一层的事（`ProjectFile::resolve_edl`）—— `mute_ranges` 是
    /// `polish.toml` 的业务字段，本 crate 不认识它。
    ///
    /// # 为什么在**帧号域**累加（规格 §9 R8 累加漂移）
    ///
    /// `out_start` 是**累加**出来的：`Σ 前面每段的时长`。如果在**秒域**累加
    /// （`out_start += (source_end - source_start) / speed`），每段一次 f64 舍入会**推着后面
    /// 所有段一起偏** —— 段越多、变速越碎越明显。`cut_ranges` 时代没有累加，这是 **EDL 特有**的风险。
    ///
    /// 所以这里的口径是：**每段先量化成整数帧，`out_start_frame` 用整数累加，最后才除以 `fps`**
    /// （`out_start = out_start_frame / fps`）。三条好处：
    /// ① 累加本身**无舍入**（整数加法）；
    /// ② 与转译器 `buildKeepRanges` 的 `outStart`（同样是整数帧累加，`polish-to-dhampir.mjs:480-484`）
    ///    **逐值一致** —— §4.4 第 4 条那条 `check-edl-invariants.mjs` 对拍的前提；
    /// ③ `out_end[i] == out_start[i+1]` **精确成立**（同在帧格上），段间不会有浮点缝。
    ///
    /// # 参数
    ///
    /// - `fps`：源素材帧率（**必需**，帧号域的前提）。`<= 0` 或非有限值 ⇒ 返回空数组。
    /// - 段的顺序**就是** `self.clips` 的顺序（§2.1：数组顺序 = 成片播放顺序），这里**不排序**。
    /// - 非法段（`source_end <= source_start`、`speed` 非正或非有限）**跳过**，不 panic ——
    ///   拦人话在 `validate`，这里只保证"算得出来"。跳过的段不占帧号。
    pub fn resolve_clips(&self, fps: f64) -> Vec<ResolvedClip> {
        let mut out = Vec::with_capacity(self.clips.len());
        if !(fps > 0.0) || !fps.is_finite() {
            return out;
        }
        // ⚠️ 累加器是 **u64 帧号**，不是 f64 秒 —— 见上面 R8。
        let mut out_cursor: u64 = 0;
        // 前一段**在成片里占的帧数**（叠化的成立条件之一要用它，见
        // `EdlClip::dissolve_overlap_frames`）。非法段被 `continue` 跳过时**不更新** ——
        // 跳过的段不占帧号，也就不该成为"前一段"。
        let mut prev_frames: u64 = 0;
        for clip in &self.clips {
            let Some(frames) = edl_clip_frames(clip, fps) else {
                continue;
            };
            let start = out_cursor;
            let end = start + frames;
            out_cursor = end;
            // ⚠️ **叠化重叠在这里唯一算一次**（`dissolve_overlap_frames` 是三家
            // （转译器 / 音轨 / 前端）共同的成立条件）。消费侧只读 `dissolve_frames`。
            let dissolve_frames = clip.dissolve_overlap_frames(frames, prev_frames, fps);
            out.push(ResolvedClip {
                out_start: start as f64 / fps,
                out_end: end as f64 / fps,
                out_duration: frames as f64 / fps,
                out_start_frame: start,
                out_end_frame: end,
                source_start: clip.source_start,
                source_end: clip.source_end,
                speed: clip.speed,
                transition_in: clip.transition_kind().map(str::to_string),
                dissolve_frames,
            });
            prev_frames = frames;
        }
        out
    }

    /// 成片总帧数 = **最后一段的 `out_end_frame`**（§2.2 规则 2，与 `dhampir::sequence_frames`
    /// 读所有层 `end` 最大值同一口径）。空 clips ⇒ `0`。
    pub fn total_frames(&self, fps: f64) -> u64 {
        self.resolve_clips(fps).last().map(|c| c.out_end_frame).unwrap_or(0)
    }
}

/// 一段在成片里占**多少帧**：`源帧数 / speed`，四舍五入到整数帧。
///
/// 口径与转译器 §4.2 第 2 条"`outStart` 累加改成 `+ (end - start) / speed`"一致 ——
/// `end - start` 是**该段的源帧数**（不是秒数乘出来再除），这样两边的中间量落在同一格上：
/// 先 `round(秒 × fps)` 得到源帧边界，再除速率。反例：`round((e - s) * fps / speed)` 在
/// 非帧对齐的时刻上会比转译器差 1 帧，而那 1 帧会**顺延到后面所有段**。
///
/// `speed <= 0` / 非有限 / `source_end <= source_start` ⇒ `None`（该段作废）。
fn edl_clip_frames(clip: &EdlClip, fps: f64) -> Option<u64> {
    if !clip.speed.is_finite() || clip.speed <= 0.0 {
        return None;
    }
    if !clip.source_start.is_finite() || !clip.source_end.is_finite() {
        return None;
    }
    let sf = edl_frame_at(clip.source_start, fps);
    let ef = edl_frame_at(clip.source_end, fps);
    if ef <= sf {
        return None;
    }
    let frames = ((ef - sf) as f64 / clip.speed).round();
    if !(frames >= 1.0) {
        return None;
    }
    Some(frames as u64)
}

// ===== 文本输入（渲染核心的最小字幕/弹幕结构，与 yekitrim-core 解耦）=====

/// 字幕输入：与 data/subtitles.json 的条目结构一致。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Subtitle {
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub html: String,
}

/// 弹幕输入：与 data/danmaku.json 的条目结构一致。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Danmaku {
    pub text: String,
    pub start: f64,
    pub end: f64,
    pub y: f64,
    #[serde(default = "default_travel")]
    pub travel_duration: f64,
}

fn default_travel() -> f64 {
    8.0
}

#[cfg(test)]
mod edl_tests {
    use super::*;

    fn clip(s: f64, e: f64, speed: f64) -> EdlClip {
        EdlClip {
            source_start: s,
            source_end: e,
            speed,
            kind: None,
            label: None,
            source_kind: None,
            asset_id: None,
            transition_in: None,
        }
    }

    /// 规格 §9 **R8 累加漂移**的**单元级**判据：`out_start` 在帧号域累加 ⇒ 整数、无漂移。
    ///
    /// 30 段各恰好 1 帧（fps=30、原速）：帧号域 ⇒ 终点精确 `30` 帧、`out_end == 1.0`；
    /// 秒域累加（`(1.0/30.0)` 加 30 次）= `0.9999999999999999` ⇒ 这条会红。
    #[test]
    fn resolve_clips_accumulates_in_frame_domain() {
        let edl = Edl {
            clips: (0..30)
                .map(|i| clip(i as f64 / 30.0, (i + 1) as f64 / 30.0, 1.0))
                .collect(),
        };
        let r = edl.resolve_clips(30.0);
        assert_eq!(r.len(), 30);
        assert_eq!(r[29].out_end_frame, 30);
        assert_eq!(r[29].out_end, 1.0, "帧号域累加必须精确到 1.0");
        assert_eq!(r[0].out_start_frame, 0);
        // 段间**精确**相接（同在帧格上，不留浮点缝）
        for w in r.windows(2) {
            assert_eq!(w[0].out_end_frame, w[1].out_start_frame);
            assert_eq!(w[0].out_end, w[1].out_start);
        }
        assert_ne!((0..30).map(|_| 1.0 / 30.0).sum::<f64>(), 1.0, "秒域累加会漂（R8）");
    }

    /// `speed` 参与时长：`(源帧数 / speed)`，四舍五入到整数帧。
    #[test]
    fn resolve_clips_scales_by_speed() {
        let edl = Edl { clips: vec![clip(0.0, 3.0, 0.5), clip(10.0, 11.0, 2.0)] };
        let r = edl.resolve_clips(30.0);
        // (0,3) = 90 帧 ÷ 0.5 = 180；(10,11) = 30 帧 ÷ 2 = 15
        assert_eq!((r[0].out_start_frame, r[0].out_end_frame), (0, 180));
        assert_eq!((r[1].out_start_frame, r[1].out_end_frame), (180, 195));
        assert_eq!(r[0].speed, 0.5);
        assert_eq!(edl.total_frames(30.0), 195, "总帧数 = 最后一段的 out_end");
    }

    /// 退化输入不 panic：fps 非法 ⇒ 空；段非法 ⇒ 跳过并**不占帧号**。
    #[test]
    fn resolve_clips_tolerates_degenerate_input() {
        assert!(Edl::default().resolve_clips(30.0).is_empty());
        assert!(Edl::default().total_frames(30.0) == 0);
        assert!(Edl { clips: vec![clip(0.0, 1.0, 1.0)] }.resolve_clips(0.0).is_empty());
        assert!(Edl { clips: vec![clip(0.0, 1.0, 1.0)] }.resolve_clips(f64::NAN).is_empty());
        let edl = Edl {
            clips: vec![
                clip(5.0, 5.0, 1.0),   // 零长
                clip(9.0, 7.0, 1.0),   // 倒置
                clip(0.0, 2.0, 0.0),   // speed = 0
                clip(0.0, 2.0, f64::INFINITY),
                clip(0.0, 2.0, 1.0),   // 唯一合法
            ],
        };
        let r = edl.resolve_clips(30.0);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].out_start_frame, 0, "被跳过的段不许占帧号");
    }

    /// 帧号换算与转译器的 `frameAt`（`Math.round(sec * fps)`）同口径。
    #[test]
    fn frame_at_matches_translator_round() {
        assert_eq!(edl_frame_at(0.0, 30.0), 0);
        assert_eq!(edl_frame_at(1.0 / 30.0, 30.0), 1);
        assert_eq!(edl_frame_at(0.5, 30.0), 15);
        assert_eq!(edl_frame_at(-3.0, 30.0), 0, "负的夹到 0（校验另有告警）");
        assert_eq!(edl_frame_at(1.0, 0.0), 0, "fps 非法 ⇒ 0");
    }

    /// 序列化契约：空 EDL 写出来**什么都没有**（连 `[edl]` 段头都不许有 ——
    /// `ProjectFile::edl` 上那层 `skip_serializing_if` 靠的就是 `Edl::is_empty`）。
    #[test]
    fn empty_edl_serializes_to_nothing() {
        assert!(Edl::default().is_empty());
        assert_eq!(serde_json::to_value(Edl::default()).unwrap(), serde_json::json!({}));
        assert_eq!(
            serde_json::to_value(Edl { clips: vec![clip(1.0, 2.0, 1.0)] }).unwrap(),
            serde_json::json!({ "clips": [{ "source_start": 1.0, "source_end": 2.0, "speed": 1.0 }] }),
            "speed=1.0 会写出来（它是必填字段的缺省值，不是 Option）"
        );
        assert!(!Edl { clips: vec![clip(1.0, 2.0, 1.0)] }.is_empty());
    }
}

#[cfg(test)]
mod two_axis_tests {
    use super::*;
// 契约往返：老工程（只有 anim）反序列化后两个新字段是 None；
// 新工程两路写进去能读回来；两边都不写时不落盘（R4）。
//
// ⚠️ 2026-10-14 扩成**三轴**（+ `transition_anim`）并把 `layout_angle_deg`
//    一起纳入 —— R4 的判据就是"**没配的字段一个键都不许多写**"，
//    所以每加一个字段都必须**在这条测试里点名**；加了字段却漏改这条，
//    存量工程产物会多出键而**没有任何守卫会红**。
#[test]
fn element_axes_roundtrip() {
    // 老格式（只有 `anim`）—— 四个新键都必须缺席。
    let old = r#"{"type":"element","time":0.5,"kind":"text","text":"x","anim":"tilt_std"}"#;
    let e: Event = serde_json::from_str(old).unwrap();
    match &e {
        Event::Element { anim, style_anim, effect_anim, transition_anim, layout_angle_deg, .. } => {
            assert_eq!(anim.as_deref(), Some("tilt_std"));
            assert!(style_anim.is_none(), "老工程不该凭空多出 style_anim");
            assert!(effect_anim.is_none(), "老工程不该凭空多出 effect_anim");
            assert!(transition_anim.is_none(), "老工程不该凭空多出 transition_anim");
            assert!(layout_angle_deg.is_none(), "老工程不该凭空多出 layout_angle_deg");
        }
        _ => panic!("不是 Element"),
    }
    // 老工程序列化回去**不许**多出键（R4）
    let back = serde_json::to_string(&e).unwrap();
    for key in ["style_anim", "effect_anim", "transition_anim", "layout_angle_deg"] {
        assert!(!back.contains(key), "老工程序列化多出了 {key}：{back}");
    }
    // 新格式：三轴 + 角度全在
    let new = r#"{"type":"element","time":0.5,"kind":"text","text":"x",
        "style_anim":"tilt","effect_anim":"shake","transition_anim":"pop",
        "layout_angle_deg":-12.5}"#;
    let e2: Event = serde_json::from_str(new).unwrap();
    match &e2 {
        Event::Element { style_anim, effect_anim, transition_anim, layout_angle_deg, .. } => {
            assert_eq!(style_anim.as_deref(), Some("tilt"));
            assert_eq!(effect_anim.as_deref(), Some("shake"));
            assert_eq!(transition_anim.as_deref(), Some("pop"));
            assert_eq!(*layout_angle_deg, Some(-12.5));
        }
        _ => panic!("不是 Element"),
    }
    // ⚠️ **角度 0 必须与"没填"区分开**：`Option` 的 `Some(0.0)` 要真的落盘。
    //    写成 `if angle != 0` 那种"顺手优化"会让"用户明确设成 0 度"静默丢失，
    //    而症状只是"我调了 0 度，它却回到了档位默认值"。
    let zero = r#"{"type":"element","time":0.5,"kind":"text","text":"x","layout_angle_deg":0.0}"#;
    let e3: Event = serde_json::from_str(zero).unwrap();
    match &e3 {
        Event::Element { layout_angle_deg, .. } => {
            assert_eq!(*layout_angle_deg, Some(0.0), "角度 0 与\"没填\"是两回事");
        }
        _ => panic!("不是 Element"),
    }
    assert!(serde_json::to_string(&e3).unwrap().contains("layout_angle_deg"),
        "显式设成 0 度的角度必须落盘（它是用户的选择，不是缺省）");
}

}
