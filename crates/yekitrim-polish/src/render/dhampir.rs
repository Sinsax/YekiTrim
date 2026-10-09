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
/// 在 YekiTrim 里再抄一份期望值，就等于给同一条规矩留了第二处实现。
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
            "Dhampir 产物缺 VERSION（{}）—— 它是产物与 YekiTrim 之间的契约版本，不能没有。",
            version.map(|v| v.display().to_string()).unwrap_or_else(|| "<未知>".into())
        ),
    }
    Ok(())
}

/// 这一层不想引日志依赖（`yekitrim-polish` 是库）—— 只在 `VTEDIT_DHAMPIR_VERBOSE` 时打印。
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

    // 本仓 exe 的位置：`<YekiTrim>/target/<profile>/yekitrim[.exe]`
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

    // 本仓源码根（编译期常量）：`<YekiTrim>/crates/yekitrim-polish`
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
        "找不到 Dhampir 二进制（{exe}）。设 VTEDIT_DHAMPIR=<路径>，或把 Dhampir 与 YekiTrim 放在同一层目录。\n  找过：\n    {}\n  \
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
/// 2. **`<程序目录>/runtime/polish-to-dhampir.mjs`** —— 打包后的位置（`xtask package` 把
///    转译器与 `effects.json` **一起**放进 `runtime/`；转译器自己先试**同目录**的
///    `effects.json`、再回落 `../spec/effects.json`，两条路见它的 `EFFECT_VOCAB`）
/// 3. `<程序目录>/tools/polish-to-dhampir.mjs` —— **老位置**（2026-10-09 之前的包 + 存量安装）
/// 4. `<程序目录>/../../{runtime,tools}/…` —— 开发树（`target/debug/yekitrim.exe` 往上两级就是仓库根）
/// 5. `CARGO_MANIFEST_DIR/../..` —— **构建期**路径，只当最后的保险
///
/// ⚠️ **第 4 条以前是唯一回退** ⇒ 装到别的机器上它指向一个**不存在的构建目录**，
/// 于是"默认出片通道"在发布形态下**根本起不来**（`find_binary`/`wasm_dirs` 早就有 `current_exe()`
/// 回退，只有它没有）。第 2 条**必须排在前面**：包内那一份才是与这个二进制配套的那一份。
/// ⚠️ **新位（`runtime/`）必须排在新老并存时的前面**：两个都在说明用户手动拷过一份，
/// 而"用哪一份"要可预期（这条曾经踩过：旧副本排第一、静默生效四天）。
/// 把 mtime 打成人能读的一行（日志里"用了哪份转译器"要能一眼看出新旧）。
fn fmt_mtime(t: std::time::SystemTime) -> String {
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 不引 chrono：只要"能比大小、能看出是哪天"，UTC 秒足够（配合大小一起看）。
    format!("mtime={secs}s(UTC)")
}

/// 候选顺序（**纯函数，可单测**）：开发构建里仓库那份排第一，发布包里 `<程序目录>/runtime` 排第一。
///
/// ⚠️ 顺序就是这条 bug 的全部：`<exe 目录>/tools/` 里可能躺着一份**打包时拷进来的旧副本**，
/// 它排第一就静默生效（本机实测四天），所有转译器修复都被误判成"没生效"。
/// 判据：`dev_build_candidates_put_repo_first`。
///
/// ⚠️ **2026-10-09：发布包里的落点由 `tools/` 改成 `runtime/`**（与词汇表合并成一个目录）。
/// 老 `<程序目录>/tools/` 仍留在候选里当**回落** —— 存量安装升级时那个目录还在，
/// 不给回落就会"升级后找不到转译器、出片直接报错"。
/// 新位必须排在老位**之前**：两个都在时（用户手动拷过一份）要优先用新的那个。
fn translator_candidates_for(exe: &Path) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let under_target = exe.components().any(|c| c.as_os_str() == "target");
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(|r| r.join("tools").join("polish-to-dhampir.mjs"));
    if under_target {
        if let Some(repo) = &repo {
            candidates.push(repo.clone());
        }
    }
    if let Some(dir) = exe.parent() {
        // 新位（`<程序目录>/runtime/`）优先，老位（`<程序目录>/tools/`）兜底
        candidates.push(dir.join("runtime").join("polish-to-dhampir.mjs"));
        candidates.push(dir.join("tools").join("polish-to-dhampir.mjs"));
        if let Some(up2) = dir.parent().and_then(Path::parent) {
            candidates.push(up2.join("runtime").join("polish-to-dhampir.mjs"));
            candidates.push(up2.join("tools").join("polish-to-dhampir.mjs"));
        }
    }
    if let Some(repo) = repo {
        candidates.push(repo);
    }
    candidates
}

