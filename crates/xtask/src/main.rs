//! YekiTrim repo tasks. Run through the cargo alias in `.cargo/config.toml`:
//!
//! ```text
//! cargo xtask scan                     # product gate: the exe must carry no espeak-ng
//! cargo xtask package [--smoke-test]   # build -> gate -> assemble -> zip
//! ```
//!
//! This replaced scripts/package-release.ps1 and scripts/scan-artifact.ps1 so the whole build
//! and release flow runs on one toolchain, with no PowerShell, no tar/7z/bzip2/python, and the
//! ASCII-only / BOM / `$LASTEXITCODE` quirks that came with them.
//!
//! Provisioning the TTS-off sherpa-onnx archive is NOT here -- that lives in the vendored
//! sherpa-onnx-sys build script (see vendor/sherpa-onnx-sys/FORK-NOTES.md), which is the one
//! place a build cannot route around. This binary only gates and packages what that produced.

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

/// Markers specific to espeak-ng. Deliberately NOT the bare substring "espeak": sherpa-onnx's
/// speaker diarization has a class named `OfflineSpeaker`, which contains "eSpeaker" and matches
/// "espeak" under a case-insensitive search -- and so does `discreteSpeaker`, which is what the
/// real binary actually retains. Pass --show-bare-espeak to see that misleading count.
const MARKERS: &[&str] = &[
    "espeak-ng",
    "espeak ng",
    "espeak_",
    "libespeak",
    "piper_phonemize",
];

/// Not release payload, and they carry cargo's own string tables (sherpa-onnx-sys's build-script
/// binary embeds its lib-name list verbatim), which produces false positives if scanned.
const SKIP_DIRS: &[&str] = &["build", "deps"];

const SCAN_EXTENSIONS: &[&str] = &["exe", "dll"];

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("scan") => cmd_scan(&args[1..]),
        Some("package") => cmd_package(&args[1..]),
        Some("help") | Some("-h") | Some("--help") | None => {
            usage();
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("xtask: unknown subcommand '{other}'\n");
            usage();
            ExitCode::from(2)
        }
    }
}

fn usage() {
    println!(
        "\
usage: cargo xtask <command> [options]

commands:
  scan [path] [--show-bare-espeak]
      Fail if a built binary links espeak-ng. `path` defaults to target/release.
      The scanner's own binary is a dev tool, not payload, and is skipped.
      Exit code 0 = clean, 1 = markers found, 3 = nothing was scanned.

  package [--skip-build] [--smoke-test] [--version V] [--out-dir DIR]
      Build release, run the scan gate, assemble the Windows x64 package, zip it.
      Output goes to <repo>/release/ (gitignored).
      --smoke-test also boots the packaged exe and checks /api/health and the WebUI."
    );
}

// ---------------------------------------------------------------------------------------------
// scan
// ---------------------------------------------------------------------------------------------

