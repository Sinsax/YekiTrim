//! **Dhampir 后端** —— 只调二进制，不链库、不嵌代码。
//!
//! # 为什么是"调二进制"而不是"链进来"
//!
//! Dhampir 是**独立的 wgpu 底座**（`docs/` 里那份契约的持有者），它的版本节奏与
//! 本仓不同。**链库会把两个仓的编译绑在一起**，而调二进制只绑**命令行**：
//! 换一个 `dhampir.exe` 就是换一个渲染器，本仓一行不用重编。
//!
//! 这也正是本仓在删掉自己那条 GPU 链路时写下的方向
//! （`backend.rs`：**「GPU 出片交给独立的 Dhampir 底座」**）。
//!
//! # 三条用法（出片 / 短片段 / 单帧）
//!
//! | 用途 | 调用 | 产物 |
//! |---|---|---|
//! | 出片（画面） | [`render_video_only`] | `_temp_video.mp4`（**无音轨**，音轨归本仓）|
//! | WebUI 短片段（B1） | [`render_clip`] | 一段 mp4 |
//! | WebUI 单帧（B2） | [`render_frame_png`] | 一张 PNG |
//!
//! 三者共用同一套"转译 -> 调二进制"的骨架（[`prepare_project`]），差别只在
//! `render` 与 `frame` 两个子命令。
//!
//! ## 为什么出片那条要 `--no-audio`
//!
//! Dhampir 默认连音轨一起出，但**音轨这一侧的规矩归本仓**（音效混音、响度跟随、
//! `_temp_audio*.aac` 那一套）。两边都做就会有两份混音实现，而"两份实现迟早分叉"
//! 是本仓已经踩过的那类错。所以：**画面归 Dhampir，声音归本仓**。
//!
//! 预览那两条**不涉及音轨**（浏览器里静音播），所以也不带音轨。
//!
//! # 找不到东西就**报错**，不静默回退
//!
//! 二进制、转译器、字体三者任一找不到都**返回 Err**。静默回退到别的通道会让人
//! 以为"我在用 Dhampir 出片" —— 而那种错只有在对比两版成片时才看得出来。

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

/// 找 Dhampir 的二进制。
///
/// 顺序：`VTEDIT_DHAMPIR` -> 与本仓同层的 `Dhampir/target/{release,debug}` -> `PATH`。
///
/// **不猜版本目录**：候选只有那两个，找不到就报错，并把找过的地方列出来。
/// **产物的三条自检**：齐全 / CLI 可执行 / 契约版本可读。
///
/// # 为什么必须"明确报错"
///
/// 产物缺一件的时候，症状**不是**"找不到 Dhampir"，而是：
///
///   * 缺 `engine.js` → 页面白屏，控制台只有一句模块 404；
///   * 缺 `dhampir_wasm_bg.wasm` → 控制台 "启动失败：unreachable executed"；
///   * 缺 CLI → 出片时才发现，而那时用户已经等了几分钟。
///
/// 这些都是"看起来像渲染坏了、其实是部署坏了"的那一类 —— 这个项目一贯的口径是
/// **宁可当场报清楚**。所以这里把三件事一次说全，并**列出实际找过的路径**。
///
/// 关于契约版本：产物 `VERSION` 里的 `project_schema` 必须**存在且可读**；
/// "与本仓期望是否一致"的**权威判定在 Dhampir 那一侧**（`project.rs` 里
/// `doc.project_schema != PROJECT_SCHEMA_VERSION` 就拒绝），这里只报出读到的那一个 ——
/// 在 V-Trim 里再抄一份期望值，就等于给同一条规矩留了第二处实现。
fn verify_artifact_dir(preview: &Path) -> Result<()> {
    let engine = preview.join("engine.js");
    let wasm_js = preview.join("pkg").join("dhampir_wasm.js");
    let wasm_bg = preview.join("pkg").join("dhampir_wasm_bg.wasm");
    let mut missing: Vec<String> = Vec::new();
    for p in [&engine, &wasm_js, &wasm_bg] {
        if !p.is_file() {
            missing.push(p.display().to_string());
        }
    }
    if !missing.is_empty() {
        bail!(
            "Dhampir 预览产物不齐，缺：\n  {}\n\
             （在 Dhampir 仓跑 `node scripts/package.mjs`，把整个目录放到 <程序目录>/dhampir/）",
            missing.join("\n  ")
        );
    }
    let version = preview.parent().map(|d| d.join("VERSION"));
    match version.as_deref() {
        Some(v) if v.is_file() => {
            let text = std::fs::read_to_string(v).unwrap_or_default();
            let schema = text
                .lines()
                .find_map(|l| l.strip_prefix("project_schema="))
                .map(str::trim)
                .unwrap_or("<VERSION 里没有 project_schema= 这一行>");
            tracing_like_log(&format!("Dhampir 产物：{}（project_schema={}）", v.display(), schema));
        }
        _ => bail!(
            "Dhampir 产物缺 VERSION（{}）—— 它是产物与 V-Trim 之间的契约版本，不能没有。",
            version.map(|v| v.display().to_string()).unwrap_or_else(|| "<未知>".into())
        ),
    }
    Ok(())
}

/// 这一层不想引日志依赖（`vtrim-polish` 是库）—— 只在 `VTEDIT_DHAMPIR_VERBOSE` 时打印。
fn tracing_like_log(msg: &str) {
    if std::env::var("VTEDIT_DHAMPIR_VERBOSE").is_ok() {
        eprintln!("[dhampir] {msg}");
    }
}