fn find_translator() -> Result<PathBuf> {
    let mut tried: Vec<String> = Vec::new();
    if let Ok(explicit) = std::env::var("VTEDIT_TRANSLATOR") {
        let p = PathBuf::from(&explicit);
        if p.is_file() {
            return Ok(p);
        }
        bail!("VTEDIT_TRANSLATOR={explicit} 不是文件");
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("yekitrim"));
    let under_target = exe.components().any(|c| c.as_os_str() == "target");
    let repo_translator = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(|repo| repo.join("tools").join("polish-to-dhampir.mjs"));
    let candidates = translator_candidates_for(&exe);
    for cand in candidates {
        if cand.is_file() {
            // **把"用了哪一份"记下来**：这条曾经是隐形的 —— 旧副本排在第一位、静默生效了四天，
            // 期间所有转译器改动都被误判成"没生效/代码没写对"。顺手把"比仓库那份旧"报成 warn。
            let meta = std::fs::metadata(&cand).ok();
            tracing::info!(
                "  · 转译器：{}（{} 字节，{}）",
                cand.display(),
                meta.as_ref().map(|m| m.len().to_string()).unwrap_or_else(|| "?".into()),
                meta.as_ref()
                    .and_then(|m| m.modified().ok())
                    .map(fmt_mtime)
                    .unwrap_or_else(|| "mtime 未知".into())
            );
            if let (Some(repo), true) = (repo_translator.as_ref(), under_target) {
                if repo.is_file() && repo != &cand {
                    let newer = match (meta.as_ref().and_then(|m| m.modified().ok()), std::fs::metadata(repo).and_then(|m| m.modified())) {
                        (Some(picked), Ok(repo_t)) => repo_t > picked,
                        _ => false,
                    };
                    if newer {
                        tracing::warn!(
                            "  ⚠ 选中的转译器比仓库那份**旧**（{}）—— 打包时拷进来的副本会排在前面，                             先删掉它或重建，否则转译器的新改动不会生效",
                            repo.display()
                        );
                    }
                }
            }
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
    // ⚠️ **先让字体库就绪**：`ensure_dir` 里带着那次幂等的“把非 ASCII 条目搬成 ASCII”迁移
    // （`yekitrim_core::fonts::migrate_ascii_layouts`）。CLI 也可能在没跑过 server / `config init`
    // 的机器上出片 —— 那时库里还是 `<中文目录>/<中文名>.ttf`，解析出来的路径直接交给 ffmpeg
    // 就是**每条字幕访问违例一次**（240 帧 4s → 37s，且一个字都画不出来）。
    // 失败不拦：库不可用时后面的解析会给它自己的错误。
    // ⚠️ **按归属**（子进程经 `YEKITRIM_OWNER` 从父进程继承身份）——
    // 多用户下必须看**这个账号自己的**字体库，否则 A 导入的字体 B 出片时会被用上（不报错）。
    let owner = yekitrim_core::owner::Owner::from_env();
    let _ = yekitrim_core::fonts::ensure_dir_for(owner);
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
        if let Some(p) = yekitrim_core::fonts::resolve_file_for(owner, &family) {
            // 库内字体：粗体就在库里按 `<id> Bold` 找；找不到就交给底座（它会退回这一支）
            let bold = yekitrim_core::fonts::resolve_file_for(owner, &format!("{family} Bold"))
                .filter(|b| *b != p);
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

/// 就绪位：**这一台机器能不能用底座出片**（`/api/polish/dhampir`、`yekitrim doctor`、
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
    /// **产品版本**（`VERSION` 的 `version=`，如 `0.2.0`）——"这一版是哪一版"。
    ///
    /// ⚠️ 它与 `contract_version` 是**两条独立的线**：契约号只在兼容变更时动，而**产物字节**
    /// 每次都可能变。0.2.0 就是「`project_schema` 没动、字幕栅格化器却换了」的那一版 ——
    /// 那种改动**不会被任何契约校验拦下**，所以产品版本必须可见。
    pub product_version: Option<String>,
    /// wasm 导出面契约（`VERSION` 的 `host_api=`）——只有定版产物才有。
    pub host_api_version: Option<String>,
    /// `node --version` 的原文（**只有深探才有**；便宜那一趟是 `None`）。
    pub node_version: Option<String>,
    /// `ffmpeg -version` 第一行 + 有没有 libass（**只有深探才有**；便宜那一趟是 `None`）。
    ///
    /// 底座 0.2.0 起字幕走 libass（`subtitles=`），所以「ffmpeg 带不带它」是**出片前置**。
    /// 不带时底座会响亮失败（不会静默出无字幕的片子），但那要等到出片那一刻 —— 这里提前说。
    pub ffmpeg: Option<String>,
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
    // 字幕栅格化要 ffmpeg 带 libass（底座 0.2.0 起字幕走 `subtitles=`）——
    // **不带的 ffmpeg 不会静默出错**（底座会响亮失败），但会在**出片那一刻**才炸。
    // 所以放在这里提前说。⚠️ 只有 `deep` 才探：这条要真起一次 ffmpeg，而就绪位在
    // `/api/health` 这类热路径上会被调（与 `node_version` 同一条纪律）。
    let mut ffmpeg_label = None;
    if deep {
        match ffmpeg_libass() {
            Some(version) => {
                if !version.libass {
                    problems.push(
                        "ffmpeg 没有带 libass（字幕要它）—— 换一个 `--enable-libass` 的构建；\
                         自检：ffmpeg -filters | grep subtitles"
                            .to_string(),
                    );
                }
                ffmpeg_label = Some(version.text);
            }
            None => problems.push(
                "找不到 ffmpeg（或它跑不起来）—— 出片要它做音轨/合流，字幕还要 libass".to_string(),
            ),
        }
    }
    // 预览产物是可选的（只有页面预览要用），缺了不影响出片 —— 只记在字段里，不进 problems。
    let (preview, contract_version, product_version, host_api_version) = match wasm_dirs() {
        Ok((engine, _pkg)) => {
            let text = engine
                .parent()
                .and_then(|dir| std::fs::read_to_string(dir.join("VERSION")).ok());
            let field = |key: &str| {
                text.as_deref().and_then(|t| {
                    t.lines()
                        .find_map(|l| l.trim().strip_prefix(key).map(str::to_string))
                })
            };
            (
                Some(engine.display().to_string()),
                field("project_schema="),
                field("version="),
                field("host_api="),
            )
        }
        Err(_) => (None, None, None, None),
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
        product_version,
        host_api_version,
        node_version: node_version_label,
        ffmpeg: ffmpeg_label,
        spec,
        problems,
    }
}

/// `ffmpeg -version` 的第一行 + **有没有 libass**（`[Parsed_subtitles_…]` 那个滤镜）。
///
/// ⚠️ 判据是 `-filters` 里有没有 `subtitles` 这一条，**不是**去 grep `--enable-libass` ——
/// 后者只是构建配置的字符串，各家发行版写法不一；而滤镜**在不在**才是能不能用的判据。
/// 上游给的正是这条自检命令（`ffmpeg -filters | grep subtitles`），这里照它实现。
pub struct FfmpegInfo {
    /// `ffmpeg -version` 的第一行（带构建标识，人读用）。
    pub text: String,
    pub libass: bool,
}

fn ffmpeg_libass() -> Option<FfmpegInfo> {
    let ver = yekitrim_core::process::cmd("ffmpeg")
        .arg("-version")
        .output()
        .ok()?;
    if !ver.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&ver.stdout)
        .lines()
        .next()
        .unwrap_or("ffmpeg")
        .trim()
        .to_string();
    let filters = yekitrim_core::process::cmd("ffmpeg")
        .args(["-hide_banner", "-filters"])
        .output()
        .ok()?;
    // 认的是滤镜表里那一行（`.. subtitles  V->V  Render text subtitles using the libass library.`）。
    let libass = String::from_utf8_lossy(&filters.stdout)
        .lines()
        .any(|l| l.split_whitespace().nth(1) == Some("subtitles"));
    Some(FfmpegInfo { text, libass })
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
    let out = yekitrim_core::process::cmd("node").arg("--version").output().ok()?;
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
pub fn prepare_project(
    clip_dir: &Path,
    project_path: &Path,
    keep_muted: bool,
    video: Option<&str>,
) -> Result<PathBuf> {
    Ok(prepare_project_reporting(clip_dir, project_path, keep_muted, video)?.0)
}

/// 传给转译器的「库 id → 真族名」映射的**文件名**（写在工程目录里，出片结束由
/// [`cleanup_project`] 收掉）。`_temp_` 前缀与其它草稿同一套命名 —— 启动清扫按这个前缀收孤儿。
const FONT_FAMILIES_FILE: &str = "_temp_font_families.json";

/// 写一份「库 id → 真族名」映射给转译器（`--font-families <json>`）。
///
/// 为什么要有它：底座 0.2.0 起走 libass，而 **libass 按族名找字体**，`polish.toml` 里存的
/// 却是**库 id**。两者在库里**确实会不一样**（实测 id `GenJyuuGothic-Bold` vs 族名
/// `Gen Jyuu Gothic Bold`），而名字对不上时 libass **不报错** —— 静默落 `ArialMT`，
/// 且 `lines_failed: 0`、`issues: []`（比 `.notdef` 还隐蔽）。
///
/// ⚠️ 与 [`yekitrim_core::fonts::family_for`] 同一个口径（**库外原样返回**）：
/// 老工程 `font = "LXGW WenKai"` 那种系统字体族名不该被改写 —— 所以这里**只放库里有的 id**，
/// 库外的值由转译器自己原样保留。
fn write_font_families(dest: &Path) -> Result<()> {
    let mut map = serde_json::Map::new();
    for e in yekitrim_core::fonts::scan_all(&yekitrim_core::fonts::dir()) {
        let fam = e.family_internal.trim();
        // 族名空 / 与 id 相同 ⇒ 没有"改写"这回事，省掉一条映射。
        if fam.is_empty() || fam == e.id {
            continue;
        }
        map.insert(e.id.clone(), serde_json::Value::String(fam.to_string()));
    }
    let body = serde_json::to_vec(&serde_json::Value::Object(map))?;
    std::fs::write(dest, body)
        .with_context(|| format!("写字体族名映射失败：{}", dest.display()))?;
    Ok(())
}

/// 与 [`prepare_project`] 同一条路，但**把转译报告也带回来**（`--strict` 要用它）。
pub fn prepare_project_reporting(
    clip_dir: &Path,
    project_path: &Path,
    keep_muted: bool,
    video: Option<&str>,
) -> Result<(PathBuf, TranslateReport)> {
    let translator = find_translator()?;
    let translated = project_path.join("_dhampir_project.json");
    let font_families = project_path.join(FONT_FAMILIES_FILE);
    write_font_families(&font_families)?;
    let t = std::time::Instant::now();
    let mut cmd = yekitrim_core::process::cmd("node");
    cmd.arg(&translator).arg(clip_dir).arg("--out").arg(&translated);
    cmd.arg("--font-families").arg(&font_families);
    if keep_muted {
        // **出片**那条腿：时间线保持整条，屏蔽区间交给尾部那一剪（见转译器里的说明）。
        // 预览**不能**加这个 —— 客户端要靠压缩时间线跳屏蔽区间。
        cmd.arg("--keep-muted");
    }
    if let Some(v) = video {
        // **只给预览**：把"实际媒体"换成 720p 代理。转译器会按**代理**探测时基/分辨率，
        // 所以层的 scale/FIT 也是按代理算的（见 `render/proxy.rs` 模块头）。
        // ⚠️ 出片那条腿必须传 `None` —— 成片永远用源。
        cmd.arg("--video").arg(v);
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
        // 传给转译器的字体族名映射（`--font-families`）。⚠️ 忘了它就会**每出一次片、
        // 在用户工程目录里留一个 `_temp_font_families.json`**（我自己实测漏过一次）——
        // 它不属于工程，不该出现在那儿。
        translated.with_file_name(FONT_FAMILIES_FILE),
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
/// `yekitrim server` 的 `follow_render_output` 认的是当年那条 chrome 腿留下的 `frames A..B: NN%` 形状 ——
/// 分钟级的出片在 WebUI 上就是"卡在 0%"。契约留在本仓（不让底座为 YekiTrim 定制输出）。
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
    // 归属在**这个函数里也算一次**（`find_font` 里那次是它自己的局部变量）：
    // `--font-dir` 交给底座的必须是同一个库。
    let owner = yekitrim_core::owner::Owner::from_env();
    let (font, font_bold) = find_font(project_font)?;
    let t = std::time::Instant::now();
    let progress_path = out.with_file_name("_temp_dhampir_progress.ndjson");
    let progress_file = std::fs::File::create(&progress_path)
        .with_context(|| format!("创建进度文件失败：{}", progress_path.display()))?;
    let mut cmd = yekitrim_core::process::cmd(&binary);
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
        // 同一个归属：底座按 `font_family` 在这个目录里找 —— 给错目录就是"库里选了字体
        // 在成片里只对字幕生效 / 或干脆换了别人的字体"。
        .arg(yekitrim_core::fonts::dir_for(owner))
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
    // 出片**永远用源**：`video = None`（代理只服务预览）
    let (translated, report) = prepare_project_reporting(clip_dir, project_path, true, None)?;
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
        let root = std::env::temp_dir().join(format!("yekitrim_dhampir_artifact_{tag}"));
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

    /// 缺 `VERSION` → 也要报错（它是产物与 YekiTrim 之间的契约版本）。
    #[test]
    fn missing_version_is_an_explicit_error() {
        let preview = make_artifact("noversion", true);
        std::fs::remove_file(preview.parent().unwrap().join("VERSION")).unwrap();
        let err = verify_artifact_dir(&preview).expect_err("缺 VERSION 时必须报错");
        assert!(format!("{err}").contains("VERSION"), "报错要点名 VERSION");
    }

    /// `VERSION` 里三个字段都要能读出来，且**三者是三条独立的线**。
    ///
    /// 为什么值得钉：产品版本（`version=`）**没有运行时 API**（wasm 侧只有
    /// `dhampir_host_api_version()`），所以"用户装的是 0.1.0 还是 0.2.0"**只能读这个文件**。
    /// 而 0.2.0 恰好是「契约号不动、字幕像素却变了」的那一版 —— 那种改动不会被任何契约
    /// 校验拦下，只有产品版本能区分它。读漏了就等于把这个盲区留在原地。
    #[test]
    fn version_file_yields_all_three_versions() {
        let dir = std::env::temp_dir().join("yekitrim_dhampir_ver3");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let text = "version=0.2.0\nproject_schema=1\nhost_api=6\nlicense=Apache-2.0\n";
        let field = |key: &str| {
            text.lines()
                .find_map(|l| l.trim().strip_prefix(key).map(str::to_string))
        };
        assert_eq!(field("version=").as_deref(), Some("0.2.0"), "产品版本");
        assert_eq!(field("project_schema=").as_deref(), Some("1"), "工程文件契约");
        assert_eq!(field("host_api=").as_deref(), Some("6"), "wasm 导出面契约");
        // ⚠️ `version=` **不许**匹配到 `host_api=` 之类的前缀相同项（这里靠的是完整前缀）。
        assert_eq!(field("api="), None, "不许把 host_api= 当成 api=");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `ffmpeg -filters` 的判据是**滤镜表里有 `subtitles`**，不是有没有 libass 这四个字母。    ///
    /// 为什么用滤镜表：`--enable-libass` 只是构建配置的字符串（各家发行版写法不一），
    /// 而**滤镜在不在**才是能不能用的判据 —— 上游给的自检命令正是 `ffmpeg -filters | grep subtitles`。
    #[test]
    fn libass_is_detected_by_filter_not_by_build_flag() {
        let parse = |out: &str| {
            out.lines()
                .any(|l| l.split_whitespace().nth(1) == Some("subtitles"))
        };
        // 真机的形状（ffmpeg 9.0.1 gyan build）。
        assert!(parse(
            " .. ass               V->V       Render ASS subtitles onto input video using the libass library.\n\
              .. subtitles         V->V       Render text subtitles onto input video using the libass library."
        ));
        // 只有构建字符串、没有滤镜 ⇒ 不算有（这条正是"别 grep --enable-libass"）。
        assert!(
            !parse("configuration: --enable-libass --enable-fontconfig"),
            "构建配置里有 --enable-libass **不等于**滤镜可用"
        );
        // 别被 `ass` 那条误判成 `subtitles`。
        assert!(!parse(" .. ass   V->V   Render ASS subtitles onto input video"));
    }

    /// `cleanup_project` 必须把**传给转译器的字体族名映射**也收掉。
    ///
    /// 为什么值得钉：那个文件是出片时写进**用户工程目录**的草稿。漏了收 = 每出一次片就多留
    /// 一个 `_temp_font_families.json`（实测我自己漏过一次，真机跑完出片后在工程目录里看到了它）。
    /// 判据刻意**对着常量**而不是字面量 —— 那样改了文件名却忘了改 cleanup 时，这条会红。
    #[test]
    fn cleanup_removes_the_font_family_map() {
        let dir = std::env::temp_dir().join("yekitrim_dhampir_cleanup");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let translated = dir.join("_dhampir_project.json");
        let families = dir.join(FONT_FAMILIES_FILE);
        for f in [&translated, &families] {
            std::fs::write(f, b"x").unwrap();
        }
        cleanup_project(&dir);
        assert!(!translated.exists(), "转译产物要收掉");
        assert!(
            !families.exists(),
            "字体族名映射也要收掉（否则工程目录里每出一次片多一个草稿）"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- M3：就绪位与转译报告的解析 ----

    /// **指到不存在的二进制 ⇒ 就绪位必须是 false，且问题里说清是哪一个。**
    ///
    /// 这条是"底座不可用要在渲染前可见"的钉子：`/api/polish/dhampir-status`、`yekitrim doctor`、
    /// 前端提示都读同一个 `readiness()` —— 它要是永远返回 true，那三处就全是装饰。
    #[test]
    fn readiness_reports_missing_binary() {
        let key = "VTEDIT_DHAMPIR_CLI";
        let old = std::env::var_os(key);
        let bogus = std::env::temp_dir().join("yekitrim-no-such-dhampir.exe");
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
        let dir = std::env::temp_dir().join(format!("yekitrim-node-scan-{}", std::process::id()));
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
        let empty = std::env::temp_dir().join(format!("yekitrim-node-scan2-{}", std::process::id()));
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
        //
        // ⚠️ **这里必须去重**：动态首项可能恰好等于栈里某项 —— 库里那支的内部族名
        // 就是 `Noto Sans SC`，而尾巴里也有一项 `Noto Sans SC`（那是"系统里那支"）。
        // `font_search_order` 本身就按族名去重，所以拿来比的期望值也得去重，否则
        // 比的是"原始串"、而被比的是"去重后的搜索序"，两者形状根本不同。
        let mut stack: Vec<String> = Vec::new();
        for family in crate::render::server::fallback_stack().split(',') {
            let s = family.trim().trim_matches(['\'', '"']).trim().to_string();
            if !s.is_empty() && !stack.iter().any(|x| x.eq_ignore_ascii_case(&s)) {
                stack.push(s);
            }
        }
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
        // 尾部顺序钉死：**雅黑在 `sans-serif` 前**（与预览 `.cap` 栈同序 —— 预览的赢家 = 成片的赢家）。
        //
        // ⚠️ 这里原来断言的是"雅黑在 `Noto Sans SC` 前"。2026-10-02 换字体后那条**失效了**：
        // 动态首项的内部族名就是 `Noto Sans SC`，去重后它只剩**下标 0 那一份** ——
        // 再去比 "雅黑 < Noto" 就变成比"雅黑 < 首项"，与它想守的"尾部顺序"不是一回事。
        // 真正要守的不变式是：**雅黑与通用族都排在末尾那段、且 sans-serif 永远垫底**。
        let order = font_search_order("");
        let yahei = order.iter().position(|s| s == "Microsoft YaHei").expect("栈里必须有雅黑");
        let last = order.len() - 1;
        assert_eq!(order[last], "sans-serif", "通用族必须垫底（它是最后一道保险）: {order:?}");
        assert!(yahei < last, "雅黑必须排在通用族之前: {order:?}");
        assert!(
            order.iter().any(|s| s == "Noto Sans SC"),
            "栈里必须有 Noto Sans SC（自带中文那支的族名）: {order:?}"
        );
        // ⚠️ **emoji 字体永远不许出现在这个搜索序里**（2026-10-02 职责分离的收口判据）。
        //
        // 底座那条腿**没有逐字形回退**：`find_font` 命中第一个能解析的族就 return，
        // 最终落成 ffmpeg `drawtext=fontfile=<单支>`（见本文件与 Dhampir 仓的 `text_raster.rs`）。
        // 所以 emoji 族一旦混进这张表并排在前面，**整条字幕轨**都会交给一支没有中文字形的字体
        // —— 中文全变豆腐块（而且不报错）。这正是"字幕 emoji 本仓不可支持"的原因：
        // 要支持得改底座契约（逐字形 runs + 彩色位图光栅化），不是改这里的栈。
        for fam in &order {
            assert!(
                !fam.to_ascii_lowercase().contains("emoji"),
                "emoji 族名不许进底座的字体搜索序（会让整条字幕轨变豆腐块）：{fam}"
            );
        }
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

    /// **开发构建必须优先仓库那份转译器**（2026-10-06 用户报的"画面构成里怎么改位置都是居中"的根因）。
    ///
    /// `<exe 目录>/tools/` 里可能有打包时拷进去的**旧副本**；它排第一就静默生效，
    /// 于是转译器的一切修复（两轴位置 / 真实时基与分辨率 / `--video` 代理）全部"没生效"。
    #[test]
    fn dev_build_candidates_put_repo_first() {
        let dev = translator_candidates_for(Path::new("F:/repo/target/debug/yekitrim.exe"));
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("仓库根")
            .join("tools")
            .join("polish-to-dhampir.mjs");
        assert_eq!(
            dev.first().map(|p| p == &repo),
            Some(true),
            "开发构建（exe 在 target/ 下）必须把仓库那份排第一，实得 {:?}",
            dev.first()
        );
        assert!(dev.len() >= 2, "候选里还得留着打包位置那条兜底：{dev:?}");

        // **发布包**：新位 `<程序目录>/runtime` 排第一（2026-10-09 由 `tools/` 改来）
        let pkg = translator_candidates_for(Path::new("C:/app/yekitrim.exe"));
        assert_eq!(
            pkg.first(),
            Some(&PathBuf::from("C:/app/runtime/polish-to-dhampir.mjs")),
            "发布包必须先找 <程序目录>/runtime：{:?}",
            pkg.first()
        );
        // ⚠️ **老位必须仍在候选里**（存量安装升级时 `<程序目录>/tools/` 那份还在）——
        // 少了它，装过老版本的机器升级后**找不到转译器、出片直接报错**。
        assert!(
            pkg.contains(&PathBuf::from("C:/app/tools/polish-to-dhampir.mjs")),
            "老位置 <程序目录>/tools 必须留作回落：{pkg:?}"
        );
        // ⚠️ **新位必须排在新老并存时的前面**：两个都在 = 用户手动拷过一份，
        // "用哪一份"要可预期（这条曾经踩过：旧副本排第一、静默生效四天）。
        let new_idx = pkg
            .iter()
            .position(|p| p == Path::new("C:/app/runtime/polish-to-dhampir.mjs"));
        let old_idx = pkg
            .iter()
            .position(|p| p == Path::new("C:/app/tools/polish-to-dhampir.mjs"));
        assert!(
            new_idx < old_idx,
            "runtime/ 必须排在 tools/ 之前（新位优先）：{pkg:?}"
        );
    }
}
