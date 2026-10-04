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
    time::SystemTime,
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

    let newest_src = newest_mtime(&src).unwrap_or(SystemTime::UNIX_EPOCH);
    let dist_time = fs::metadata(&index)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    if dist_time < newest_src {
        panic!(
            "webui/dist 比 webui/src 旧 —— 内嵌进去的会是**过期界面**。\n\
             先 `npm --prefix webui run build` 再编译。\n\
             （这条检查原先只在 `cargo xtask package` 里，现在提前到编译期。）"
        );
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

/// `dir` 树下最新的修改时间（目录本身不算，只看文件）。
fn newest_mtime(dir: &Path) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let p = entry.path();
        let t = if p.is_dir() {
            newest_mtime(&p)
        } else {
            fs::metadata(&p).and_then(|m| m.modified()).ok()
        };
        if let Some(t) = t {
            newest = Some(match newest {
                Some(n) if n >= t => n,
                _ => t,
            });
        }
    }
    newest
}
