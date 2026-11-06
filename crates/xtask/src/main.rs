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
use std::process::{Command, ExitCode, Stdio};
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

/// ELF magic (`\x7fELF`)。
///
/// ⚠️ 2026-10-05：**Linux 产物没有扩展名**，所以旧的「按扩展名匹配」在 Linux 上
/// 一个文件都收不到 —— `cargo xtask scan` 直接以「nothing to scan」退出（exit 3）。
/// 那意味着**GPL 红线在 Linux 上根本没有被执行过**：一个链接了 espeak-ng 的 Linux
/// 二进制会**静默通过**。而这正是我们准备发布的那个平台。
///
/// 现在按**内容**认：扩展名命中 **或** 文件头是 ELF 魔数，都收。
/// 按内容判不会误伤 —— 它只决定"要不要扫"，扫到的标记才决定成败。
const ELF_MAGIC: &[u8] = b"\x7fELF";

/// 判断这个文件是不是"可能是可执行产物"，需要进 GPL 扫描。
///
/// 目录遍历时用。显式传单个文件时**不经过这里**（见 `run_scan`）—— 用户点名要扫的文件
/// 一律照扫，不做自我过滤、也不要求它是 ELF。
fn looks_like_artifact(path: &Path) -> bool {
    let extension = path
        .extension()
        .and_then(OsStr::to_str)
        .map(|ext| ext.to_ascii_lowercase());
    if SCAN_EXTENSIONS
        .iter()
        .any(|wanted| extension.as_deref() == Some(wanted))
    {
        return true;
    }

    // 无扩展名的 ELF（Linux 上的 `yekitrim`）。只读开头 4 字节，不整个读进来。
    let mut head = [0u8; 4];
    match fs::File::open(path).and_then(|mut f| io::Read::read_exact(&mut f, &mut head)) {
        Ok(()) => head == ELF_MAGIC,
        Err(_) => false,
    }
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

        if looks_like_artifact(&path) {
            if is_self_artifact(&path) {
                artifacts.skipped_self.push(path);
            } else {
                artifacts.found.push(path);
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
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

  package [--skip-build] [--smoke-test] [--version V] [--out-dir DIR] [--target windows|linux]
      Build release, run the mandatory espeak-ng gate, assemble the package, archive it.
      Output goes to <repo>/release/ (gitignored).
      --target defaults to the host OS.
        windows -> yekitrim-v<VERSION>-win-x64.zip      (exe + tray + .ico)
        linux   -> yekitrim-v<VERSION>-linux-x64.tar.gz (ELF; keeps the 0755 bit, which zip drops)
      The Linux path additionally refuses to package a binary whose GLIBC_* requirement is
      above the pinned ubuntu:24.04 baseline -- see docker/Dockerfile.linux-x64.
      --smoke-test also boots the packaged binary and checks /api/health and the WebUI."
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
    /// 目标平台：`windows`（zip / yekitrim.exe）或 `linux`（tar.gz / yekitrim）。
    /// 不给就按**当前宿主**推断 —— 本机是 Linux 就打 Linux 包。
    target: Option<String>,
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
        target: None,
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
            "--target" => {
                index += 1;
                match args.get(index) {
                    Some(value) => options.target = Some(value.clone()),
                    None => {
                        eprintln!("xtask package: --target needs a value (windows|linux)");
                        return ExitCode::from(2);
                    }
                }
            }
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

    // ---------------------------------------------------------------- 目标平台
    //
    // 2026-10-05 加 Linux 通道。以前 `package` 只会打 Windows 包（包名硬编码 win-x64、
    // 只找 `yekitrim.exe`、只产 zip）。Linux 侧要发就得有一条**同一套工具链**的路，
    // 而不是另写一套脚本 —— 那正是这个 crate 当初取代 PowerShell 的理由。
    //
    // ⚠️ Linux 产物**必须**在固定基线的容器里编（见 `docker/Dockerfile.linux-x64`）：
    // 在滚动发行版上直编会引用宿主才有的 glibc 符号，下游连启动都启动不了。
    // 这里不强制（本地调试也可能要打），但会打印实际测得的 glibc 上限提醒。
    let target_os = options.target.as_deref().unwrap_or(if cfg!(windows) {
        "windows"
    } else {
        "linux"
    });
    let is_linux = match target_os {
        "windows" => false,
        "linux" => true,
        other => {
            return Err(format!(
                "unknown --target '{other}' (expected 'windows' or 'linux')"
            )
            .into())
        }
    };

    let version = match &options.version {
        Some(version) => version.clone(),
        None => cli_version(&root)?,
    };
    println!("==> version: {version}");
    println!("==> target:  {target_os}");

    let out_dir = options
        .out_dir
        .clone()
        .unwrap_or_else(|| root.join("release"));
    let package_name = format!(
        "yekitrim-v{version}-{}",
        if is_linux { "linux-x64" } else { "win-x64" }
    );
    let package_dir = out_dir.join(&package_name);
    let zip_path = out_dir.join(format!(
        "{package_name}.{}",
        if is_linux { "tar.gz" } else { "zip" }
    ));

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

    if is_linux {
        return run_package_linux(options, &root, &version, &package_name, &package_dir, &zip_path);
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

    // WebUI 现在**编在 exe 里**（`crates/yekitrim-server/build.rs` 把 `webui/dist` 用
    // `include_bytes!` 嵌进去），所以包里**不再带** `webui/dist/`：
    //   · "只换 exe 就能升级" —— 前端跟着 exe 一起换，不存在"新 exe + 旧界面"这种错配；
    //   · 新鲜度检查也搬去了编译期（`dist` 比 `src` 旧会让**编译失败**），这里不必再查一遍。
    // ⚠️ 字体是同一个套路，所以包里 `resources/fonts/` 只留说明文件（见下面那段"纯重复"）。

    println!("==> assembling {package_name}");
    if package_dir.exists() {
        fs::remove_dir_all(&package_dir)?;
    }
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

    // ---- 默认出片通道的随身件（2026-10-01）----
    //
    // 底座那条腿要三样东西，缺一样"默认通道"在目标机器上就起不来：
    //   ① 转译器 `tools/polish-to-dhampir.mjs`（它按**同层** `../spec/effects.json` 读效果词汇表）
    //   ② 效果词汇表 `spec/effects.json`
    //   ③ 底座自己：`dhampir/bin/<exe>`（出片）+ `dhampir/preview/{engine.js,pkg/*}` + `dhampir/VERSION`
    //      —— 包内形状由 `yekitrim_polish::render::dhampir::{find_binary, wasm_dirs, preview_dir}` 定义
    //      （`<程序目录>/dhampir/…`），底座侧的产出来自它自己的 `node scripts/package.mjs`。
    //
    // ⚠️ **2026-10-09：转译器与词汇表合并进 `<程序目录>/runtime/`**（原先 `tools/` + `spec/`）。
    // 消费侧的候选顺序跟着改（`dhampir.rs::translator_candidates_for` 与
    // `pipeline.rs::spec_effects_path` 都是"新位优先、老位回落"）。
    let runtime_dir = package_dir.join("runtime");
    fs::create_dir_all(&runtime_dir)?;
    let translator = root.join("tools").join("polish-to-dhampir.mjs");
    if !translator.is_file() {
        return Err(format!("missing translator at {}", translator.display()).into());
    }
    fs::copy(&translator, runtime_dir.join("polish-to-dhampir.mjs"))?;
    let vocab = root.join("spec").join("effects.json");
    if !vocab.is_file() {
        return Err(format!("missing effect vocabulary at {}", vocab.display()).into());
    }
    fs::copy(&vocab, runtime_dir.join("effects.json"))?;

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
    println!("    + runtime/polish-to-dhampir.mjs + runtime/effects.json（转译器与效果词汇表）");

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

    // ⚠️ 这里**不该**再要求 `webui/dist`：它已编进 exe。要求它反而会让"正确的包"被判失败。
    for required in ["yekitrim.exe", "LICENSE", "THIRD-PARTY-NOTICES.md"] {
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

/// 把 zip 的校验和写成**每次都重新生成**的旁路文件（放在发行包**之外**，避免自指）。/// 底座新的默认产物目录：`<dhampir>/dist/dhampir-<版本>+<sha>/`，取**版本最高**的一份。
///
/// 为什么要有它：底座 `scripts/package.mjs` 的默认输出改成了 `dist/`，而我们的打包器一直按 `out/` 找，
/// 症状是"底座明明打好了，这边却报产物不齐"（2026-10-01 踩到）。两份都认：这里给 dist 里最新的一份，
/// 调用方拿不到时再退回 `out/`。
///
/// ⚠️ **按版本排序，不按 mtime**（2026-10-08 修）：`dist/` 现在会**同时留着好几个版本**
/// （打过 0.1.0 又打了 0.2.0），而 mtime 排序的语义是"最后碰过的那个"—— 在 0.1.0 目录里
/// 随便 `touch` 一下、或重新跑一次旧版本的打包，就会**静默把旧底座装进发行包**。
/// 这一条在 0.2.0 之前不致命（相邻版本差别小），但 0.2.0 换了字幕栅格化器
/// （`drawtext` → libass）⇒ **老底座出来的是另一套字幕像素**，而**没有任何自动信号**会报出来
/// （`project_schema` / `host_api` 都没动）。所以这里必须比版本，不能比时间。
/// 认不出版本的目录排最后（拿不到就退回目录名比较，仍然不退回 mtime）。
fn newest_dhampir_dist(root: &Path) -> Option<PathBuf> {
    let mut found: Vec<(Vec<u64>, String, PathBuf)> = fs::read_dir(root.join("dist"))
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("dhampir-"))
        })
        .map(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
            (dhampir_version_key(&name), name, p)
        })
        .collect();
    // 版本号逐段数值比较（0.10.0 > 0.9.0；字典序会反过来），同版本再按目录名稳定定序。
    found.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    found.pop().map(|(_, _, p)| p)
}

/// 从 `dhampir-<版本>+<sha>` 里抠出可逐段比较的版本键。认不出 ⇒ 空键（排最后）。
///
/// 只认 `dhampir-` 前缀之后、第一个 `+` 或 `-` 之前的那一段（`0.2.0+60197a2`、`0.2.0-win32-x64` 都中）。
fn dhampir_version_key(dir_name: &str) -> Vec<u64> {
    let rest = dir_name.strip_prefix("dhampir-").unwrap_or(dir_name);
    let version = rest.split(['+', '-']).next().unwrap_or("");
    let parts: Vec<u64> = version.split('.').map_while(|s| s.parse().ok()).collect();
    // `parse` 到第一个非数字就停：此刻若没吃完整段，说明这不是版本号（例如 `dhampir-nightly`）。
    if parts.is_empty() || parts.len() != version.split('.').count() {
        return Vec::new();
    }
    parts
}

// ============================================================================
// Linux x64 打包通道（2026-10-05）
// ============================================================================
//
// 与 Windows 通道的差异，逐条都有理由：
//
//   · **归档用 tar.gz，不是 zip**。Linux 用户一律 `tar xzf`；zip 在这边不通用，
//     且 zip 会丢掉 Unix 权限位 —— 那样解出来的 `yekitrim` **不是可执行文件**，
//     用户得自己 `chmod +x`。tar 保留 0755。
//   · **没有托盘外壳**（`yekitrim-tray` 是 Windows 专属概念，没有桌面常驻进程这回事）。
//   · **没有 `.ico`**（图标归桌面环境的 .desktop 文件，不归二进制）。
//   · **必须查 glibc 上限**：Linux 二进制会写死它需要的最高 `GLIBC_x.y` 符号版本，
//     宿主低于它就**直接起不来**（`version 'GLIBC_2.39' not found`）。这不是能靠文档
//     提醒的注意事项 —— Windows 通道没有这个维度，所以这里必须多一道检查。
//   · **底座（Dhampir）**：Windows 是 `dhampir.exe`，Linux 是 `dhampir`（无扩展名）。
//
// 门禁与 Windows 完全同一套：`scan` 是 **mandatory and unskippable**。
// ⚠️ 而 `scan` 在 2026-10-05 之前**扫不到 ELF**（按扩展名收文件，Linux 产物没有扩展名），
// 所以 Linux 侧此前等于**没有 GPL 门禁**。已修（见 `looks_like_artifact`）。

/// Linux 产物允许出现的最高 glibc 符号版本。
///
/// 基座是 `ubuntu:24.04`（glibc 2.39）。比这更高的符号说明**没在容器里编**
/// （比如在滚动发行版上直编）—— 那种产物绝大多数用户跑不了，必须拦下。
const MAX_GLIBC: (u32, u32) = (2, 39);

fn run_package_linux(
    options: &PackageOptions,
    root: &Path,
    version: &str,
    package_name: &str,
    package_dir: &Path,
    tarball: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let exe = root.join("target").join("release").join("yekitrim");
    if !exe.is_file() {
        return Err(format!(
            "missing {} -- run `cargo build --release -p yekitrim-cli` first",
            exe.display()
        )
        .into());
    }

    // ---------------------------------------------------------------- GPL 门禁
    // 与 Windows 同一条，不可跳过。对 ELF 一样有效（`scan` 已改成按内容认文件）。
    println!("==> asserting the binary carries no espeak-ng");
    let scan = Command::new(env::current_exe()?)
        .args(["scan", exe.to_string_lossy().as_ref()])
        .status()?;
    if !scan.success() {
        return Err("the product gate rejected target/release/yekitrim. Refusing to package a \
                    binary that links espeak-ng."
            .into());
    }

    // ---------------------------------------------------------------- glibc 上限
    println!("==> checking the glibc baseline");
    let glibc_max = max_glibc_symbol(&exe)?;
    match glibc_max {
        Some((major, minor)) => {
            println!("    requires glibc <= {major}.{minor}");
            if (major, minor) > MAX_GLIBC {
                return Err(format!(
                    "this binary needs GLIBC_{major}.{minor}, above the {}.{} baseline.\n\
                     It was almost certainly built on a rolling-release host instead of the \
                     pinned container.\n\
                     Rebuild with:  docker/Dockerfile.linux-x64 (see README).\n\
                     Shipping it would give users \"version `GLIBC_{major}.{minor}' not found\" \
                     on every distro we target.",
                    MAX_GLIBC.0, MAX_GLIBC.1
                )
                .into());
            }
        }
        None => {
            // 一个 GLIBC_x.y 都没有很奇怪（静态链接？）—— 不当失败，但要说出来。
            println!("    WARNING: no GLIBC_* symbol versions found; cannot verify the baseline");
        }
    }

    // ---------------------------------------------------------------- 组装
    println!("==> assembling {package_name}");
    if package_dir.exists() {
        fs::remove_dir_all(package_dir)?;
    }
    fs::create_dir_all(package_dir.join("resources").join("fonts"))?;

    fs::copy(&exe, package_dir.join("yekitrim"))?;
    // WebUI / 字体与 Windows 一样是**编进二进制**的，包里不带 dist（见 AGENTS.md）。
    // 但保留了 resources/fonts 空目录 + 说明：那是运行时的数据目录落点，用户会往里放字体。
    fs::write(
        package_dir.join("resources").join("fonts").join("放这里.txt"),
        "本目录就是字体库。\n\n\
         YekiTrim 首次启动会把自带字体落盘到 <数据目录>/resources/fonts/。\n\
         数据目录默认是**本程序所在目录**，可用环境变量 YEKITRIM_DATA_DIR 覆盖。\n",
    )?;

    for name in ["LICENSE", "THIRD-PARTY-NOTICES.md"] {
        let src = root.join(name);
        if src.is_file() {
            fs::copy(&src, package_dir.join(name))?;
        } else {
            return Err(format!("{name} is mandatory in the package but missing from the repo").into());
        }
    }

    // 发行说明（有才带）
    let notes = root.join("docs").join("releases").join(format!("v{version}.md"));
    if notes.is_file() {
        fs::copy(&notes, package_dir.join("RELEASE-NOTES.md"))?;
    } else {
        println!("    note: docs/releases/v{version}.md not found; packaging without RELEASE-NOTES.md");
    }

    // 默认出片通道（Dhampir 底座）。Linux 侧二进制不带扩展名。
    //
    // ⚠️ **2026-10-05：这里一开始漏了 `preview/` 与 `VERSION`。**
    // 第一版 Linux 打包只拷了 `dhampir/bin/dhampir`，于是包内缺
    // `<程序目录>/dhampir/preview/{engine.js,pkg/*}` 与 `dhampir/VERSION` ——
    // 而 `yekitrim_polish::render::dhampir::preview_dir()` 正是按**这个形状**找预览产物的。
    // 后果：Linux 用户**没有页面预览**（出片不受影响，因为那是另一条腿；
    // 但"预览缺失"在 UI 上是静默的 —— 字段为 None，不进 problems，用户只会发现预览不好使）。
    // Windows 通道一直带着它们，两个平台必须同形状。
    let dhampir_artifact = if options.no_dhampir {
        None
    } else {
        Some(
            options
                .dhampir_artifact
                .clone()
                .or_else(|| {
                    let sibling = root.parent()?.join("Dhampir");
                    newest_dhampir_dist(&sibling).or_else(|| Some(sibling.join("out")))
                })
                .ok_or("no sibling Dhampir checkout: pass --dhampir-artifact <dir> or --no-dhampir")?,
        )
    };

    let dhampir_exe = options
        .dhampir_exe
        .clone()
        .or_else(|| {
            let sibling = root.parent()?.join("Dhampir").join("target").join("release").join("dhampir");
            sibling.is_file().then_some(sibling)
        })
        .or_else(|| {
            let sibling = root.parent()?.join("Dhampir").join("target").join("debug").join("dhampir");
            sibling.is_file().then_some(sibling)
        });

    if options.no_dhampir {
        println!("    note: --no-dhampir, packaging WITHOUT the renderer (default path will be unavailable)");
    } else {
        // 底座产物三样 + VERSION：缺一即拒绝（与 Windows 同一条判据）。
        if let Some(artifact) = &dhampir_artifact {
            for required in [
                artifact.join("preview").join("engine.js"),
                artifact.join("preview").join("pkg").join("dhampir_wasm.js"),
                artifact.join("preview").join("pkg").join("dhampir_wasm_bg.wasm"),
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
            copy_dir_all(
                &artifact.join("preview"),
                &package_dir.join("dhampir").join("preview"),
            )?;
            fs::copy(
                &artifact.join("VERSION"),
                package_dir.join("dhampir").join("VERSION"),
            )?;
            println!("    dhampir artifact: {}", artifact.display());
        }
        match &dhampir_exe {
            Some(path) => {
                let bin_dir = package_dir.join("dhampir").join("bin");
                fs::create_dir_all(&bin_dir)?;
                fs::copy(path, bin_dir.join("dhampir"))?;
                println!("    dhampir: {}", path.display());
            }
            None => {
                return Err(
                    "no Dhampir renderer binary found.\n\
                     The default render path is the product's main path, so packaging without it \
                     is refused.\n\
                     Build it first, or point at one with --dhampir-exe <path>, or pass \
                     --no-dhampir to ship a package whose default path is unavailable."
                        .into(),
                );
            }
        }
    }

    // 转译器 + 效果词汇表（与 Windows 同源）
    //
    // ⚠️ **2026-10-09：两者合并进 `<程序目录>/runtime/`**（原先分别是 `tools/` 与 `spec/`）。
    // 合并后转译器与词汇表**同目录**，所以它自己那份查找会先试 `./effects.json`、
    // 再回落 `../spec/effects.json`（见 `polish-to-dhampir.mjs` 的 `EFFECT_VOCAB`）——
    // 开发树里仍是 `tools/` + `spec/` 并列，两条路都得留。
    let runtime_dir = package_dir.join("runtime");
    fs::create_dir_all(&runtime_dir)?;
    let translator = root.join("tools").join("polish-to-dhampir.mjs");
    if translator.is_file() {
        fs::copy(&translator, runtime_dir.join("polish-to-dhampir.mjs"))?;
    }
    let spec = root.join("spec").join("effects.json");
    if spec.is_file() {
        fs::copy(&spec, runtime_dir.join("effects.json"))?;
    }

    // ---------------------------------------------------------------- 可选冒烟
    if options.smoke_test {
        smoke_test_linux(package_dir)?;
        // ⚠️ **烟测会把内嵌字体解包进包目录，必须清掉**（与 Windows 通道同一条，
        // 2026-10-02 在 Windows 上实际踩到，2026-10-05 发现 Linux 通道漏了这一步）。
        //
        // 机制：`fonts::ensure_dir` 写的**就是** `<数据目录>/resources/fonts`，
        // 而烟测起的进程就在包目录里（`exe_dir()` = 包目录）⇒ 两个自带字体
        // （NotoSansSC-Regular + NotoColorEmoji-CBDT，合计约 **21 MB**）被解包进
        // `resources/fonts/<slug>/`，随后原样进归档。
        //
        // 这是**纯重复**（同样字节已经 `include_bytes!` 编在 `yekitrim` 里了），
        // 还直接违反发行说明「发行包只带一个空 fonts 目录 + 说明」那条。
        // 实测未清理时包从 40.3 MB 涨到 56.2 MB —— 正好是那 21 MB 字体。
        //
        // 只删字体目录里的**子项**，保留 `放这里.txt`（组装阶段写的，属于包的一部分）。
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
    }

    // ---------------------------------------------------------------- 打包
    println!("==> compressing {}", tarball.display());
    if tarball.exists() {
        fs::remove_file(tarball)?;
    }
    tar_gz_dir(package_dir, tarball)?;

    let size_bytes = fs::metadata(tarball)?.len();
    let size_mb = size_bytes as f64 / (1024.0 * 1024.0);
    let digest = sha256_of_file(tarball)?;
    println!();
    println!("done: {}", tarball.display());
    println!("  size:   {size_mb:.1} MB");
    println!("  sha256: {digest}");

    let sidecar = write_checksum_sidecar(tarball, &digest, size_bytes, options.smoke_test)?;
    println!("  checksum: {}", sidecar.display());

    Ok(())
}

/// 从 ELF 的动态符号表里取它需要的**最高** `GLIBC_x.y`。
///
/// 刻意不依赖外部工具（`objdump` / `readelf`）—— 那个 crate 只用 std + sha2 + zip，
/// 塞一个外部依赖进来会破坏"不需要工具链之外的任何东西"这条（见 AGENTS.md）。
/// 直接把文件读进来找 `GLIBC_` 字面量：这些版本串就在 `.dynstr` 里，纯字节匹配足够。
fn max_glibc_symbol(path: &Path) -> io::Result<Option<(u32, u32)>> {
    let bytes = fs::read(path)?;
    let needle = b"GLIBC_";
    let mut best: Option<(u32, u32)> = None;

    let mut i = 0usize;
    while i + needle.len() < bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            // 形如 GLIBC_2.39：解析紧跟其后的 "数字.数字"
            let rest = &bytes[i + needle.len()..];
            let end = rest
                .iter()
                .position(|b| !(b.is_ascii_digit() || *b == b'.'))
                .unwrap_or(rest.len());
            if let Ok(text) = std::str::from_utf8(&rest[..end]) {
                if let Some((major, minor)) = text.split_once('.') {
                    if let (Ok(major), Ok(minor)) =
                        (major.parse::<u32>(), minor.parse::<u32>())
                    {
                        if best.is_none_or(|b| (major, minor) > b) {
                            best = Some((major, minor));
                        }
                    }
                }
            }
            i += needle.len();
        } else {
            i += 1;
        }
    }
    Ok(best)
}