/// 预览包目录：`VTEDIT_DHAMPIR_PREVIEW` 优先，其次 `<程序目录>/dhampir/preview`。
///
/// 这里**找不到不报错**：那说明当前是"指着源码树"的开发形态，交给 `wasm_dirs` 的
/// 老路去试；两条都不行时，那里的错误信息会把两边都列出来。
fn preview_dir() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("VTEDIT_DHAMPIR_PREVIEW") {
        let dir = PathBuf::from(p);
        if dir.is_dir() {
            return Some(dir);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.join("dhampir").join("preview");
    if dir.is_dir() {
        return Some(dir);
    }
    None
}

/// 出片用的 CLI：`VTEDIT_DHAMPIR_CLI` 优先，其次 `<程序目录>/dhampir/bin/<exe>`。
///
/// 两个都没有才走 [`find_binary`] 的老路（同层 `Dhampir/target/{release,debug}`）。
/// 这样"发布包"不需要任何环境变量，"开发"仍然可以指向源码树。
fn find_binary() -> Result<PathBuf> {
    let exe = if cfg!(windows) { "dhampir.exe" } else { "dhampir" };
    let mut tried: Vec<String> = Vec::new();

    // **定版产物优先**：`VTEDIT_DHAMPIR_CLI`（直接给一个文件），
    // 其次 `<程序目录>/dhampir/bin/<exe>`（发布包解压后的形状）。
    //
    // ⚠️ 两者的**失败含义不同**（2026-09-30 修）：
    //   * `VTEDIT_DHAMPIR_CLI` 是**显式指定** —— 指到不存在的文件就报错，不给"猜一个别的"；
    //   * `<程序目录>/dhampir/bin/` 只是**约定位置** —— 开发时它本来就不在，
    //     这时必须继续往下试（同层 `Dhampir/target/{release,debug}`、PATH）。
    // 早先两种混成一条 `bail!`，后果是**源码树下永远找不到底座**：
    // `current_exe()` 总能算出那个约定路径，于是它一 bail，后面的回退链一次都没跑到。
    if let Ok(explicit) = std::env::var("VTEDIT_DHAMPIR_CLI") {
        let p = PathBuf::from(&explicit);
        if p.is_file() {
            return Ok(p);
        }
        bail!("VTEDIT_DHAMPIR_CLI={explicit} 不是文件");
    }
    if let Some(p) = std::env::current_exe()
        .ok()
        .and_then(|c| c.parent().map(|d| d.join("dhampir").join("bin").join(exe)))
    {
        if p.is_file() {
            return Ok(p);
        }
        tried.push(format!("{}（发布包形状的约定位置）", p.display()));
    }

    if let Ok(explicit) = std::env::var("VTEDIT_DHAMPIR") {
        let p = PathBuf::from(&explicit);
        if p.is_file() {
            return Ok(p);
        }
        tried.push(format!("VTEDIT_DHAMPIR={explicit}（不是文件）"));
    }

    // 本仓 exe 的位置：`<V-Trim>/target/<profile>/vtrim[.exe]`
    if let Ok(current) = std::env::current_exe() {
        if let Some(dir) = current.parent() {
            for up in [2usize, 3] {
                let mut root = dir.to_path_buf();
                for _ in 0..up {
                    root = root.parent().map(Path::to_path_buf).unwrap_or(root);
                }
                for profile in ["release", "debug"] {
                    let cand = root.join("Dhampir").join("target").join(profile).join(exe);
                    if cand.is_file() {
                        return Ok(cand);
                    }
                    tried.push(cand.to_string_lossy().into_owned());
                }
            }
        }
    }

    // 本仓源码根（编译期常量）：`<V-Trim>/crates/vtrim-polish`
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(repo) = manifest.parent().and_then(Path::parent) {
        if let Some(parent) = repo.parent() {
            for profile in ["release", "debug"] {
                let cand = parent.join("Dhampir").join("target").join(profile).join(exe);
                if cand.is_file() {
                    return Ok(cand);
                }
                tried.push(cand.to_string_lossy().into_owned());
            }
        }
    }

    if which(exe).is_some() {
        return Ok(PathBuf::from(exe));
    }
    tried.push(format!("PATH 上的 {exe}"));

    bail!(
        "找不到 Dhampir 二进制（{exe}）。设 VTEDIT_DHAMPIR=<路径>，或把 Dhampir 与 V-Trim 放在同一层目录。\n  找过：\n    {}\n  \
         （出片只有这一条腿 —— 本仓那条 headless Chrome 兜底腿已于 2026-10-01 整条删除，别去找它）",
        tried.join("\n    ")
    )
}

/// 极简的 PATH 查找（只为上面那一句服务，不引 crate）。
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let cand = dir.join(name);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// 找转译器。它**住在本仓**（`tools/polish-to-dhampir.mjs`），发布包里也带着它（见 `xtask package`）。
///
/// # 顺序（2026-10-01 修：发布形态下"找不到转译器"）
///
/// 1. `VTEDIT_TRANSLATOR`（显式；指错 = **当场报错**，与 `find_binary` 同一口径）
/// 2. **`<程序目录>/tools/polish-to-dhampir.mjs`** —— 打包后的位置（`xtask package` 会把
///    `tools/` 与 `spec/effects.json` 一起放进去；转译器自己按**同层** `../spec/effects.json` 读词汇表）
/// 3. `<程序目录>/../../tools/…` —— 开发树（`target/debug/vtrim.exe` 往上两级就是仓库根）
/// 4. `CARGO_MANIFEST_DIR/../..` —— **构建期**路径，只当最后的保险
///
/// ⚠️ 第 4 条以前是唯一回退 ⇒ 装到别的机器上它指向一个**不存在的构建目录**，
/// 于是"默认出片通道"在发布形态下**根本起不来**（`find_binary`/`wasm_dirs` 早就有 `current_exe()`
/// 回退，只有它没有）。第 2 条**必须排在前面**：包内那一份才是与这个二进制配套的那一份。
fn find_translator() -> Result<PathBuf> {
    let mut tried: Vec<String> = Vec::new();
    if let Ok(explicit) = std::env::var("VTEDIT_TRANSLATOR") {
        let p = PathBuf::from(&explicit);
        if p.is_file() {
            return Ok(p);
        }
        bail!("VTEDIT_TRANSLATOR={explicit} 不是文件");
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("tools").join("polish-to-dhampir.mjs"));
            if let Some(up2) = dir.parent().and_then(Path::parent) {
                candidates.push(up2.join("tools").join("polish-to-dhampir.mjs"));
            }
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(repo) = manifest.parent().and_then(Path::parent) {
        candidates.push(repo.join("tools").join("polish-to-dhampir.mjs"));
    }
    for cand in candidates {
        if cand.is_file() {
            return Ok(cand);
        }
        tried.push(cand.to_string_lossy().into_owned());
    }
    bail!(
        "找不到转译器 `tools/polish-to-dhampir.mjs`。设 VTEDIT_TRANSLATOR=<路径>，\
         或确保发布包里带了 `tools/`（`cargo xtask package` 会带）。\n  找过：\n    {}",
        tried.join("\n    ")
    )
}

/// 字体族名归一化：只留 ASCII 字母数字并小写。
///
/// `"Noto Sans SC"` → `notosanssc`，`NotoSansSC-VF.ttf` → `notosansscvf` ——
/// **按"归一化后包含"匹配**，这正是底座 `resolve_family` 的比法（这里与它同口径）。
fn normalize_family(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// 族名 → 文件名里该出现的片段（**别名表**）。
///
/// 为什么需要它：Windows 上 `"Microsoft YaHei"` 的文件叫 **`msyh.ttc`**，名字里根本没有那串字；
/// `"SimHei"` → `simhei.ttf`（这条归一化后能直接匹配 ✓ 不用别名）。
fn family_aliases(family: &str) -> &'static [&'static str] {
    match normalize_family(family).as_str() {
        // 微软雅黑：常规 `msyh.ttc` / 粗体 `msyhbd.ttc` / 细体 `msyhl.ttc`
        "microsoftyahei" | "微软雅黑" => &["msyh"],
        "sansserif" => {
            // 通用族：按平台默认的**中文无衬线**落点（Windows = 微软雅黑；Linux = Noto Sans CJK）
            if cfg!(windows) {
                &["msyh"]
            } else {
                &["notosanscjk", "notosanssc", "dejavusans"]
            }
        }
        _ => &[],
    }
}

/// 系统字体目录（**只列这几个**：就绪位/出片都会调它，别去递归整个 `/usr/share/fonts`）。
fn system_font_dirs() -> Vec<PathBuf> {
    if cfg!(windows) {
        let mut dirs = vec![PathBuf::from("C:/Windows/Fonts")];
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            // Win10+ 用户装的字体落这儿（系统目录里没有）
            dirs.push(PathBuf::from(local).join("Microsoft").join("Windows").join("Fonts"));
        }
        dirs
    } else if cfg!(target_os = "macos") {
        vec![
            PathBuf::from("/System/Library/Fonts"),
            PathBuf::from("/Library/Fonts"),
        ]
    } else {
        vec![
            PathBuf::from("/usr/share/fonts"),
            PathBuf::from("/usr/local/share/fonts"),
        ]
    }
}

/// 把系统字体目录读成 `(归一化文件名, 路径)` 的清单（**读一次，供整条栈复用**）。
fn system_font_listing() -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for dir in system_font_dirs() {
        // Linux 的字体按子目录组织，浅一层足够（/usr/share/fonts/truetype/... ）
        let mut scan = vec![dir.clone()];
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    scan.push(p);
                }
            }
        }
        for d in scan {
            let Ok(entries) = std::fs::read_dir(&d) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|s| s.to_str()) else { continue };
                let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
                if !["ttf", "ttc", "otf"].contains(&ext.as_str()) {
                    continue;
                }
                out.push((normalize_family(name), path));
            }
        }
    }
    out
}

/// 从清单里挑一支：**先精确匹配**，再按别名/族名包含匹配，并且**优先非粗体/非斜体**。
///
/// 抽成纯函数是为了可测（"这一族在这份清单里会挑到哪个文件"的判据不该依赖本机装了什么字体）。
fn pick_font_from_listing(listing: &[(String, PathBuf)], family: &str) -> Option<PathBuf> {
    let want = normalize_family(family);
    let aliases = family_aliases(family);
    let matches: Vec<&(String, PathBuf)> = listing
        .iter()
        .filter(|(stem, _)| {
            (!want.is_empty() && stem.contains(&want))
                || aliases.iter().any(|a| stem.contains(&normalize_family(a)))
        })
        .collect();
    if matches.is_empty() {
        return None;
    }
    let exact = matches
        .iter()
        .find(|(stem, _)| want.is_empty() == false && stem.starts_with(&want) && stem.len() == want.len());
    if let Some((_, p)) = exact {
        return Some(p.clone());
    }
    let plain = |stem: &str| {
        !["bold", "bd", "italic", "oblique", "light", "thin", "black", "heavy"]
            .iter()
            .any(|m| stem.contains(m))
    };
    matches
        .iter()
        .find(|(stem, _)| plain(stem))
        .or_else(|| matches.first())
        .map(|(_, p)| p.clone())
}