fn cmd_scan(args: &[String]) -> ExitCode {
    let mut path = repo_root().join("target").join("release");
    let mut show_bare = false;

    for arg in args {
        match arg.as_str() {
            "--show-bare-espeak" => show_bare = true,
            other if other.starts_with('-') => {
                eprintln!("xtask scan: unknown flag {other}");
                return ExitCode::from(2);
            }
            other => path = PathBuf::from(other),
        }
    }

    let artifacts = match collect_artifacts(&path) {
        Ok(artifacts) => artifacts,
        Err(err) => {
            eprintln!("xtask scan: cannot read {}: {err}", path.display());
            return ExitCode::from(3);
        }
    };

    if artifacts.found.is_empty() {
        for skipped in &artifacts.skipped_self {
            eprintln!(
                "note: skipped {} (this tool's own binary)",
                skipped.display()
            );
        }
        eprintln!("xtask scan: nothing to scan at {}", path.display());
        eprintln!("That is itself a gate failure: it usually means the build never ran.");
        return ExitCode::from(3);
    }

    let mut total_bytes: u64 = 0;
    let mut per_marker = vec![0usize; MARKERS.len()];
    let mut bare = 0usize;
    let mut offenders: Vec<(PathBuf, usize)> = Vec::new();

    for file in &artifacts.found {
        let bytes = match fs::read(file) {
            Ok(bytes) => bytes,
            Err(err) => {
                eprintln!("xtask scan: cannot read {}: {err}", file.display());
                return ExitCode::from(3);
            }
        };
        total_bytes += bytes.len() as u64;

        let lowered = to_ascii_lowercase(&bytes);
        let mut hits = 0usize;
        for (index, marker) in MARKERS.iter().enumerate() {
            let count = count_marker(&lowered, marker);
            per_marker[index] += count;
            hits += count;
        }
        if show_bare {
            bare += count_marker(&lowered, "espeak");
        }
        if hits > 0 {
            offenders.push((file.clone(), hits));
        }
    }

    println!("scanned: {}", path.display());
    println!(
        "files:   {}, {} MB",
        artifacts.found.len(),
        total_bytes / (1024 * 1024)
    );
    for skipped in &artifacts.skipped_self {
        println!(
            "skip:    {} (this tool's own binary, not release payload)",
            skipped.display()
        );
    }
    println!("markers: {}", MARKERS.join(", "));
    println!();

    for (index, marker) in MARKERS.iter().enumerate() {
        let count = per_marker[index];
        if count == 0 {
            println!("  ok      {:<18} 0", marker);
        } else {
            println!("  HIT     {:<18} {}", marker, count);
        }
    }
    if show_bare {
        println!();
        println!("  bare \"espeak\" (diagnostic only): {bare}");
        println!("  Speaker-diarization identifiers (OfflineSpeaker / discreteSpeaker) match this");
        println!("  and are not espeak-ng. It never affects the exit code.");
    }

    let total: usize = per_marker.iter().sum();
    if total == 0 {
        println!();
        println!("PASS: no espeak-ng markers found");
        return ExitCode::SUCCESS;
    }

    println!();
    for (file, hits) in &offenders {
        println!("  {}  [{hits} hits]", file.display());
    }
    println!("FAIL: {total} espeak-ng marker(s) found");
    ExitCode::from(1)
}

struct Artifacts {
    found: Vec<PathBuf>,
    /// The scanner's own binaries, reported but never counted. See `is_self_artifact`.
    skipped_self: Vec<PathBuf>,
}

fn collect_artifacts(path: &Path) -> io::Result<Artifacts> {
    if !path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} does not exist", path.display()),
        ));
    }

    let mut artifacts = Artifacts {
        found: Vec::new(),
        skipped_self: Vec::new(),
    };
    if path.is_file() {
        artifacts.found.push(path.to_path_buf());
        return Ok(artifacts);
    }

    collect_dir(path, &mut artifacts)?;
    artifacts.found.sort();
    artifacts.skipped_self.sort();
    Ok(artifacts)
}

/// Both profiles build this crate, so the scanner meets its own binary whenever it walks
/// `target/<profile>`. That binary is a dev tool, not release payload, and it carries MARKERS
/// verbatim as string literals -- scanning it is a guaranteed false positive. Skipping by name
/// cannot hide a real offender: the package gate scans `target/release/yekitrim.exe` explicitly.
///
/// An explicit single-file argument is always scanned as given; this only filters directory walks.
fn is_self_artifact(path: &Path) -> bool {
    path.file_stem()
        .and_then(OsStr::to_str)
        .map(|stem| stem.eq_ignore_ascii_case("xtask"))
        .unwrap_or(false)
}

fn collect_dir(dir: &Path, artifacts: &mut Artifacts) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            let skip = path
                .file_name()
                .and_then(OsStr::to_str)
                .map(|name| SKIP_DIRS.contains(&name))
                .unwrap_or(false);
            if !skip {
                collect_dir(&path, artifacts)?;
            }
            continue;
        }

        let extension = path
            .extension()
            .and_then(OsStr::to_str)
            .map(|ext| ext.to_ascii_lowercase());
        if SCAN_EXTENSIONS
            .iter()
            .any(|wanted| extension.as_deref() == Some(wanted))
        {
            if is_self_artifact(&path) {
                artifacts.skipped_self.push(path);
            } else {
                artifacts.found.push(path);
            }
        }
    }
    Ok(())
}

fn to_ascii_lowercase(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().map(|b| b.to_ascii_lowercase()).collect()
}