/// Linux 冒烟：起打包好的 `yekitrim server`，验 `/api/health` 与 WebUI，然后杀掉并清理运行态。
///
/// 与 Windows 版的差别：不设 `CREATE_NO_WINDOW`（那是 Windows 概念），
/// 可执行文件名不带 `.exe`。其余判据一致（`"ok"` + `true`、`<div id="app">`）。
fn smoke_test_linux(package_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let exe = package_dir.join("yekitrim");
    if !exe.is_file() {
        return Err(format!("smoke test: missing {}", exe.display()).into());
    }

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let port: u16 = 18000 + (nanos % 1000) as u16;

    println!("==> smoke testing the packaged binary on port {port}");
    let mut child = Command::new(&exe)
        .current_dir(package_dir)
        .args([
            "server",
            "--local-no-auth",
            "--port",
            &port.to_string(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut health_ok = false;
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(500));
            if let Ok(body) = http_get(port, "/api/health", Duration::from_secs(5)) {
                if body.contains("ok") && body.contains("true") {
                    health_ok = true;
                    break;
                }
            }
        }
        if !health_ok {
            return Err("smoke test: /api/health never became healthy".into());
        }
        println!("    /api/health ok");

        match http_get(port, "/", Duration::from_secs(5)) {
            Ok(body) if body.contains("<div id=\"app\">") => println!("    WebUI root ok"),
            Ok(_) => return Err("smoke test: / did not serve the WebUI".into()),
            Err(err) => return Err(format!("smoke test: / did not respond: {err}").into()),
        }
        Ok(())
    })();

    let _ = child.kill();
    let _ = child.wait();

    // 首次运行会生成运行态（数据目录、server.toml 含真实绝对路径）—— 一律不许进发布包。
    // ⚠️ **2026-10-09：素材库落点由 `stickers/` 改成 `resources/` 下的子目录**。
    // ⚠️ **2026-10-30：多用户布局会在包目录下生成 `global/`（内置字体）与 `users/`** ——
    //    实测它们被原样打进 tar.gz（包从 ~40MB 涨到 57.6MB，且带着烟测的解包字体）。
    //    `global/` 与 `users/` 都是**运行态**，组装阶段不会创建它们，一律清掉；
    //    仅保留 `resources/fonts/放这里.txt` 这个随包说明。
    // ⚠️ `resources/` 与 `runtime/` 是**随包发的**，别整个删 —— 只清生成出来的素材库子目录，
    // 且 `resources/fonts` 只清子项、保留组装阶段写的 `放这里.txt`（与 Windows 通道同一条）。
    for junk in [".yekitrim", "stickers", "global", "users"] {
        let p = package_dir.join(junk);
        if p.exists() {
            let _ = fs::remove_dir_all(&p);
        }
    }
    for junk in ["resources/stickers", "resources/videos"] {
        let p = package_dir.join(junk);
        if p.exists() {
            let _ = fs::remove_dir_all(&p);
        }
    }
    let fonts_dir = package_dir.join("resources").join("fonts");
    if let Ok(entries) = fs::read_dir(&fonts_dir) {
        for entry in entries.flatten() {
            if entry.file_name() == "放这里.txt" {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                let _ = fs::remove_dir_all(&path);
            } else {
                let _ = fs::remove_file(&path);
            }
        }
    }
    result
}