/// **要按哪几支族名去找字体**（顺序就是"预览会用哪一支"的顺序）。
///
/// ⚠️ 这是"字体不对"那次的修点：预览那条腿按字体栈走，**底座这条腿以前完全不看** ——
/// 结果：本机没装栈首那支、装了栈里后面的 ⇒ 预览与成片字族不同（用户报的"字体不对"）。
/// 现在两侧**同一份栈**，用户选的那支排在最前。
///
/// ⚠️ **栈用动态版 [`server::fallback_stack()`]**（首项 = 库里默认那支的内部族名），**不是**
/// `SYSTEM_FONT_STACK` 常量 —— 常量只是空库回落态。用常量的话「设为默认」对成片永远无效
/// （预览变了、成片不变，正是这轮要收掉的第三处不统一）。
fn font_search_order(project_font: &str) -> Vec<String> {
    let mut order: Vec<String> = Vec::new();
    let mut push = |s: &str| {
        let cleaned = s.trim().trim_matches(['\'', '"']).trim().to_string();
        if !cleaned.is_empty() && !order.iter().any(|x| x.eq_ignore_ascii_case(&cleaned)) {
            order.push(cleaned);
        }
    };
    push(project_font);
    for family in crate::render::server::fallback_stack().split(',') {
        push(family);
    }
    order
}

/// 找字体文件（**常规 + 粗体**）。Dhampir **不猜系统字体**（它自己的规矩：显式给 `--font-file`），
/// 所以这一侧必须给出**存在的文件**。
///
/// # 顺序（2026-10-01 定稿：与**预览**那份字体栈逐支同序）
///
/// 1. `VTEDIT_DHAMPIR_FONT`（显式指定，最高优先）
/// 2. `font_search_order(style.subtitle.font)`：**项目选的那支排最前**，然后是
///    `render::server::fallback_stack()`（动态兜底栈，首项随库里默认走）—— 它必须与
///    **`PolishPreview.vue` 的 `.cap` 栈**同源（预览的 fallback 来自
///    `/api/polish/fonts` 的 `fallback_stack`，同一份计算）：栈的顺序决定
///    "项目选的那支没装时谁赢"，而用户在预览里看到的就是这个赢家。
///
/// 每一支都按 **① 字体库（`<程序目录>/resources/fonts`，id = 文件名去扩展名）
/// → ② 系统字体目录（族名归一化包含 + 别名表）** 的顺序解析；
/// 同一族里再配一支**粗体**（`bold`/`bd` 命名，缺了就给 `None`，由底座退回常规体）。
///
/// ⚠️ 教训（都踩过）：
///   * 只查字体库 ⇒ 库里没有就退 `msyh.ttc`，而**预览用的是栈里第一支装了的** ✗；
///   * **不给粗体文件** ⇒ 契约里 `font_weight: 700` 落不到实处，成片比预览**细一圈**
///     （底座不合成粗体：`font_weight >= 600` 时它只会去要 `--font-bold-file`）；
///   * 不区分粗细地挑文件 ⇒ `msyh` 会先匹配到 `msyhbd`（粗体）当常规用 ✗ —— 所以挑的时候
///     **优先非粗非斜**，粗体另走 [`pick_bold_from_listing`]。
fn find_font(project_font: &str) -> Result<(PathBuf, Option<PathBuf>)> {
    if let Ok(explicit) = std::env::var("VTEDIT_DHAMPIR_FONT") {
        let p = PathBuf::from(&explicit);
        if p.is_file() {
            return Ok((p, None));
        }
        bail!("VTEDIT_DHAMPIR_FONT={explicit} 不是文件");
    }
    let listing = system_font_listing();
    let mut tried: Vec<String> = Vec::new();
    for family in font_search_order(project_font) {
        if let Some(p) = vtrim_core::fonts::resolve_file(&family) {
            // 库内字体：粗体就在库里按 `<id> Bold` 找；找不到就交给底座（它会退回这一支）
            let bold = vtrim_core::fonts::resolve_file(&format!("{family} Bold")).filter(|b| *b != p);
            return Ok((p, bold));
        }
        if let Some(p) = pick_font_from_listing(&listing, &family) {
            let bold = pick_bold_from_listing(&listing, &family).filter(|b| *b != p);
            return Ok((p, bold));
        }
        tried.push(family);
    }
    bail!(
        "找不到字体文件（按预览那份字体栈找过：{}）。\n  \
         修法：设 VTEDIT_DHAMPIR_FONT=<ttf/ttc/otf 路径>，或往字体库（<程序目录>/resources/fonts/）放一支",
        tried.join(" / ")
    )
}

/// 同一族的**粗体**：文件名里带 `bold`/`bd` 的那一支（`msyh.ttc` → `msyhbd.ttc`）。
///
/// 找不到就返回 `None` —— 底座在 `font_weight >= 600` 时会退回常规体（**它不合成粗体**）。
/// 可变字体（`NotoSansSC-VF.ttf`）也属这一类：文件本身能变重，但 drawtext 只取默认实例。
fn pick_bold_from_listing(listing: &[(String, PathBuf)], family: &str) -> Option<PathBuf> {
    let want = normalize_family(family);
    let aliases = family_aliases(family);
    let mut cands: Vec<&(String, PathBuf)> = listing
        .iter()
        .filter(|(stem, _)| {
            let family_hit = (!want.is_empty() && stem.contains(&want))
                || aliases.iter().any(|a| stem.contains(&normalize_family(a)));
            family_hit && ["bold", "bd", "heavy", "black"].iter().any(|m| stem.contains(m))
        })
        .collect();
    // `bd`（雅黑粗体的文件名）优先于 `black`/`heavy` 这类更重的档
    cands.sort_by_key(|(stem, _)| if stem.contains("bd") { 0 } else { 1 });
    cands.first().map(|(_, p)| p.clone())
}