/// Non-overlapping occurrences, matching the semantics of the PowerShell scanner this replaced
/// (it advanced `indexOf` past each match, and used a regex split for the same reason).
fn count_marker(haystack: &[u8], needle: &str) -> usize {
    let needle = needle.as_bytes();
    if needle.is_empty() || haystack.len() < needle.len() {
        return 0;
    }

    let mut count = 0usize;
    let mut index = 0usize;
    while index + needle.len() <= haystack.len() {
        if &haystack[index..index + needle.len()] == needle {
            count += 1;
            index += needle.len();
        } else {
            index += 1;
        }
    }
    count
}

// ---------------------------------------------------------------------------------------------
// package
// ---------------------------------------------------------------------------------------------

struct PackageOptions {
    version: Option<String>,
    out_dir: Option<PathBuf>,
    skip_build: bool,
    smoke_test: bool,
    /// Dhampir 预览产物目录（含 `preview/` 与 `VERSION`）。默认找同层 `../Dhampir/out`。
    dhampir_artifact: Option<PathBuf>,
    /// Dhampir 出片二进制。默认找同层 `../Dhampir/target/release/dhampir.exe`。
    dhampir_exe: Option<PathBuf>,
    /// 显式**不打**底座：默认通道在目标机器上就不存在（要用户自己装）。
    /// 不写这个开关而找不到底座 ⇒ **直接拒绝打包**（"默认出片通道"是产品的主路径，不能悄悄没有）。
    no_dhampir: bool,
}

fn cmd_package(args: &[String]) -> ExitCode {
    let mut options = PackageOptions {
        version: None,
        out_dir: None,
        skip_build: false,
        smoke_test: false,
        dhampir_artifact: None,
        dhampir_exe: None,
        no_dhampir: false,
    };

    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--skip-build" => options.skip_build = true,
            "--smoke-test" => options.smoke_test = true,
            "--no-dhampir" => options.no_dhampir = true,
            "--dhampir-artifact" => {
                index += 1;
                match args.get(index) {
                    Some(value) => options.dhampir_artifact = Some(PathBuf::from(value)),
                    None => {
                        eprintln!("xtask package: --dhampir-artifact needs a value");
                        return ExitCode::from(2);
                    }
                }
            }
            "--dhampir-exe" => {
                index += 1;
                match args.get(index) {
                    Some(value) => options.dhampir_exe = Some(PathBuf::from(value)),
                    None => {
                        eprintln!("xtask package: --dhampir-exe needs a value");
                        return ExitCode::from(2);
                    }
                }
            }
            "--version" => {
                index += 1;
                match args.get(index) {
                    Some(value) => options.version = Some(value.clone()),
                    None => {
                        eprintln!("xtask package: --version needs a value");
                        return ExitCode::from(2);
                    }
                }
            }
            "--out-dir" => {
                index += 1;
                match args.get(index) {
                    Some(value) => options.out_dir = Some(PathBuf::from(value)),
                    None => {
                        eprintln!("xtask package: --out-dir needs a value");
                        return ExitCode::from(2);
                    }
                }
            }
            other => {
                eprintln!("xtask package: unknown option {other}");
                return ExitCode::from(2);
            }
        }
        index += 1;
    }

    match run_package(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!();
            eprintln!("xtask package failed: {err}");
            ExitCode::from(1)
        }
    }
}