/// 用系统 `tar` 打 `.tar.gz`。
///
/// ⚠️ 这里**故意**破一次"不依赖外部工具"的规矩，理由要说清楚：
/// 现有的 `zip` crate 只能写 zip，而 Linux 必须发 tar（见本节开头：zip 会丢权限位，
/// 解出来不可执行）。为了写 tar.gz 去引一个 bzip2/gzip 的 Rust 依赖不划算 ——
/// `tar` 是 Linux 上的**基线工具**（POSIX 规范的一部分，容器基座里必然有），
/// 与 python / PowerShell（那个 crate 当初刻意摆脱的东西）不是一个量级。
///
/// ⚠️ **必须固定 mtime / 排序 / 属主**，否则同一份内容每次打包 sha256 都变，
/// `checksums` 记录就永远校验不上（这一点在 sherpa 仓实测踩过）。
fn tar_gz_dir(source: &Path, destination: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let parent = source.parent().ok_or("tarball source has no parent")?;
    let name = source.file_name().ok_or("tarball source has no name")?;

    let status = Command::new("tar")
        .current_dir(parent)
        .args([
            "-czf",
            destination.to_string_lossy().as_ref(),
            "--sort=name",
            "--mtime=2026-01-01 00:00:00 UTC",
            "--owner=0",
            "--group=0",
            "--numeric-owner",
            name.to_string_lossy().as_ref(),
        ])
        .status()?;
    if !status.success() {
        return Err(format!("tar failed ({status})").into());
    }
    Ok(())
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
    // 校验行按平台给：Linux 用 sha256sum，Windows 保留 certutil / Get-FileHash。
    // 写错平台等于给用户一条跑不通的命令 —— 那和没有校验和一样糟。
    let is_linux_artifact = name.ends_with(".tar.gz");

    // ⚠️ Linux 侧的**首行必须是 `sha256sum -c` 能直接吃的格式**：
    //     `<64位hex>  <文件名>`（两个空格）。
    //     2026-10-05 之前这里只放了个裸文件名 + `sha256  <digest>`，看着像校验和，
    //     实际 `sha256sum -c` 会报 "no properly formatted checksum lines found"
    //     —— 又是一个"撒谎的校验和"。注释里却写着它就是给 `-c` 用的。
    let first_line = if is_linux_artifact {
        format!("{digest}  {name}")
    } else {
        name.clone()
    };

    let verify = if is_linux_artifact {
        format!(
            "#   sha256sum -c {name}.sha256.txt      # 直接校验（本文件首行就是可解析格式）\n\
             #   sha256sum {name}                    # 只想看实际值\n"
        )
    } else {
        format!(
            "  certutil -hashfile {name} SHA256\n  \
             (Get-FileHash .\\{name} -Algorithm SHA256).Hash\n"
        )
    };
    let platform = if is_linux_artifact {
        "Linux x64"
    } else {
        "Windows x64"
    };
    // Linux 版把机器可解析那行放**最前**（`-c` 只认得了格式的行，其余按注释忽略；
    // 但保守起见其余行一律 `#` 开头，免得被误解析）。
    let text = if is_linux_artifact {
        format!(
            "{first_line}\n\
             #\n\
             # size    {size_bytes} bytes ({mb:.1} MB)\n\
             # built   {built}（{platform}；{how}）\n\
             #\n\
             # 校验：\n{verify}\
             # 说明：本文件由 `cargo xtask package` **每次重新生成**（别手工改）；\n\
             # 刻意放在发行包**之外** —— RELEASE-NOTES.md 在包内，把校验和写进去会形成自指。\n"
        )
    } else {
        format!(
            "{first_line}\n\
             sha256  {digest}\n\
             size    {size_bytes} bytes ({mb:.1} MB)\n\
             built   {built}（{platform}；{how}）\n\
             \n\
             校验：\n{verify}\
             \n\
             说明：本文件由 `cargo xtask package` **每次重新生成**（别手工改）；\n\
             刻意放在发行包**之外** —— RELEASE-NOTES.md 在包内，把校验和写进去会形成自指。\n"
        )
    };
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

    // First run creates .yekitrim next to the exe, and the asset library under resources/;
    // runtime state must not ship.
    //
    // ⚠️ `resources/` 本身是**随包发的**（里面有 `fonts/放这里.txt`），所以**不能整个删**
    // —— 只清首次运行会生成的那几个**素材库**子目录。`runtime/` 也是随包发的，**不清理**。
    // ⚠️ `resources/fonts` **不在这里删**：它的清理在下面，得保留 `放这里.txt`。
    for generated in [".yekitrim", "global", "users"] {
        let path = package_dir.join(generated);
        if path.exists() {
            fs::remove_dir_all(&path)?;
        }
    }
    for generated in ["resources/stickers", "resources/videos"] {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn dist(dir: &str, names: &[&str]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("xtask-dist-{dir}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for n in names {
            fs::create_dir_all(root.join("dist").join(n)).unwrap();
        }
        root
    }

    fn picked(root: &Path) -> Option<String> {
        newest_dhampir_dist(root)
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
    }

    #[test]
    fn version_key_parses_both_dir_shapes() {
        assert_eq!(dhampir_version_key("dhampir-0.2.0+60197a2"), vec![0, 2, 0]);
        assert_eq!(dhampir_version_key("dhampir-0.2.0-win32-x64"), vec![0, 2, 0]);
        // 认不出来的排最后，不许当成 0.0.0 混进中间。
        assert!(dhampir_version_key("dhampir-nightly").is_empty());
        assert!(dhampir_version_key("dhampir-").is_empty());
    }

    /// 0.10.0 必须赢过 0.9.0 —— 字典序会给出反答案，这正是不能 `sort()` 字符串的原因。
    #[test]
    fn version_key_sorts_numerically_not_lexically() {
        assert!(dhampir_version_key("dhampir-0.10.0+a") > dhampir_version_key("dhampir-0.9.0+a"));
        assert!(dhampir_version_key("dhampir-1.0.0+a") > dhampir_version_key("dhampir-0.99.0+a"));
    }

    /// 核心判据：**旧版本目录更"新"也不许被选中**（这就是 mtime 排序的病）。
    #[test]
    fn older_version_wins_never_even_when_touched_later() {
        let root = dist("stale-touch", &["dhampir-0.2.0+aaa", "dhampir-0.1.0+bbb"]);
        // 给**旧**的那份刷一个更晚的 mtime，模拟"重新打了一次 0.1.0"。
        let stale = root.join("dist").join("dhampir-0.1.0+bbb");
        let f = stale.join("VERSION");
        fs::write(&f, "version=0.1.0\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&f, "version=0.1.0\n").unwrap();
        assert_eq!(picked(&root).as_deref(), Some("dhampir-0.2.0+aaa"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unrecognised_dirs_never_beat_a_real_version() {
        let root = dist("junk", &["dhampir-0.1.0+a", "dhampir-scratch"]);
        assert_eq!(picked(&root).as_deref(), Some("dhampir-0.1.0+a"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn no_dist_dir_is_none_so_caller_falls_back_to_out() {
        let root = std::env::temp_dir().join(format!("xtask-none-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("out")).unwrap();
        assert_eq!(picked(&root), None);
        let _ = fs::remove_dir_all(&root);
    }
}