/// **wasm 包的落点**：`(网页目录, pkg 目录)`。
///
/// 浏览器端实时预览要两样东西，它们住在 Dhampir 仓里、**不入本仓 git**：
///
/// * `<Dhampir>/web`             —— `engine.js`（宿主引擎，页面 `import` 它）
/// * `<Dhampir>/crates/dhampir-wasm/www/pkg` —— `dhampir_wasm.js` + `.wasm`
///
/// `VTEDIT_DHAMPIR_WASM` 可以整体覆盖（指向 Dhampir 仓根）；否则按与二进制同一套
/// "同层目录"规则找 —— **与 `find_binary` 同源**，这样"换了 Dhampir 仓"只需要改一处。
pub fn wasm_dirs() -> Result<(PathBuf, PathBuf)> {
    // **定版产物优先**：`<程序目录>/dhampir/preview`（或 `VTEDIT_DHAMPIR_PREVIEW`）。
    //
    // 产物里 `engine.js` 与 `pkg/` 是**平铺**在同一个目录下的 —— 正好对上本函数的
    // `(web, pkg)` 两返回值，所以这里不需要任何路径知识。找不到才退回下面那条
    // "指着源码仓根"的老路（开发时用，见 `VTEDIT_DHAMPIR_WASM`）。
    if let Some(dir) = preview_dir() {
        // **找到了产物目录就按产物的标准校验**（不齐就报清楚，别退回源码树那条路 ——
        // 那会把"部署坏了"伪装成"开发形态"，用户看到的是白屏而不是这句话）。
        verify_artifact_dir(&dir)?;
        return Ok((dir.clone(), dir.join("pkg")));
    }
    let mut tried: Vec<String> = Vec::new();
    let mut roots: Vec<PathBuf> = Vec::new();

    if let Ok(explicit) = std::env::var("VTEDIT_DHAMPIR_WASM") {
        roots.push(PathBuf::from(explicit));
    }
    // 与 find_binary 同一套：本仓 exe 上溯，以及编译期源码根。
    if let Ok(current) = std::env::current_exe() {
        if let Some(dir) = current.parent() {
            for up in [2usize, 3] {
                let mut root = dir.to_path_buf();
                for _ in 0..up {
                    root = root.parent().map(Path::to_path_buf).unwrap_or(root);
                }
                roots.push(root.join("Dhampir"));
            }
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(repo) = manifest.parent().and_then(Path::parent) {
        if let Some(parent) = repo.parent() {
            roots.push(parent.join("Dhampir"));
        }
    }

    for root in &roots {
        let web = root.join("web");
        let pkg = root.join("crates").join("dhampir-wasm").join("www").join("pkg");
        if web.join("engine.js").is_file() && pkg.join("dhampir_wasm.js").is_file() {
            return Ok((web, pkg));
        }
        tried.push(format!("{}（web/engine.js 或 pkg/dhampir_wasm.js 不在）", root.display()));
    }
    bail!(
        "找不到 Dhampir 的 wasm 预览包。设 VTEDIT_DHAMPIR_WASM=<Dhampir 仓根>。\n\
         （浏览器端实时预览需要 <Dhampir>/web/engine.js 与 <Dhampir>/crates/dhampir-wasm/www/pkg —— \
         先在那里跑 wasm-pack build）\n  找过：\n    {}",
        tried.join("\n    ")
    )
}

/// 出片的产物路径（与当年那条 chrome 腿**同一个**中间文件 `_temp_video.mp4`，后面音轨链路原样接上）。
pub fn video_intermediate(project_path: &Path) -> PathBuf {
    project_path.join("_temp_video.mp4")
}

/// 转译报告的摘要（`DROP` / `CLAMP` 的条数与逐条文本）。
///
/// 报告是**接口产物**（见 `tools/AGENTS.md`），所以这里解析它的**固定形状**
/// （`---- DROP (N) ----` 之后每行 `  [where] what`）—— 解析失败就当作"没报告"，
/// **绝不让它把出片搞崩**（报告只影响 `--strict` 判断与日志）。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct TranslateReport {
    pub drop_count: usize,
    pub clamp_count: usize,
    pub drops: Vec<String>,
    pub clamps: Vec<String>,
}

impl TranslateReport {
    pub fn parse(stdout: &str) -> Self {
        let mut report = Self::default();
        let mut cur: Option<&str> = None;
        for line in stdout.lines() {
            let trimmed = line.trim_end();
            if let Some(rest) = trimmed.strip_prefix("---- ") {
                // `---- DROP (11) ----`
                cur = if rest.starts_with("DROP") {
                    Some("DROP")
                } else if rest.starts_with("CLAMP") {
                    Some("CLAMP")
                } else {
                    None
                };
                continue;
            }
            if trimmed.starts_with("===") || trimmed.starts_with("转译完成") {
                cur = None;
                continue;
            }
            let Some(level) = cur else { continue };
            let Some(open) = trimmed.find('[') else { continue };
            let Some(close) = trimmed[open..].find(']') else { continue };
            // 转译器那一行的形状是 `  [where] what`（两空格缩进）——照原样收进 `[where] what`，
            // 报错时能直接贴给人看。**别忘了 `]` 后面那半句**（第一版只拼了 where）。
            let where_part = &trimmed[open + 1..open + close];
            let what_part = trimmed[open + close + 1..].trim();
            let item = format!("[{where_part}] {what_part}");
            match level {
                "DROP" => {
                    report.drop_count += 1;
                    report.drops.push(item);
                }
                _ => {
                    report.clamp_count += 1;
                    report.clamps.push(item);
                }
            }
        }
        report
    }
}

/// 就绪位：**这一台机器能不能用底座出片**（`/api/polish/dhampir`、`vtrim doctor`、
/// 前端"通道不可用"提示共用同一份判断）。
///
/// ⚠️ **只做定位，不渲染、不跑转译器**（一次请求里不能顺带跑一次 node）：
/// 二进制 / 预览产物 / 转译器 / 字体 / node / 版本契约 六项各查一次文件。
/// 每一项失败都带一句**怎么办**（`problems`）—— 这就是"底座不可用要在渲染前可见"的落点。
#[derive(Debug, Clone, serde::Serialize)]
pub struct Readiness {
    pub ready: bool,
    pub binary: Option<String>,
    pub preview: Option<String>,
    pub translator: Option<String>,
    pub font: Option<String>,
    /// **同一族的粗体文件**（`msyh.ttc` → `msyhbd.ttc`）。
    ///
    /// 字幕是 `font_weight: 700`、弹幕 `600`，底座**不合成粗体**：它只会在 `>= 600` 时去要
    /// 这个文件，拿不到就退回常规体 ⇒ **笔画比预览细一圈**。所以"有没有粗体"必须能被看见
    /// （`None` 时 `problems` 里会有一条 ⚠️）。
    pub font_bold: Option<String>,
    /// 版本契约（发布包里 `VERSION` 的 `project_schema=`）——只有定版产物才有。
    pub contract_version: Option<String>,
    /// `node --version` 的原文（**只有深探才有**；便宜那一趟是 `None`）。
    pub node_version: Option<String>,
    /// **效果词汇表**（`spec/effects.json`）的路径。
    ///
    /// `None` **不是错误**：找不到包内那份时会退到**编译期内嵌**那一份（`include_str!`），
    /// 行为等价 —— 但"在哪"要可见，所以 doctor 会把两种情况分开印。
    pub spec: Option<String>,
    pub problems: Vec<String>,
}

/// `deep` = 顺带跑一次 `node --version`（转译器要 Node ≥ 18，而"PATH 里有个 node"不等于"它能跑"——
    /// 商店里的桩、装了一半的 Node 都属于这一类）。
pub fn readiness(project_font: &str, deep: bool) -> Readiness {
    let mut problems = Vec::new();
    let binary = match find_binary() {
        Ok(p) => Some(p.display().to_string()),
        Err(e) => {
            problems.push(format!("出片二进制：{e}"));
            None
        }
    };
    let translator = match find_translator() {
        Ok(p) => Some(p.display().to_string()),
        Err(e) => {
            problems.push(format!("转译器：{e}"));
            None
        }
    };
    // 字体：常规 + **粗体那一支**（少了它 700/600 的字重会退回常规体 —— 见 `font_bold` 的说明）。
    let (font, font_bold) = match find_font(project_font) {
        Ok((regular, bold)) => (Some(regular.display().to_string()), bold.map(|b| b.display().to_string())),
        Err(e) => {
            problems.push(format!("字体：{e}"));
            (None, None)
        }
    };
    // 转译器是 node 脚本：node 不在 PATH 时"二进制与字体都齐"也出不了片。
    // ⚠️ 局部变量别叫 `node_version` —— 那会**遮住**上面那个同名函数（`E0618`）。
    let mut node_version_label = None;
    if !node_on_path() {
        problems.push(
            format!(
                "转译器要 Node {MIN_NODE_MAJOR}+（`node --version` 能跑通），当前 PATH 上找不到 —— \
                 先装 Node（出片只有底座这一条腿，没有可退的第二条）"
            ),
        );
    } else if deep {
        match node_version() {
            Some((major, minor, raw)) => {
                if major < MIN_NODE_MAJOR {
                    problems.push(format!(
                        "Node 版本太低：{raw}（转译器要 ≥ v{MIN_NODE_MAJOR}）——升级 Node（出片只有底座这一条腿）"
                    ));
                }
                node_version_label = Some(format!("{raw}（v{major}.{minor}）"));
            }
            None => {
                // PATH 里有个 `node` 却跑不出 `--version`：多半是桩 / 装了一半。
                problems.push(
                    "PATH 上有 `node`，但 `node --version` 跑不通（商店桩或装了一半？）—— \
                     重装 Node（出片只有底座这一条腿）"
                        .to_string(),
                );
            }
        }
    }
    // 预览产物是可选的（只有页面预览要用），缺了不影响出片 —— 只记在字段里，不进 problems。
    let (preview, contract_version) = match wasm_dirs() {
        Ok((engine, _pkg)) => {
            let version = engine
                .parent()
                .and_then(|dir| std::fs::read_to_string(dir.join("VERSION")).ok())
                .and_then(|text| {
                    text.lines()
                        .find_map(|l| l.trim().strip_prefix("project_schema=").map(str::to_string))
                });
            (Some(engine.display().to_string()), version)
        }
        Err(_) => (None, None),
    };
    // 词汇表：包内那份的路径（找不到就 None ⇒ 走编译期内嵌，**不是错误**，所以不进 `problems`）。
    let spec = crate::render::pipeline::spec_effects_path().map(|p| p.display().to_string());
    Readiness {
        ready: problems.is_empty(),
        binary,
        preview,
        translator,
        font,
        font_bold,
        contract_version,
        node_version: node_version_label,
        spec,
        problems,
    }
}

/// `node` 在不在给定的 PATH 里 —— **不启动进程**（就绪位会在 `/api/health` 这类热路径上被调）。
fn node_in_path_var(paths: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let name = if cfg!(windows) { "node.exe" } else { "node" };
    std::env::split_paths(&paths?).map(|dir| dir.join(name)).find(|p| p.is_file())
}

fn node_on_path() -> bool {
    node_in_path_var(std::env::var_os("PATH")).is_some()
}

/// 转译器要求的最低 Node 大版本（`node --version` 的 `v18.x` ⇒ 18）。
///
/// 为什么要有这条：转译器用了 `Array.prototype.at` / 顶层 await / `??=` 这些语法，
/// **旧 Node 会直接语法错**（症状是"转译失败 + 一屏 SyntaxError"，而用户以为自己没装 Node）。
/// 就绪位把它变成**渲染前一句话**。
pub const MIN_NODE_MAJOR: u32 = 18;

/// 解析 `node --version` 的输出（`v20.11.1` ⇒ `(20, 11)`）。
///
/// 纯函数，**为了可测**：真实环境里没法安全地"假装 node 是 v16"（改 PATH 是进程级的）。
pub fn parse_node_version(stdout: &str) -> Option<(u32, u32)> {
    let text = stdout.trim();
    let rest = text.strip_prefix('v').unwrap_or(text);
    let mut parts = rest.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().unwrap_or(0);
    Some((major, minor))
}

/// `node --version` 跑一次（**只在"深探"里调**：`/api/polish/dhampir-status` 与 `polish doctor`；
/// `/api/health` 是热路径 + 无鉴权，只走"PATH 里有没有"那条便宜判断）。
fn node_version() -> Option<(u32, u32, String)> {
    let out = Command::new("node").arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let parsed = parse_node_version(&raw)?;
    Some((parsed.0, parsed.1, raw))
}

/// **转译一次**：`polish.toml` -> Dhampir 工程文件。返回工程文件路径。
///
/// 出片、短片段、单帧三条路都从这里起 —— 转译只写一份，就不存在"三条路各转一遍、
/// 哪天有一路忘了改"的问题。
///
/// 旁写文件（`dhampir-danmaku.ass` / `dhampir-subtitle.srt`）会落在工程文件旁边，
/// 由 [`cleanup_project`] 收拾。
pub fn prepare_project(clip_dir: &Path, project_path: &Path, keep_muted: bool) -> Result<PathBuf> {
    Ok(prepare_project_reporting(clip_dir, project_path, keep_muted)?.0)
}

/// 与 [`prepare_project`] 同一条路，但**把转译报告也带回来**（`--strict` 要用它）。
pub fn prepare_project_reporting(
    clip_dir: &Path,
    project_path: &Path,
    keep_muted: bool,
) -> Result<(PathBuf, TranslateReport)> {
    let translator = find_translator()?;
    let translated = project_path.join("_dhampir_project.json");
    let t = std::time::Instant::now();
    let mut cmd = Command::new("node");
    cmd.arg(&translator).arg(clip_dir).arg("--out").arg(&translated);
    if keep_muted {
        // **出片**那条腿：时间线保持整条，屏蔽区间交给尾部那一剪（见转译器里的说明）。
        // 预览**不能**加这个 —— 客户端要靠压缩时间线跳屏蔽区间。
        cmd.arg("--keep-muted");
    }
    let out = cmd.output().with_context(|| {
        format!(
            "跑转译器失败：{}（需要一个能用的 node）",
            translator.display()
        )
    })?;
    if !out.status.success() {
        bail!(
            "转译失败（exit={:?}）：{}\n{}",
            out.status.code(),
            translator.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let report = TranslateReport::parse(&stdout);
    if report.drop_count > 0 || report.clamp_count > 0 {
        // 摘要进日志：`--strict` 的报错、以及"报告变了"这件事都要在出片日志里看得见。
        tracing::info!(
            "  · 转译报告：丢失 {} / 降级 {}（`polish render --strict` 会在丢失 > 0 时拒绝出片）",
            report.drop_count,
            report.clamp_count
        );
    }
    tracing::info!(
        "  v 转译 -> {}  ⏱ {:.1}s",
        translated.display(),
        t.elapsed().as_secs_f64()
    );
    Ok((translated, report))
}

/// 收拾转译留下的草稿（**只删自己造的**：`_temp_video.mp4` 归主流程）。
pub fn cleanup_project(project_path: &Path) {
    let translated = project_path.join("_dhampir_project.json");
    for scratch in [
        translated.clone(),
        translated.with_file_name("dhampir-danmaku.ass"),
        translated.with_file_name("dhampir-subtitle.srt"),
    ] {
        std::fs::remove_file(&scratch).ok();
    }
}

/// 读底座那一行 NDJSON 的 `progress`：`{"event":"progress","done":N,"total":M}`。
///
/// **不解析就丢**（返回 `None`）—— 底座换文案不该让出片失败；它只影响进度显示。
fn parse_progress_line(line: &str) -> Option<(u64, u64)> {
    if !line.contains("progress") {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if v.get("event")?.as_str()? != "progress" {
        return None;
    }
    Some((v.get("done")?.as_u64()?, v.get("total")?.as_u64()?))
}

/// 底座那一行是不是终态（`{"event":"done",...}`）。
///
/// ⚠️ **不能只看 `"done"` 这个子串**：`progress` 行里也有 `"done":N` 字段，
/// 拿子串判会把每一条进度都当成终态、读循环第一次就退出（症状是"进度只出现在最后"）。
fn is_done_line(line: &str) -> bool {
    if !line.contains("done") {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(line.trim())
        .ok()
        .and_then(|v| v.get("event").and_then(|e| e.as_str()).map(|e| e == "done"))
        .unwrap_or(false)
}

/// 从 `offset` 起读**追加写**的文件，切出完整行；半行留在 `buf` 里等下一次。
///
/// 用字节缓冲而不是 `read_to_string`：追加写的一刻可能正好切在一个多字节字符中间
/// （底座最后那行 `done` 里带产物路径，路径可能是中文），`read_to_string` 会整段报错、白丢一批行。
fn read_new_lines(path: &Path, offset: &mut u64, buf: &mut Vec<u8>) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut out = Vec::new();
    let Ok(mut f) = std::fs::File::open(path) else {
        return out;
    };
    if f.seek(SeekFrom::Start(*offset)).is_err() {
        return out;
    }
    let mut chunk = Vec::new();
    if f.read_to_end(&mut chunk).is_err() || chunk.is_empty() {
        return out;
    }
    *offset += chunk.len() as u64;
    buf.extend_from_slice(&chunk);
    while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
        let line: Vec<u8> = buf.drain(..=pos).collect();
        let text = String::from_utf8_lossy(&line).trim().to_string();
        if !text.is_empty() {
            out.push(text);
        }
    }
    out
}

/// 边等边把底座的 NDJSON 翻成进度行的那点状态。
///
/// 做成一个结构体而不是闭包：闭包会把 `seen_done` / `last_line` 借到循环之后，
/// 而那两样**恰恰是失败时要报出来的**（`E0505`）。
struct ProgressSink {
    seen_done: bool,
    last_line: String,
    last_emit: std::time::Instant,
    t0: std::time::Instant,
}

impl ProgressSink {
    fn new(t0: std::time::Instant) -> Self {
        Self {
            seen_done: false,
            last_line: String::new(),
            // 起手就允许打第一条（`elapsed` 一开始必然小于 1.5s）
            last_emit: t0 - std::time::Duration::from_secs(10),
            t0,
        }
    }

    fn on_line(&mut self, line: String) {
        if is_done_line(&line) {
            self.seen_done = true;
        }
        self.last_line = line.clone();
        let Some((done, total)) = parse_progress_line(&line) else {
            return;
        };
        let pct = if total == 0 { 100 } else { (done * 100 / total).min(100) };
        if done >= total || self.last_emit.elapsed().as_millis() >= 1500 {
            tracing::info!(
                "    frames 0..{}: {}%  {:.0}s",
                total.saturating_sub(1),
                pct,
                self.t0.elapsed().as_secs_f64()
            );
            self.last_emit = std::time::Instant::now();
        }
    }
}

/// **整段有多少帧** —— 从**转译后的工程**里读，不用上层那个估算值。
///
/// ⚠️ 这条是实测踩出来的（2026-09-30）：转译器写的时间基是**素材自己的**
/// （demo 那份是 `60/1`），而 `cmd_render` 里 `fps` 是写死的 30 —— 拿 `duration × 30`
/// 当 `--to`，12 秒的片子会**只出 360 帧 = 6 秒**，而音轨照旧 12 秒，合流之后更难看。
/// 权威在工程里：所有层 `end` 的最大值就是序列长度（工程的 `[0, end)` 半开区间，
/// 所以最后一帧是 `end - 1`）。
///
/// `pub` 是给集成测试用的：它要拿同一个数去核对"出来的帧数对不对"（差一帧是这类缝最爱出的错）。
pub fn sequence_frames(translated: &Path) -> Result<usize> {
    let raw = std::fs::read_to_string(translated)
        .with_context(|| format!("读转译产物失败：{}", translated.display()))?;
    let doc: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("转译产物不是合法 JSON：{}", translated.display()))?;
    let mut max_end: u64 = 0;
    if let Some(tracks) = doc.get("timeline").and_then(|t| t.get("tracks")).and_then(|t| t.as_array()) {
        for track in tracks {
            for layer in track.get("layers").and_then(|l| l.as_array()).into_iter().flatten() {
                if layer.get("enabled").and_then(|e| e.as_bool()) == Some(false) {
                    continue;
                }
                if let Some(end) = layer.get("end").and_then(|e| e.as_u64()) {
                    max_end = max_end.max(end);
                }
            }
        }
    }
    if max_end == 0 {
        bail!(
            "转译产物里没有任何层的 `end` —— 算不出要出多少帧（{}）",
            translated.display()
        );
    }
    Ok(max_end as usize)
}

/// 调一次 `dhampir render`。`from`/`to` 都是**闭区间**的帧号。
///
/// # 为什么把底座的 stdout **落文件**、再边等边 tail
///
/// 底座的进度是 NDJSON（`start` / `progress` / `done`）。**不翻译的话外面那层收不到任何进度**：
/// `vtrim server` 的 `follow_render_output` 认的是当年那条 chrome 腿留下的 `frames A..B: NN%` 形状 ——
/// 分钟级的出片在 WebUI 上就是"卡在 0%"。契约留在本仓（不让底座为 V-Trim 定制输出）。
///
/// 但**不能用管道**收（本仓踩过两次：Chrome/ffmpeg 是孙进程，它们继承 stdout 句柄，
/// 只要还有一个活着管道就永不 EOF；`api/AGENTS.md` 那条纪律同样适用于这里，只是深了一层）。
/// 所以照外层的做法：`stdout` 重定向进 `_temp_dhampir_progress.ndjson`，
/// `try_wait()` 轮询 + 按 offset tail 它 —— 进程退出判定与进度显示互不依赖。
/// 文件名带 `_temp_` 前缀：失败时 `cleanup_render_leftovers` 会把它一起收掉。
///
/// stderr 直接**继承**：底座自己的 warn/诊断原样落进任务日志（父进程读的就是同一份文件）。
fn run_render(
    translated: &Path,
    clip_dir: &Path,
    from: usize,
    to: usize,
    out: &Path,
    with_audio: bool,
    project_font: &str,
) -> Result<()> {
    let binary = find_binary()?;
    let (font, font_bold) = find_font(project_font)?;
    let t = std::time::Instant::now();
    let progress_path = out.with_file_name("_temp_dhampir_progress.ndjson");
    let progress_file = std::fs::File::create(&progress_path)
        .with_context(|| format!("创建进度文件失败：{}", progress_path.display()))?;
    let mut cmd = Command::new(&binary);
    cmd.arg("render")
        .arg("--project")
        .arg(translated)
        .arg("--from")
        .arg(from.to_string())
        .arg("--to")
        .arg(to.to_string())
        .arg("--out")
        .arg(out)
        .arg("--asset-root")
        .arg(clip_dir)
        .arg("--font-file")
        .arg(&font)
        // ⚠️ **粗体必须显式给**（2026-10-01 修："字体不对"的真凶）：
        // 转译器给字幕写了 `font_weight: 700`、弹幕 `600`（与**预览**的 `.cap{font-weight:700}` /
        // `.dm{font-weight:600}` 同口径），而底座按契约 `font_weight >= 600` 去要**粗体文件** ——
        // 不给就退回 `--font-file`（常规体）⇒ **笔画比预览细一圈**（底座自己的注释原话）。
        // 底座不猜系统字体，所以这个文件也得由这一侧解析出来。
        .args(
            font_bold
                .as_ref()
                .map(|b| vec!["--font-bold-file".to_string(), b.display().to_string()])
                .unwrap_or_default(),
        )
        // **字体库整个交给底座**：它按每条轨的 `font_family`（= 库里那个 id，转译器写的是
        // 项目配置里的值）在目录里找 —— 只给 `--font-file` 的话，"库里选了字体"这件事
        // 在成片里只对**字幕那一支**生效、弹幕那支仍然是兜底字体。
        .arg("--font-dir")
        .arg(vtrim_core::path::font_dir())
        .stdout(std::process::Stdio::from(progress_file))
        .stderr(std::process::Stdio::inherit());
    if !with_audio {
        cmd.arg("--no-audio");
    }
    let mut child = cmd
        .spawn()
        .with_context(|| format!("起 Dhampir 失败：{}", binary.display()))?;

    // 边等边翻译进度：每行翻成与当年 chrome 腿同形的 `frames A..B: NN%`（父进程的 parser 认这个），
    // 但最多每 1.5s 打一条（进度行本身不进日志，靠任务卡的进度条表达）。
    let mut offset = 0u64;
    let mut buf: Vec<u8> = Vec::new();
    let mut sink = ProgressSink::new(t);
    let status = loop {
        for line in read_new_lines(&progress_path, &mut offset, &mut buf) {
            sink.on_line(line);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(400)),
            Err(e) => bail!("等 Dhampir 退出失败：{}（{}）", binary.display(), e),
        }
    };
    // 收尾再读一遍：进程退出与最后一次写入之间可能还有没被 tail 到的行
    for line in read_new_lines(&progress_path, &mut offset, &mut buf) {
        sink.on_line(line);
    }
    let _ = std::fs::remove_file(&progress_path);
    let (seen_done, last_line) = (sink.seen_done, sink.last_line);

    if !status.success() && !seen_done {
        bail!(
            "Dhampir 出片失败（exit={:?}）：{}\n  最后一行：{}\n  （底座的 warn/诊断在上面的日志里）",
            status.code(),
            binary.display(),
            last_line
        );
    }
    if !out.is_file() {
        bail!(
            "Dhampir 报了成功，但产物不存在：{} —— 不当作成功",
            out.display()
        );
    }
    tracing::info!(
        "  v Dhampir 出片 -> {}  ⏱ {:.1}s",
        out.display(),
        t.elapsed().as_secs_f64()
    );
    Ok(())
}

/// **出一条只含画面的片子**（音轨留给本仓）。
///
/// 帧区间取 `[0, 工程的序列长度)`：**帧数从转译产物里读**（见 [`sequence_frames`]），
/// 不用调用方那个 `duration × fps` 的估算 —— 上层的 `fps` 是写死的 30，而工程的时间基
/// 是素材自己的（60），拿它算会**只出一半**。Dhampir 的 `--to` 是**闭区间**，所以传 `n - 1`。
///
/// `project_font` = `style.subtitle.font`（字体库 id，空 = 系统字体栈）—— 给 [`find_font`] 用。
///
/// `strict` = `polish render --strict`：**转译报告里 `DROP > 0` 就拒绝出片**。
/// 给"不允许静默缺效果"的人一个开关（默认关：DROP 清单是**已知且逐条有理由**的，
/// 见 `docs/plan/dhampir-parity-register.md` §五 M2）。
pub fn render_video_only(
    project_path: &Path,
    clip_dir: &Path,
    project_font: &str,
    strict: bool,
) -> Result<PathBuf> {
    let out = video_intermediate(project_path);
    // **出片：整条时间线**（`keep_muted = true`）—— 屏蔽区间由 `cmd_render` 尾部那一剪按源时间去掉，
    // 与当年 chrome 那条腿同一处、同一条口径。传 false 会**剪两次**（成片短一大截 + 音轨错位）。
    let (translated, report) = prepare_project_reporting(clip_dir, project_path, true)?;
    if strict && report.drop_count > 0 {
        cleanup_project(project_path);
        bail!(
            "--strict：转译报告里有 {} 条 DROP（参照有、底座表达不了的），拒绝出片。\n  逐条：\n    {}\n  \
             （不加 `--strict` 就会照常出片，但这些效果在成片里是缺的）",
            report.drop_count,
            report.drops.join("\n    ")
        );
    }
    let frames = sequence_frames(&translated)?;
    run_render(&translated, clip_dir, 0, frames - 1, &out, false, project_font)?;
    cleanup_project(project_path);
    Ok(out)
}



/// 供 UI / 日志用的一句话：这一趟用的是哪个二进制、哪个转译器、哪个字体。
///
/// `project_font` = `style.subtitle.font`：字体那一行**要报真正会用的那一支**
/// （库里选了字体时它就不是系统兜底那份了），否则日志会与成片对不上。
pub fn describe(project_font: &str) -> String {
    let b = find_binary()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|e| format!("<找不到：{e}>"));
    let t = find_translator()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|e| format!("<找不到：{e}>"));
    // **常规 + 粗体都打出来**：字幕是 `font_weight: 700`、弹幕 `600`，底座要的是**粗体那一支**；
    // 少了它笔画就比预览细一圈（这正是"字体不对"的那种症状），所以出片日志里要看得见。
    let f = match find_font(project_font) {
        Ok((regular, bold)) => match bold {
            Some(b) => format!("{}（粗体 {}）", regular.display(), b.display()),
            None => format!("{}（**没有粗体那一支** → 700/600 的字重会退回常规体）", regular.display()),
        },
        Err(e) => format!("<找不到：{e}>"),
    };
    format!("dhampir={b}\n  translator={t}\n  font={f}")
}

