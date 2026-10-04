//! 把 `webui/dist` **编进二进制**（release 的默认来源）。
//!
//! ## 为什么
//!
//! 原先发布包要带 `webui/dist/`（30 个文件、1.2 MB），而服务端是**运行时读盘**的
//! （`web.rs` 的四级候选目录）。于是"只换 exe"会**静默**端出旧界面 —— 查找链会老老实实命中
//! exe 旁那份旧 dist，看不出任何异常。编进二进制后这个组合在物理上不存在。
//!
//! 字体早就是这个套路（`include_bytes!`，`xtask package` 甚至会把包里那份重复的删掉，
//! 理由写着"同样的字节已经编在 exe 里了"）。这里只是把同一个做法用到 dist 上，**不引新依赖**。
//!
//! ## 新鲜度：过期就**编译失败**
//!
//! 这比旧流程**强**：旧流程只在 `cargo xtask package` 时检查 dist 是否比 src 新，
//! 而 `cargo build --release` 出来的 exe 照样可能内嵌/配套一份过期界面。
//! 现在 `dist` 缺失、或比 `src` 里最新的文件旧，编译直接失败 ——
//! 「测的就是发的」这条因此真的成立。
//!
//! ⚠️ 生成的是 `include_bytes!(<绝对路径>)` 字面量（经 `{:?}` 转义），所以生成物不依赖
//! build.rs 的 cwd，也不受路径里反斜杠/空格/非 ASCII 的影响。

use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo 一定会给这个变量"));
    // crates/yekitrim-server → crates → 仓库根
    let root = manifest
        .parent()
        .and_then(Path::parent)
        .expect("本 crate 在 crates/<name> 下，往上两级就是仓库根")
        .to_path_buf();
    let dist = root.join("webui").join("dist");
    let src = root.join("webui").join("src");

    // 源侧任何改动都重跑（虽然 dist 才是判据，但这样"改了 src 忘了 build"能立刻反映出来）
    println!("cargo:rerun-if-changed={}", src.display());
    println!("cargo:rerun-if-changed={}", dist.display());

    let index = dist.join("index.html");
    if !index.is_file() {
        panic!(
            "找不到 WebUI 构建产物：{}\n\
             它在**编译期**被 include_bytes! 编进二进制（release 的默认来源），所以必须先构建：\n\
             \x20   npm --prefix webui run build\n\
             （`npm run dev` 的 vite dev server 不需要它 —— 那条路前端根本不走服务端。）",
            index.display()
        );
    }

    // 新鲜度：**比内容指纹，不比 mtime**。
    //
    // ⚠️ 为什么不能用 mtime：全新 `git clone` 时所有文件的 mtime 都是检出那一刻、逐个差几毫秒，
    // 于是"dist 比 src 旧"会**随机误报** —— 别人的第一次构建直接被这条打死（Linux 拉取必踩）。
    // 指纹由前端构建写进 `dist/.built-from`（`webui/vite.config.js` 里那个插件），
    // 算法与下面的 `src_fingerprint` 必须逐字节一致；**不一致会立刻在这里报错**（也是交叉校验）。
    let want = src_fingerprint(&src);
    let marker = dist.join(".built-from");
    match read_marker(&marker) {
        Some(got) if got != want => panic!(
            "webui/dist 不是从当前的 webui/src 构建出来的 —— 内嵌进去的会是**过期界面**。\n\
             \x20 dist 记录的 src 指纹: {got}\n\
             \x20 当前 src 的指纹:      {want}\n\
             先 `npm --prefix webui run build` 再编译。"
        ),
        Some(_) => {} // 一致
        None => println!(
            "cargo:warning={} 里没有 src 指纹（旧版 dist？），无法判断界面是否过期 —— 继续编译。\n\
             cargo:warning=要恢复这道检查：`npm --prefix webui run build` 重建一次 dist。",
            marker.display()
        ),
    }

    let mut files: Vec<(String, PathBuf)> = Vec::new();
    collect(&dist, &dist, &mut files)
        .unwrap_or_else(|e| panic!("遍历 {} 失败：{e}", dist.display()));
    // 路径排序：让生成物在 dist 内容不变时**逐字节稳定**（否则每次重编都改 out_dir，
    // 会让 release 的链接缓存白失效）。
    files.sort();

    let mut out = String::new();
    out.push_str("// 由 crates/yekitrim-server/build.rs 生成 —— 别手改。\n");
    out.push_str("/// 编进二进制的 WebUI 资源：");
    out.push_str(&format!("{} 个文件（键是相对 `webui/dist` 的 `/` 分隔路径）。\n", files.len()));
    out.push_str("pub static WEBUI_DIST: &[(&str, &[u8])] = &[\n");
    for (rel, abs) in &files {
        // `{:?}` 会把反斜杠 / 引号 / 非 ASCII 都转义成合法的 Rust 字面量
        out.push_str(&format!("    ({:?}, include_bytes!({:?})),\n", rel, abs));
    }
    out.push_str("];\n");

    let dest = PathBuf::from(env::var("OUT_DIR").expect("cargo 一定会给 OUT_DIR"))
        .join("webui_dist.rs");
    fs::write(&dest, out).unwrap_or_else(|e| panic!("写 {} 失败：{e}", dest.display()));
}

/// 递归收集 `dir` 下的普通文件，键统一用 `/` 分隔（与 HTTP 路径同形，跨平台一致）。
fn collect(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        // 点文件不进内嵌表（`.built-from` 是构建元数据，没有理由端给浏览器）
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect(base, &path, out)?;
        } else if path.is_file() {
            let rel = path
                .strip_prefix(base)
                .expect("collect 只在 base 之下走")
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push((rel, path));
        }
    }
    Ok(())
}

/// `src/` 的内容指纹：FNV-1a 64，喂入的流 = 按 **UTF-8 字节序**排序的每个文件的
/// `相对路径 + NUL + 内容 + NUL`（键用 `/` 分隔）。
///
/// ⚠️ 与 `webui/vite.config.js` 的 `srcFingerprint()` **必须逐字节一致**（含排序口径 ——
/// JS 的 `localeCompare` 跟语言有关，那边刻意用了 `Buffer.compare`）。
fn src_fingerprint(dir: &Path) -> String {
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    collect(dir, dir, &mut files).unwrap_or_else(|e| panic!("遍历 {} 失败：{e}", dir.display()));
    // 逐字节排序（`String` 的比较就是字节序；`/` 是 ASCII，两种语言一致）
    files.sort();
    let mut stream: Vec<u8> = Vec::new();
    for (rel, abs) in &files {
        stream.extend_from_slice(rel.as_bytes());
        stream.push(0);
        match fs::read(abs) {
            Ok(bytes) => stream.extend_from_slice(&bytes),
            Err(e) => panic!("读 {} 失败：{e}", abs.display()),
        }
        stream.push(0);
    }
    format!("{:016x}", fnv1a64(&stream))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 读 `dist/.built-from` 里的 `src_fnv1a64 = <16 hex>`；文件不在或没这行 = `None`。
fn read_marker(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    text.lines()
        .find_map(|l| l.trim().strip_prefix("src_fnv1a64"))
        .and_then(|rest| rest.split('=').nth(1))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