fn run_package(options: &PackageOptions) -> Result<(), Box<dyn std::error::Error>> {
    let root = repo_root();

    let version = match &options.version {
        Some(version) => version.clone(),
        None => cli_version(&root)?,
    };
    println!("==> version: {version}");

    let out_dir = options
        .out_dir
        .clone()
        .unwrap_or_else(|| root.join("release"));
    let package_name = format!("yekitrim-v{version}-win-x64");
    let package_dir = out_dir.join(&package_name);
    let zip_path = out_dir.join(format!("{package_name}.zip"));

    if !options.skip_build {
        println!("==> cargo build --release -p yekitrim-cli --locked");
        let status = Command::new(cargo())
            .current_dir(&root)
            .args(["build", "--release", "-p", "yekitrim-cli", "--locked"])
            .status()?;
        if !status.success() {
            return Err(format!("cargo build failed ({status})").into());
        }
    }

    let exe = root.join("target").join("release").join("yekitrim.exe");
    if !exe.is_file() {
        return Err(format!("missing {}", exe.display()).into());
    }
    // Windows 托盘外壳（双击即后台起 server）。非 Windows 上这个 bin 也会编出来（占位 main），
    // 所以这里不 cfg 分支：有就带上，没有就只是少了托盘这个入口。
    let tray_exe = root
        .join("target")
        .join("release")
        .join("yekitrim-tray.exe");

    // Gate. Mandatory and unskippable, for the same reason it was unskippable in the script it
    // replaced: linking espeak-ng turns the shipped binary into a GPL combined work, and the
    // failure prints nothing at all. (The vendored build script already refuses to link a lib set
    // that is not the pinned TTS-off one -- this checks the artifact that actually came out.)
    println!("==> asserting the binary carries no espeak-ng");
    let scan = Command::new(env::current_exe()?)
        .args(["scan", exe.to_string_lossy().as_ref()])
        .status()?;
    if !scan.success() {
        return Err(
            "the product gate rejected target/release/yekitrim.exe. Refusing to package a binary \
             that links espeak-ng."
                .into(),
        );
    }
    // 托盘是**另一个 bin**，但依赖图与主程序基本同源 ⇒ 同一道门禁照样要过。
    if tray_exe.is_file() {
        let scan = Command::new(env::current_exe()?)
            .args(["scan", tray_exe.to_string_lossy().as_ref()])
            .status()?;
        if !scan.success() {
            return Err(
                "the product gate rejected target/release/yekitrim-tray.exe. Refusing to package \
                 a binary that links espeak-ng."
                    .into(),
            );
        }
    }

    // The WebUI ships only from webui/dist, so a stale dist ships a stale UI.
    let dist = root.join("webui").join("dist");
    if !dist.join("index.html").is_file() {
        return Err("webui/dist is missing: run 'npm --prefix webui run build' first".into());
    }
    if let (Some(newest_src), Ok(dist_mtime)) = (
        newest_mtime(&root.join("webui").join("src"))?,
        fs::metadata(dist.join("index.html")).and_then(|m| m.modified()),
    ) {
        if newest_src > dist_mtime + Duration::from_secs(1) {
            println!(
                "warning: webui/src is newer than webui/dist -- run 'npm --prefix webui run build'"
            );
        }
    }

    println!("==> assembling {package_name}");
    if package_dir.exists() {
        fs::remove_dir_all(&package_dir)?;
    }
    fs::create_dir_all(package_dir.join("webui"))?;
    fs::create_dir_all(package_dir.join("resources").join("fonts"))?;

    fs::copy(&exe, package_dir.join("yekitrim.exe"))?;
    if tray_exe.is_file() {
        fs::copy(&tray_exe, package_dir.join("yekitrim-tray.exe"))?;
    }
    // 托盘图标：`yekitrim-tray` 运行时从 **exe 同级** 读 `yekitrim.ico`（见 tray.rs::load_icon），
    // 缺了也能跑（退回系统图标），但那样托盘里就是个通用图标。
    let ico = root
        .join("crates")
        .join("yekitrim-cli")
        .join("assets")
        .join("yekitrim.ico");
    if ico.is_file() {
        fs::copy(&ico, package_dir.join("yekitrim.ico"))?;
    }
    copy_dir_all(&dist, &package_dir.join("webui").join("dist"))?;

    // ---- 默认出片通道的随身件（2026-10-01）----
    //
    // 底座那条腿要三样东西，缺一样"默认通道"在目标机器上就起不来：
    //   ① 转译器 `tools/polish-to-dhampir.mjs`（它按**同层** `../spec/effects.json` 读效果词汇表）
    //   ② 效果词汇表 `spec/effects.json`
    //   ③ 底座自己：`dhampir/bin/<exe>`（出片）+ `dhampir/preview/{engine.js,pkg/*}` + `dhampir/VERSION`
    //      —— 包内形状由 `yekitrim_polish::render::dhampir::{find_binary, wasm_dirs, preview_dir}` 定义
    //      （`<程序目录>/dhampir/…`），底座侧的产出来自它自己的 `node scripts/package.mjs`。
    fs::create_dir_all(package_dir.join("tools"))?;
    fs::create_dir_all(package_dir.join("spec"))?;
    let translator = root.join("tools").join("polish-to-dhampir.mjs");
    if !translator.is_file() {
        return Err(format!("missing translator at {}", translator.display()).into());
    }
    fs::copy(
        &translator,
        package_dir.join("tools").join("polish-to-dhampir.mjs"),
    )?;
    let vocab = root.join("spec").join("effects.json");
    if !vocab.is_file() {
        return Err(format!("missing effect vocabulary at {}", vocab.display()).into());
    }
    fs::copy(&vocab, package_dir.join("spec").join("effects.json"))?;

    if options.no_dhampir {
        println!("warning: --no-dhampir：包里**不带底座** —— 目标机器上默认出片通道不可用");
        println!(
            "         （用户需自行放置 <程序目录>/dhampir/…；出片只有这一条腿，没有可退的第二条）"
        );
    } else {
        // 同层目录规则与 `find_binary` 一致：`YekiTrim` 与 `Dhampir` 并排时才好找。
        let sibling = root.parent().map(|p| p.join("Dhampir"));
        let artifact = options
            .dhampir_artifact
            .clone()
            // ⚠️ 2026-10-01：底座 `scripts/package.mjs` 的默认输出是 **`dist/dhampir-<版本>+<sha>`**，
            // 不是 `out/` —— 我们一直按 `out/` 找，于是"底座明明打好了"却报"产物不齐"。
            // 现在：先取 `dist/dhampir-*` 里**最新**的一份，再退回 `out/`（老布局）。
            .or_else(|| {
                sibling
                    .as_ref()
                    .and_then(|d| newest_dhampir_dist(d).or_else(|| Some(d.join("out"))))
            })
            .ok_or("no sibling Dhampir checkout: pass --dhampir-artifact <dir> or --no-dhampir")?;
        let base_exe = options.dhampir_exe.clone().unwrap_or_else(|| {
            sibling
                .as_ref()
                .map(|d| d.join("target").join("release").join("dhampir.exe"))
                .unwrap_or_else(|| PathBuf::from("dhampir.exe"))
        });
        for required in [
            artifact.join("preview").join("engine.js"),
            artifact.join("preview").join("pkg").join("dhampir_wasm.js"),
            artifact
                .join("preview")
                .join("pkg")
                .join("dhampir_wasm_bg.wasm"),
            artifact.join("VERSION"),
        ] {
            if !required.is_file() {
                return Err(format!(
                    "Dhampir 产物不齐：{} 不存在。\n  \
                     在 Dhampir 仓跑 `node scripts/package.mjs` 生成产物，或用 --dhampir-artifact <dir> 指定；\
                     确实要打成「没有底座」的包就加 --no-dhampir",
                    required.display()
                )
                .into());
            }
        }
        if !base_exe.is_file() {
            return Err(format!(
                "找不到 Dhampir 出片二进制 {}（用 --dhampir-exe <path> 指定，或 --no-dhampir 明确不带）",
                base_exe.display()
            )
            .into());
        }
        copy_dir_all(
            &artifact.join("preview"),
            &package_dir.join("dhampir").join("preview"),
        )?;
        fs::copy(
            &artifact.join("VERSION"),
            package_dir.join("dhampir").join("VERSION"),
        )?;
        fs::create_dir_all(package_dir.join("dhampir").join("bin"))?;
        let base_name = if cfg!(windows) {
            "dhampir.exe"
        } else {
            "dhampir"
        };
        fs::copy(
            &base_exe,
            package_dir.join("dhampir").join("bin").join(base_name),
        )?;
        println!("    + dhampir/（出片二进制 + 预览产物 + VERSION）");
    }
    println!("    + tools/polish-to-dhampir.mjs + spec/effects.json（转译器与效果词汇表）");

    // ⚠️ 2026-10-02 起：自带字体**内嵌在 yekitrim.exe 里**（`crates/yekitrim-core/assets/fonts/`，
    // 经 include_bytes! 进二进制），首次启动自动落盘到字体库 —— 所以**发行包仍然只带一个空目录**，
    // 但用户侧已经不需要任何"下载"动作。内嵌的授权依据 = 两支都是 SIL OFL-1.1（正文明确允许
    // bundled/embedded）；许可原文也一并内嵌、随字体落盘。
    // 用户自己导入的字体放这里（`yekitrim_core::fonts`，v2 = 一支一个目录 + FONT.toml）。
    // ⚠️ **本目录就是字体库本体**（2026-10-02 资产收回程序目录之后）。
    // 曾经它只是"老安装的回落态"，正式位置在 `%USERPROFILE%\.yekitrim\fonts` ——
    // 那条已经作废：现在读写都走这里，**别再改回用户目录**。
    fs::write(
        package_dir
            .join("resources")
            .join("fonts")
            .join("放这里.txt"),
        concat!(
            "【本目录就是字体库本体 —— 把 .ttf / .otf 直接放进这里即可。\n",
            "（2026-10-02 起：资产全部围绕程序目录。以前正式位置在\n",
            "  %USERPROFILE%\\.yekitrim\\fonts，那条已作废，程序不再读它。）】\n",
            "把字体文件（.ttf / .otf）放进字体库，精修页里就会多出对应的字体可选：\n",
            "\n",
            "  - 字幕字体 =「生成与导出」卡 → LLM 剪辑参数块 → 字幕字体\n",
            "  - 封面字体 = 封面设计器 →「排版」组 → 字体\n",
            "\n",
            "⚠️ .ttc / .otc（字体集合）用不了：浏览器的 @font-face 不认集合，\n",
            "   收进来会出现「预览回退系统字体、出片却用了它」。请先拆包成单个 .ttf/.otf。\n",
            "\n",
            "自带字体（Noto Sans SC + Noto Color Emoji，均 OFL-1.1，可商用/可嵌入/可再分发）\n",
            "已内嵌在程序里，首次启动会自动装进字体库 —— 不需要联网、也不需要手动下载。\n",
            "  yekitrim fonts list       看看库里有什么\n",
            "  yekitrim fonts install    万一被删了，重新装回来（幂等）\n",
            "\n",
            "其余字体请自行获取并遵守其许可（见 README「字体」）。\n",
        ),
    )?;

    let notes = root
        .join("docs")
        .join("releases")
        .join(format!("v{version}.md"));
    if notes.is_file() {
        fs::copy(&notes, package_dir.join("RELEASE-NOTES.md"))?;
    } else {
        println!("warning: release notes not found: {}", notes.display());
    }

    // Licence material. The shipped exe statically links third-party code that is almost entirely
    // MIT / Apache-2.0 / BSD / ISC / Zlib, and every one of those requires the copyright notice and
    // licence text to travel with the binary; Apache-2.0 additionally requires the licence text and
    // any NOTICE file. Shipping *no* licence material is the one option that is certainly wrong, so
    // these are hard requirements rather than best-effort copies -- and note this is independent of
    // what YekiTrim's own licence ends up being.
    for asset in ["LICENSE", "THIRD-PARTY-NOTICES.md"] {
        let source = root.join(asset);
        if !source.is_file() {
            return Err(format!(
                "missing {asset} at the repo root: it must ship alongside the binary"
            )
            .into());
        }
        fs::copy(&source, package_dir.join(asset))?;
    }

    for required in [
        "yekitrim.exe",
        "webui/dist/index.html",
        "LICENSE",
        "THIRD-PARTY-NOTICES.md",
    ] {
        if !package_dir.join(required).exists() {
            return Err(format!("package is missing {required}").into());
        }
    }

    if options.smoke_test {
        smoke_test(&package_dir)?;
    }

    println!("==> compressing {}", zip_path.display());
    if zip_path.exists() {
        fs::remove_file(&zip_path)?;
    }
    zip_dir(&package_dir, &zip_path)?;

    let size_mb = fs::metadata(&zip_path)?.len() as f64 / (1024.0 * 1024.0);
    let size_bytes = fs::metadata(&zip_path)?.len();
    let digest = sha256_of_file(&zip_path)?;
    println!();
    println!("done: {}", zip_path.display());
    println!("  size:   {size_mb:.1} MB");
    println!("  sha256: {digest}");

    // ⚠️ **校验和文件也必须每次重新生成**（2026-10-01 修）。
    //
    // 这个旁路文件以前只在手工发版时写：于是 `release/yekitrim-v0.2.1-win64.zip.sha256.txt`
    // 从 2026-09-22 起就没再更新过，而 zip 早换过好几版 —— **一个撒谎的校验和比没有更糟**
    // （用户会照它校验，然后以为下载坏了）。现在它在 `package` 里跟着 zip 一起写。
    let sidecar = write_checksum_sidecar(&zip_path, &digest, size_bytes, options.smoke_test)?;
    println!("  checksum: {}", sidecar.display());

    Ok(())
}