/// 把 `anyhow::Error` 转成给用户看的一段话（日志/接口回包都用得上）。
pub fn explain(error: &anyhow::Error) -> String {
    let mut chain = vec![error.to_string()];
    let mut cur = error.source();
    while let Some(next) = cur {
        chain.push(next.to_string());
        cur = next.source();
    }
    chain.join(" <- ")
}

#[cfg(test)]
mod artifact_tests {
    use super::*;

    /// 造一个**最小但齐全**的产物目录（`<root>/preview/...` + `<root>/VERSION`）。
    fn make_artifact(tag: &str, with_pkg: bool) -> PathBuf {
        let root = std::env::temp_dir().join(format!("vtrim_dhampir_artifact_{tag}"));
        let preview = root.join("preview");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(preview.join("pkg")).unwrap();
        std::fs::write(preview.join("engine.js"), b"// engine").unwrap();
        if with_pkg {
            std::fs::write(preview.join("pkg/dhampir_wasm.js"), b"// glue").unwrap();
            std::fs::write(preview.join("pkg/dhampir_wasm_bg.wasm"), b"\0asm").unwrap();
        }
        std::fs::write(root.join("VERSION"), "project_schema=1\ngit=test\n").unwrap();
        preview
    }

    /// 齐全 → 通过。
    #[test]
    fn complete_artifact_passes() {
        let preview = make_artifact("ok", true);
        verify_artifact_dir(&preview).expect("齐全的产物不该报错");
    }

