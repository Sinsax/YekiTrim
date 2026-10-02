/// 可选后端类型：`--renderer` / `VTEDIT_RENDERER` 的取值口径（**只此一份**）。
///
/// ⚠️ **曾经有 `Wgpu`**（`vtrim-render-core` 的 wgpu 合成管线 + `render/native/`）——
/// 整条已删除（用户拍板：GPU 出片交给独立的 Dhampir 底座）。
/// ⚠️ **曾经有 `Chrome`**（本仓 headless Chrome 逐帧 canvas，`render/chrome.rs` + `headless_chrome`
/// 依赖）—— 2026-10-01 整条删除（用户拍板：出片只有底座一条腿，不再要求用户装浏览器）。
/// ⚠️ **逐帧接缝也删了**（2026-10-02，B2 收口）：`FrameRenderer` trait + `create_backend` +
/// `render_chunk_sync` 整套"逐帧后端"机械已不存在 —— Dhampir 是**整段一起出**
/// （`render --from --to --out`），硬塞进逐帧接口就得每帧起一个进程。真要接第二个
/// 逐帧后端时从 git 历史把这套形状挖回来重新评估，别凭记忆重写。
///
/// 所以 `--renderer` / `VTEDIT_RENDERER` 现在**只认 `dhampir`**；老命令里的 `wgpu` / `native`
/// 仍视作 `dhampir`（它们本来就是"用 GPU 出片"的意思）；`chrome` 与拼错的 `chome` **认不出**
/// —— 调用方按"认不出就报错"处理（见 `pipeline.rs`），**别把 `chrome` 归到 `dhampir` 上**：
/// 那是静默换路，而两版成片的差别只有对着看才发现。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderBackendKind {
    /// **Dhampir 底座**（独立仓，走二进制整段出片；见 `render/dhampir.rs`）。
    Dhampir,
}

impl RenderBackendKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Dhampir => "dhampir",
        }
    }

    /// 认不出的值返回 `None`（含已删除的 `wgpu` / `native` / `chrome`）—— 由调用方决定怎么报。
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "dhampir" | "wgpu" | "native" => Some(Self::Dhampir),
            // ⚠️ `chrome` / 老命令里拼错的 `chome` **不再被认成任何东西**：那条腿已删除，
            // 认成 dhampir 就是"选了 Chrome 却在跑底座"的静默换路。报错文案由 `pipeline.rs` 给。
            _ => None,
        }
    }
}