/// 把 zip 的校验和写成**每次都重新生成**的旁路文件（放在发行包**之外**，避免自指）。
/// 底座新的默认产物目录：`<dhampir>/dist/dhampir-<版本>+<sha>/`，取**最新**的一份。
///
/// 为什么要有它：底座 `scripts/package.mjs` 的默认输出改成了 `dist/`，而我们的打包器一直按 `out/` 找，
/// 症状是"底座明明打好了，这边却报产物不齐"（2026-10-01 踩到）。两份都认：这里给 dist 里最新的那份，
/// 调用方拿不到时再退回 `out/`。
fn newest_dhampir_dist(root: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(root.join("dist"))
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("dhampir-"))
        })
        .collect();
    found.sort_by_key(|p| fs::metadata(p).and_then(|m| m.modified()).ok());
    found.pop()
}

fn write_checksum_sidecar(
    zip_path: &Path,
    digest: &str,
    size_bytes: u64,
    smoke_tested: bool,
) -> io::Result<PathBuf> {
    let path = PathBuf::from(format!("{}.sha256.txt", zip_path.display()));
    let name = zip_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let mb = size_bytes as f64 / (1024.0 * 1024.0);
    let built = fmt_utc_date(SystemTime::now());
    let how = if smoke_tested {
        "cargo xtask package --smoke-test"
    } else {
        "cargo xtask package"
    };
    let text = format!(
        "{name}\n\
         sha256  {digest}\n\
         size    {size_bytes} bytes ({mb:.1} MB)\n\
         built   {built}（Windows x64；{how}）\n\
         \n\
         校验：\n  \
         certutil -hashfile {name} SHA256\n  \
         (Get-FileHash .\\{name} -Algorithm SHA256).Hash\n\
         \n\
         说明：本文件由 `cargo xtask package` **每次重新生成**（别手工改）；\n\
         刻意放在发行包**之外** —— RELEASE-NOTES.md 在 zip 内，把校验和写进去会形成自指。\n"
    );
    fs::write(&path, text)?;
    Ok(path)
}