    /// **缺一件就必须明确报错** —— 而不是退回源码树那条路
    /// （退回去会把"部署坏了"伪装成"开发形态"，用户看到的是白屏而不是一句话）。
    ///
    /// 这条测试的意义在于：它**会红**。没有它，"三条自检"只是一段从没被执行过的代码。
    #[test]
    fn missing_piece_is_an_explicit_error() {
        let preview = make_artifact("missing", false);
        let err = verify_artifact_dir(&preview).expect_err("缺 pkg 两件时必须报错");
        let msg = format!("{err}");
        assert!(msg.contains("不齐"), "报错要说清\"不齐\"：{msg}");
        assert!(
            msg.contains("dhampir_wasm.js") && msg.contains("dhampir_wasm_bg.wasm"),
            "报错要**列出缺了哪些**：{msg}"
        );
        assert!(
            msg.contains("package.mjs"),
            "报错要告诉人下一步去哪儿拿产物：{msg}"
        );
    }

    /// 缺 `VERSION` → 也要报错（它是产物与 V-Trim 之间的契约版本）。
    #[test]
    fn missing_version_is_an_explicit_error() {
        let preview = make_artifact("noversion", true);
        std::fs::remove_file(preview.parent().unwrap().join("VERSION")).unwrap();
        let err = verify_artifact_dir(&preview).expect_err("缺 VERSION 时必须报错");
        assert!(format!("{err}").contains("VERSION"), "报错要点名 VERSION");
    }

