//! 把品牌图标作为**资源**编进 exe（`yekitrim.exe` 与 `yekitrim-tray.exe` 都受益）。
//!
//! 不给 exe 内嵌图标，它在资源管理器/任务栏里就是那个白纸默认图标。
//!
//! ## 为什么是"自己找 rc.exe"而不是用 `embed-resource` crate
//!
//! 那个 crate 干净好用，但实测它给 `Cargo.lock` 带进 **10 个新包**
//! （`embed-resource` + `vswhom`/`vswhom-sys`/`winreg` + 一套 `toml 1.x`（与已有的 `toml 0.8` 并存）
//! + `windows-sys 0.59`）。本仓对依赖是**记账制**的：`THIRD-PARTY-NOTICES.md` 里
//! "构建期使用、不随二进制分发"那节写死了包数与逐条授权，而且它自己写着
//! "新增依赖后要按同样的方式重跑审计"。为**一个图标**去动那份审计账不划算。
//!
//! `rc.exe` 不是新东西：它跟 `link.exe` 同属本仓**本来就要求**的 MSVC 工具链 + Windows SDK。
//!
//! ## 尽力而为
//!
//! 找不到 rc.exe 就**跳过后继续构建**（只发一条 `cargo:warning`）—— 图标是外观，
//! 不该让"缺一个 SDK 工具"变成构建失败。宁可没图标，也不要别人 clone 下来编不过。

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    // ⚠️ 用 TARGET 而不是 HOST：交叉编译时该看目标平台。非 Windows 整条跳过。
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // 换图标要能触发重编，否则改了 .ico 还是旧图标，而且**不报错**
    println!("cargo:rerun-if-changed=assets/yekitrim.ico");
    println!("cargo:rerun-if-changed=assets/yekitrim.rc");

    let Some(rc) = find_rc() else {
        println!(
            "cargo:warning=没找到 rc.exe（Windows SDK 的资源编译器），跳过 exe 图标；\
             exe 在资源管理器里会是默认图标。装 Windows SDK 后重编即可。"
        );
        return;
    };

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let res = out.join("yekitrim-icon.res");
    // ⚠️ 构建脚本的 CWD 就是 crate 根，而 .rc 里写的是**相对 crate 根**的 `assets/yekitrim.ico`
    let status = Command::new(&rc)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res)
        .arg("assets/yekitrim.rc")
        .status();
    match status {
        Ok(s) if s.success() && res.is_file() => {
            println!("cargo:rustc-link-arg={}", res.display());
        }
        other => {
            // 不 panic：图标是外观，不该拦构建。但必须让人看见（静默过去 = "exe 莫名没图标"）
            println!("cargo:warning=rc.exe 编译图标资源失败（{other:?}），本次产物没有 exe 图标");
        }
    }
}

/// 找 `rc.exe`：先在 PATH 上找，再扫 Windows SDK 的 `bin/<版本>/<架构>/`（取最高版本）。
fn find_rc() -> Option<PathBuf> {
    if let Ok(p) = which("rc.exe") {
        return Some(p);
    }
    // 目标架构 → SDK 里的目录名
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86") => "x86",
        Ok("aarch64") => "arm64",
        _ => "x64",
    };
    let kits = std::env::var("ProgramFiles(x86)")
        .or_else(|_| std::env::var("ProgramFiles"))
        .ok()?;
    let bin = Path::new(&kits)
        .join("Windows Kits")
        .join("10")
        .join("bin");
    let mut best: Option<(Vec<u64>, PathBuf)> = None;
    for entry in std::fs::read_dir(&bin).ok()?.flatten() {
        let cand = entry.path().join(arch).join("rc.exe");
        if !cand.is_file() {
            continue;
        }
        // 版本号按数字段比大小，别按字符串（"10.0.999" < "10.0.26100" 会错）
        let ver: Vec<u64> = entry
            .file_name()
            .to_string_lossy()
            .split('.')
            .map(|s| s.parse().unwrap_or(0))
            .collect();
        if best.as_ref().map(|(v, _)| ver > *v).unwrap_or(true) {
            best = Some((ver, cand));
        }
    }
    best.map(|(_, p)| p)
}

/// PATH 上找可执行文件（自己走一遍，不引 `which` crate）。
fn which(name: &str) -> Result<PathBuf, ()> {
    let path = std::env::var_os("PATH").ok_or(())?;
    for dir in std::env::split_paths(&path) {
        let cand = dir.join(name);
        if cand.is_file() {
            return Ok(cand);
        }
    }
    Err(())
}