/// UTC 日期（YYYY-MM-DD）。xtask 刻意不带 chrono（依赖只有 std + sha2 + zip），
/// 这里用 Howard Hinnant 的 civil_from_days（纯整数运算）。
fn fmt_utc_date(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn smoke_test(package_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    println!("==> smoke test: boot the packaged exe and verify");
    let exe = package_dir.join("yekitrim.exe");
    let port =
        18_000 + (SystemTime::now().duration_since(UNIX_EPOCH)?.subsec_nanos() % 1000) as u16;

    let mut command = Command::new(&exe);
    command.args(["server", "--port", &port.to_string()]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = command.spawn()?;
    let mut ok = false;

    for _ in 0..20 {
        sleep(Duration::from_millis(500));
        let health = http_get(port, "/api/health", Duration::from_secs(3));
        let index = http_get(port, "/", Duration::from_secs(5));
        if let (Ok(health), Ok(index)) = (health, index) {
            if health.contains("\"ok\"")
                && health.contains("true")
                && index.contains(" 200 ")
                && index.contains("<div id=\"app\">")
            {
                println!("    /api/health ok, / served the packaged WebUI");
                ok = true;
                break;
            }
        }
    }

    let _ = child.kill();
    let _ = child.wait();

    // First run creates .yekitrim / stickers next to the exe; runtime state must not ship.
    for generated in [".yekitrim", "stickers"] {
        let path = package_dir.join(generated);
        if path.exists() {
            fs::remove_dir_all(&path)?;
        }
    }

    // ⚠️ **内嵌字体也会被这次启动解包进包目录，必须一并清掉**（2026-10-02 实际踩到）。
    //
    // 机制：`fonts::ensure_dir` 写的**就是** `<数据目录>/resources/fonts`，而烟测起的 exe
    // 就在包目录里（`exe_dir()` = 包目录）—— 于是两个自带字体（合计 **约 21 MB**）被解包进
    // `resources/fonts/<slug>/`，接着被原样压进 zip：**包从 37.2 MB 涨到 52.4 MB**。
    //
    // 这是**纯重复**：同样的字节已经通过 `include_bytes!` 编在 `yekitrim.exe` 里了
    // （exe 67.5 MB 里就含它们），包再带一份没有任何意义，还与发行说明
    // 「发行包仍然只带一个空目录」自相矛盾。
    //
    // 只删**字体目录里的子项**，保留 `放这里.txt` —— 那份说明是组装阶段写进去的、
    // 属于包的一部分（有判据要求它在），不能一起删掉。
    let fonts_dir = package_dir.join("resources").join("fonts");
    if fonts_dir.is_dir() {
        for entry in fs::read_dir(&fonts_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            if name == "放这里.txt" {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                fs::remove_dir_all(&path)?;
            } else {
                fs::remove_file(&path)?;
            }
        }
    }

    if !ok {
        return Err(
            "smoke test failed: the server did not return /api/health and the WebUI in \
                    time"
                .into(),
        );
    }
    Ok(())
}

/// Minimal HTTP/1.0 GET. The server is local and short-lived, so a hand-rolled request keeps this
/// tool free of an HTTP client dependency.
fn http_get(port: u16, path: &str, timeout: Duration) -> io::Result<String> {
    let address: SocketAddr = format!("127.0.0.1:{port}").parse().map_err(|err| {
        io::Error::new(io::ErrorKind::InvalidInput, format!("bad address: {err}"))
    })?;

    let mut stream = TcpStream::connect_timeout(&address, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    write!(
        stream,
        "GET {path} HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )?;
    stream.flush()?;

    let mut body = Vec::new();
    stream.read_to_end(&mut body)?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

fn zip_dir(source: &Path, destination: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let file = fs::File::create(destination)?;
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(9));
    let base = source.parent().unwrap_or(source).to_path_buf();

    add_dir_to_zip(&mut writer, source, &base, options)?;
    writer.finish()?;
    Ok(())
}

fn add_dir_to_zip<W: Write + io::Seek>(
    writer: &mut zip::ZipWriter<W>,
    dir: &Path,
    base: &Path,
    options: zip::write::SimpleFileOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = path
            .strip_prefix(base)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");

        if path.is_dir() {
            writer.add_directory(format!("{name}/"), options)?;
            add_dir_to_zip(writer, &path, base, options)?;
        } else {
            writer.start_file(name, options)?;
            let mut source = fs::File::open(&path)?;
            io::copy(&mut source, writer)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------------------------

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/xtask must live two levels below the repo root")
        .to_path_buf()
}

fn cargo() -> String {
    env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

/// Reads the package version from the **single source**: `[workspace.package] version`
/// in the root manifest.
///
/// Why root-first: the crates now say `version.workspace = true`, so the old per-crate
/// scan (`crates/yekitrim-cli/Cargo.toml` `[package] version`) finds nothing and packaging
/// would fail with "no version found". Keeping that scan as a fallback preserves old
/// checkouts. The version here is what names the release archive (`yekitrim-v<版本>-win-x64`),
/// so it must be read from the same place the binary's `CARGO_PKG_VERSION` comes from.
fn cli_version(root: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let root_manifest = root.join("Cargo.toml");
    if let Some(version) = table_version(&root_manifest, "[workspace.package]") {
        return Ok(version);
    }
    let manifest = root.join("crates").join("yekitrim-cli").join("Cargo.toml");
    if let Some(version) = table_version(&manifest, "[package]") {
        return Ok(version);
    }
    Err(format!(
        "no version found in {} ([workspace.package]) or {} ([package])",
        root_manifest.display(),
        manifest.display()
    )
    .into())
}

/// Naive scan for a literal `version = "…"` inside the named TOML table.
///
/// Deliberately not `version.workspace = true`-aware: an inherited version is not a
/// literal, and silently returning `"true"` as a version string would be worse than
/// falling through to the next source.
fn table_version(manifest: &Path, table: &str) -> Option<String> {
    let text = fs::read_to_string(manifest).ok()?;

    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == table;
            continue;
        }
        if !inside {
            continue;
        }
        if let Some(rest) = line.strip_prefix("version") {
            if let Some(rest) = rest.trim_start().strip_prefix('=') {
                let value = rest.trim().trim_matches('"');
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }

    None
}

fn newest_mtime(dir: &Path) -> io::Result<Option<SystemTime>> {
    if !dir.is_dir() {
        return Ok(None);
    }

    let mut newest: Option<SystemTime> = None;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let candidate = if path.is_dir() {
            newest_mtime(&path)?
        } else {
            fs::metadata(&path).and_then(|meta| meta.modified()).ok()
        };
        if let Some(candidate) = candidate {
            if newest.map(|current| candidate > current).unwrap_or(true) {
                newest = Some(candidate);
            }
        }
    }
    Ok(newest)
}

fn copy_dir_all(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn sha256_of_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