    // ---- M3：就绪位与转译报告的解析 ----

    /// **指到不存在的二进制 ⇒ 就绪位必须是 false，且问题里说清是哪一个。**
    ///
    /// 这条是"底座不可用要在渲染前可见"的钉子：`/api/polish/dhampir-status`、`vtrim doctor`、
    /// 前端提示都读同一个 `readiness()` —— 它要是永远返回 true，那三处就全是装饰。
    #[test]
    fn readiness_reports_missing_binary() {
        let key = "VTEDIT_DHAMPIR_CLI";
        let old = std::env::var_os(key);
        let bogus = std::env::temp_dir().join("vtrim-no-such-dhampir.exe");
        let _ = std::fs::remove_file(&bogus);
        // ⚠️ `set_var` 在 Rust 2024 是 unsafe（进程级环境变量与线程共享）。
        // 这个测试是**唯一**动 `VTEDIT_DHAMPIR_CLI` 的地方，且只读它一次、立刻还原 ——
        // 测试并行下仍属"约定俗成的小风险"，换来的是"就绪位真的会判 false"这条钉子。
        unsafe { std::env::set_var(key, &bogus) };
        let r = readiness("", false);
        unsafe {
            match old {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
        assert!(!r.ready, "指到不存在的二进制时必须 readiness=false");
        assert!(r.binary.is_none(), "定位不到就不该给出路径");
        assert!(
            r.problems.iter().any(|p| p.contains("出片二进制")),
            "问题清单要点名是哪一项坏了：{:?}",
            r.problems
        );
    }

    // ---- M5（去 Node 那条线）：版本要求的**纯函数**钉子 ----

    /// `node --version` 的解析：认 `v20.11.1` / `20.11.1` / `v18`（缺小段按 0 算）。
    ///
    /// 为什么钉它：这条判据决定"就绪位说不说'Node 版本太低'"。真实环境没法安全地
    /// 假装 node 是 v16（改 PATH 是进程级的），所以把判断抽成纯函数来测。
    #[test]
    fn node_version_parsing() {
        assert_eq!(parse_node_version("v20.11.1\n"), Some((20, 11)));
        assert_eq!(parse_node_version("18.19.0"), Some((18, 19)));
        assert_eq!(parse_node_version("v18"), Some((18, 0)));
        assert_eq!(parse_node_version("garbage"), None);
        assert_eq!(parse_node_version(""), None);
        // 判据：大版本 < 18 才算"太低"（与 `MIN_NODE_MAJOR` 同源）
        assert_eq!(parse_node_version("v16.20.2").map(|(m, _)| m < MIN_NODE_MAJOR), Some(true));
        assert_eq!(parse_node_version("v18.0.0").map(|(m, _)| m < MIN_NODE_MAJOR), Some(false));
    }

    /// "PATH 里有没有 node" 这条便宜判断：给一个**临时目录**当 PATH 来测（不动真 PATH）。
    #[test]
    fn node_lookup_is_a_path_scan() {
        let dir = std::env::temp_dir().join(format!("vtrim-node-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let name = if cfg!(windows) { "node.exe" } else { "node" };
        // 空目录 ⇒ 找不到
        assert!(node_in_path_var(Some(dir.clone().into_os_string())).is_none());
        // 放一个同名文件 ⇒ 找得到（**不要求它可执行**：这正是"商店桩"那种形状，
        // 所以还需要 `deep` 那趟 `node --version` 才能分辨）
        let fake = dir.join(name);
        std::fs::write(&fake, b"stub").unwrap();
        assert_eq!(node_in_path_var(Some(dir.clone().into_os_string())), Some(fake.clone()));
        // 多个目录：按 PATH 顺序取第一个命中的
        let empty = std::env::temp_dir().join(format!("vtrim-node-scan2-{}", std::process::id()));
        let joined = std::env::join_paths([empty, dir.clone()]).unwrap();
        assert_eq!(node_in_path_var(Some(joined)), Some(fake));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **"要按哪几支族名去找"= 项目选的那支 + 预览那份动态栈**（顺序就是优先级）。
    ///
    /// 这条钉的是"字体不对"的根因：底座那条腿以前**完全不看**字体栈，只查字体库
    /// （库里没有就退 `msyh.ttc`），而预览落到栈里第一支装了的字体 ⇒ 两边字族不同。
    /// ⚠️ 断言**动态**算（首项随库里默认走，开发机装了字体、CI 没装都不能红）。
    #[test]
    fn font_search_order_follows_preview_stack() {
        // 项目选的那支永远排最前
        assert_eq!(font_search_order("SimHei")[0], "SimHei");
        assert_eq!(font_search_order("  ")[0], font_search_order("")[0], "空字体不产生空段");
        // 项目字体之后 = 动态兜底栈逐支同序（就是预览 `.cap` 那份的来源）
        let stack: Vec<String> = crate::render::server::fallback_stack()
            .split(',')
            .map(|s| s.trim().trim_matches(['\'', '"']).trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        assert!(!stack.is_empty(), "兜底栈不能是空的");
        let with_font = font_search_order("SimHei");
        assert_eq!(with_font[1..], stack[..], "项目字体之后必须 = 动态兜底栈逐支同序");
        // 与栈里已有的重名只留一份（Noto 在栈里 ⇒ 比"项目字体 + 全栈"短一项）
        assert_eq!(
            font_search_order("Noto Sans SC").len(),
            with_font.len() - 1,
            "与栈重名只留一份"
        );
        // 空项目字体 = 就是栈本身（不产生空段）
        assert_eq!(font_search_order("  "), stack, "空字体 ⇒ 兜底栈本身");
        // 尾部顺序钉死：雅黑在 Noto 前（与预览 `.cap` 栈同序 —— 预览的赢家 = 成片的赢家）
        let order = font_search_order("");
        let yahei = order.iter().position(|s| s == "Microsoft YaHei").expect("栈里必须有雅黑");
        let noto = order.iter().position(|s| s == "Noto Sans SC").expect("栈里必须有 Noto");
        assert!(yahei < noto, "雅黑必须在 Noto 前（预览那份栈的顺序）: {order:?}");
        // （"回落态与老常量同族名同序"那条钉在 `server.rs::stack_head_tracks_installed_library_font`
        //   —— 它必须条件化（开发机设了默认字体时首项不同），别在这里写死再红一遍。）
    }

    /// **粗体那一支**：`msyh.ttc` → `msyhbd.ttc`；可变字体没有独立粗体文件就得老实给 `None`
    /// （底座不合成粗体，只会退回常规体 —— 所以"有没有粗体"必须能在日志里看见）。
    #[test]
    fn pick_bold_from_listing_finds_the_bold_sibling() {
        let listing = vec![
            ("msyhttc".to_string(), PathBuf::from("C:/f/msyh.ttc")),
            ("msyhbdttc".to_string(), PathBuf::from("C:/f/msyhbd.ttc")),
            ("msyhlttc".to_string(), PathBuf::from("C:/f/msyhl.ttc")),
            ("notosansscvfttf".to_string(), PathBuf::from("C:/f/NotoSansSC-VF.ttf")),
        ];
        assert_eq!(
            pick_bold_from_listing(&listing, "Microsoft YaHei"),
            Some(PathBuf::from("C:/f/msyhbd.ttc")),
            "粗体要挑到 msyhbd（不是常规 msyh、也不是细体 msyhl）"
        );
        assert_eq!(
            pick_bold_from_listing(&listing, "Noto Sans SC"),
            None,
            "可变字体只有一支文件 ⇒ 没有独立粗体，返回 None（调用方据此在日志里说清）"
        );
        assert_eq!(pick_font_from_listing(&listing, "Microsoft YaHei"), Some(PathBuf::from("C:/f/msyh.ttc")));
    }

    /// 在**合成清单**里挑字体：别名（`Microsoft YaHei` → `msyh.ttc`）、归一化包含
    /// （`Noto Sans SC` → `NotoSansSC-VF.ttf`）、以及**优先非粗非斜**（`msyh` 别挑到 `msyhbd`）。
    #[test]
    fn pick_font_from_listing_prefers_regular_and_uses_aliases() {
        let listing = vec![
            ("msyhbdttc".to_string(), PathBuf::from("C:/f/msyhbd.ttc")),
            ("msyhttc".to_string(), PathBuf::from("C:/f/msyh.ttc")),
            ("notosansscvfttf".to_string(), PathBuf::from("C:/f/NotoSansSC-VF.ttf")),
            ("lxgwwenkai_boldttf".to_string(), PathBuf::from("C:/f/LXGWWenKai_Bold.ttf")),
            ("simheittf".to_string(), PathBuf::from("C:/f/simhei.ttf")),
        ];
        // 归一化包含：族名中间有空格、文件名带 `-VF` 后缀，也要认出同一族
        assert_eq!(
            pick_font_from_listing(&listing, "Noto Sans SC"),
            Some(PathBuf::from("C:/f/NotoSansSC-VF.ttf"))
        );
        // 别名表：Microsoft YaHei 的文件叫 msyh.ttc —— 且**必须挑常规体**（不是 msyhbd）
        assert_eq!(
            pick_font_from_listing(&listing, "Microsoft YaHei"),
            Some(PathBuf::from("C:/f/msyh.ttc"))
        );
        // 只有粗体时退而求其次（有总比没有强）
        let only_bold = vec![("msyhbdttc".to_string(), PathBuf::from("C:/f/msyhbd.ttc"))];
        assert_eq!(
            pick_font_from_listing(&only_bold, "Microsoft YaHei"),
            Some(PathBuf::from("C:/f/msyhbd.ttc"))
        );
        // 认不出就返回 None（由调用方继续沿栈往下找）—— 不许"猜一个相似的"
        assert_eq!(pick_font_from_listing(&listing, "Absolutely Not Installed"), None);
    }

    /// 转译报告的解析（`--strict` 的判据就靠它）：分隔行、逐条文本、计数。
    #[test]
    fn translate_report_parses_drop_and_clamp() {
        let stdout = "转译完成 -> x\n  轨道 3 条 / 素材 1 个\n\n---- DROP (2) ----\n  [a] 第一条\n  [b] 第二条\n\n---- CLAMP (1) ----\n  [c] 降级那条\n\n---- NOTE (1) ----\n  [d] 注意\n";
        let r = TranslateReport::parse(stdout);
        assert_eq!(r.drop_count, 2);
        assert_eq!(r.clamp_count, 1);
        assert!(r.drops[0].contains("a") && r.drops[0].contains("第一条"));
        assert!(r.clamps[0].contains("降级那条"));
        // 没有报告（正常工程）也要能解析成 0，而不是崩
        let empty = TranslateReport::parse("转译完成 -> x\n  轨道 1 条\n");
        assert_eq!((empty.drop_count, empty.clamp_count), (0, 0));
    }
}
