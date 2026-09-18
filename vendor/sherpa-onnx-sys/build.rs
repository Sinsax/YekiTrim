//! V-Trim fork of sherpa-onnx-sys. Everything except this file is verbatim upstream.
//! See FORK-NOTES.md next to this file for the deltas and how to resync on a bump.
//!
//! The deltas exist because upstream (a) links GPL-3.0-or-later espeak-ng unconditionally,
//! (b) prefers its own target/ cache over any archive the consumer points at, and (c) verifies
//! nothing it downloads. All three fail silently. The protection therefore lives here, on the
//! one path every build has to take.

use std::env;
use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{collections::HashSet, ffi::OsString};

use bzip2::read::BzDecoder;
use sha2::{Digest, Sha256};
use tar::Archive;

// V-Trim delta 1: upstream points at github.com/k2-fsa/sherpa-onnx, whose prebuilt archive
// links espeak-ng. This points at the TTS-off rebuild of the same version instead.
const RELEASE_BASE_URL: &str =
    "https://github.com/Sinsax/sherpa-onnx-prebuilt-no-tts/releases/download";

// V-Trim delta 2: the only archive this fork will accept. Same file name as upstream so it is a
// drop-in replacement, different bytes. Bump the name and the hash together.
const PINNED_ARCHIVE_NAME: &str = "sherpa-onnx-v1.13.3-win-x64-static-MT-Release-lib.tar.bz2";
const PINNED_ARCHIVE_SHA256: &str =
    "94d733a9cda2d959b1f924d75f771154a5acc4c8241b4393967afdb725e23b1f";

// Placeholder stubs shipped inside the pinned archive. SHERPA_ONNX_STATIC_LIBS below names them
// unconditionally, so the files must exist -- but with TTS off nothing references an espeak
// symbol and the linker pulls nothing out of them. The real upstream espeak-ng.lib is 823,674
// bytes; the placeholder is 896. Anything above STUB_MAX_BYTES is the real library.
const STUB_LIBS: &[&str] = &["espeak-ng.lib", "piper_phonemize.lib", "ucd.lib"];
const STUB_MAX_BYTES: u64 = 4096;

const SHERPA_ONNX_STATIC_LIBS: &[&str] = &[
    "sherpa-onnx-c-api",
    "sherpa-onnx-core",
    "kaldi-decoder-core",
    "sherpa-onnx-kaldifst-core",
    "sherpa-onnx-fstfar",
    "sherpa-onnx-fst",
    "kaldi-native-fbank-core",
    "kissfft-float",
    "piper_phonemize",
    "espeak-ng",
    "ucd",
    "onnxruntime",
    "ssentencepiece_core",
];

type DynError = Box<dyn Error>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LinkMode {
    Static,
    Shared,
}

fn main() {
    if let Err(err) = try_main() {
        panic!("{err}");
    }
}

fn try_main() -> Result<(), DynError> {
    println!("cargo:rerun-if-env-changed=SHERPA_ONNX_LIB_DIR");
    println!("cargo:rerun-if-env-changed=SHERPA_ONNX_ARCHIVE_DIR");
    println!("cargo:rerun-if-env-changed=DOCS_RS");

    if env::var_os("DOCS_RS").is_some() {
        // docs.rs sets DOCS_RS=1; skip downloading/linking native libraries
        // so that `cargo doc` can succeed without the real C artifacts.
        return Ok(());
    }

    let target_os = env::var("CARGO_CFG_TARGET_OS")?;
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH")?;
    let link_mode = resolve_link_mode()?;
    let lib_dir = resolve_lib_dir(link_mode, &target_os, &target_arch)?;

    println!("cargo:rustc-link-search=native={}", lib_dir.display());

    if link_mode == LinkMode::Shared && matches!(target_os.as_str(), "linux" | "macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
        emit_relative_rpath(&target_os);
        copy_unix_runtime_libs(&lib_dir, &target_os)?;
    }

    if link_mode == LinkMode::Shared && target_os == "windows" {
        copy_windows_runtime_dlls(&lib_dir)?;
    }

    match link_mode {
        LinkMode::Static => emit_static_link_directives(&target_os),
        LinkMode::Shared => emit_shared_link_directives(),
    }

    Ok(())
}

fn resolve_link_mode() -> Result<LinkMode, DynError> {
    let static_enabled = env::var_os("CARGO_FEATURE_STATIC").is_some();
    let shared_enabled = env::var_os("CARGO_FEATURE_SHARED").is_some();

    if static_enabled && shared_enabled {
        return Err("Features `static` and `shared` cannot be enabled at the same time".into());
    }

    if shared_enabled {
        Ok(LinkMode::Shared)
    } else {
        Ok(LinkMode::Static)
    }
}

fn resolve_lib_dir(
    link_mode: LinkMode,
    target_os: &str,
    target_arch: &str,
) -> Result<PathBuf, DynError> {
    if let Some(path) = env::var_os("SHERPA_ONNX_LIB_DIR") {
        let path = PathBuf::from(path);
        if !path.is_dir() {
            return Err(format!(
                "SHERPA_ONNX_LIB_DIR does not exist or is not a directory: {}",
                path.display()
            )
            .into());
        }
        return Ok(path);
    }

    download_prebuilt_libs(link_mode, target_os, target_arch)
}

fn download_prebuilt_libs(
    link_mode: LinkMode,
    target_os: &str,
    target_arch: &str,
) -> Result<PathBuf, DynError> {
    let archive_name = archive_name(link_mode, target_os, target_arch)?;

    // V-Trim delta 2. Our release only carries the pinned Windows x64 static archive. Any other
    // target or link mode would have to come from the official upstream release, which links
    // GPL-3.0-or-later espeak-ng unconditionally -- so fail loudly rather than quietly ship a
    // GPL binary. SHERPA_ONNX_LIB_DIR (checked first, below) remains the escape hatch.
    if archive_name != PINNED_ARCHIVE_NAME {
        return Err(format!(
            "V-Trim's vendored sherpa-onnx-sys only ships the TTS-off archive\n  \
             {PINNED_ARCHIVE_NAME}\nbut this build resolves to\n  {archive_name}\n\
             Build for windows/x86_64 with the default (static) feature set, or set \
             SHERPA_ONNX_LIB_DIR to a lib directory you built yourself. Removing the \
             [patch.crates-io] entry for sherpa-onnx-sys in the workspace root Cargo.toml opts \
             back in to the upstream crate -- and to its GPL espeak-ng link."
        )
        .into());
    }

    let archive_stem = archive_name.trim_end_matches(".tar.bz2");

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let cache_root = target_dir_from_out_dir(&out_dir)?.join("sherpa-onnx-prebuilt");
    let extracted_dir = cache_root.join(archive_stem);
    let lib_dir = extracted_dir.join("lib");

    if lib_dir.is_dir() {
        // V-Trim delta 4, and the important half of it: the cache outranks every archive source
        // below, so an extracted dir left behind by an official-archive build would win and
        // nothing downstream would notice. Check the contents instead of trusting the path.
        // This is what makes a bare `cargo build` (and an IDE build) safe.
        assert_placeholders_are_placeholders(&lib_dir)?;
        return Ok(lib_dir);
    }

    fs::create_dir_all(&cache_root)?;

    let archive_path = cache_root.join(&archive_name);

    // An archive left in the cache by an older build was never verified (upstream verifies
    // nothing), so hash it before use. On mismatch, drop it and re-acquire below.
    if archive_path.is_file() {
        match sha256_of_file(&archive_path) {
            Ok(found) if found == PINNED_ARCHIVE_SHA256 => {}
            Ok(found) => {
                println!(
                    "cargo:warning=cached {archive_name} has sha256 {found}, expected \
                     {PINNED_ARCHIVE_SHA256}; re-acquiring"
                );
                fs::remove_file(&archive_path)?;
            }
            Err(err) => {
                println!("cargo:warning=could not hash cached {archive_name} ({err}); re-acquiring");
                fs::remove_file(&archive_path)?;
            }
        }
    }

    if !archive_path.is_file() {
        if let Some(local_archive_dir) = env::var_os("SHERPA_ONNX_ARCHIVE_DIR") {
            let local_archive_path = PathBuf::from(local_archive_dir).join(&archive_name);
            if !local_archive_path.is_file() {
                return Err(format!(
                    "SHERPA_ONNX_ARCHIVE_DIR does not contain expected archive: {}",
                    local_archive_path.display()
                )
                .into());
            }

            // Offline path, but still the pinned bytes: pointing this at the official archive
            // must fail, not build.
            verify_pinned_sha256(&local_archive_path)?;
            copy_file_atomically(&local_archive_path, &archive_path)?;
        } else {
            let version = env!("CARGO_PKG_VERSION");
            let url = format!("{RELEASE_BASE_URL}/v{version}/{archive_name}");
            eprintln!("Downloading TTS-off sherpa-onnx libs from {url}");

            // V-Trim delta 5: upstream sets no timeouts at all, so a stalled transfer hangs the
            // build forever without printing anything. Measured here behind a local proxy: the
            // 106 MiB transfer crawled to 66 MiB and then stopped growing -- 0 bytes in 20 s, no
            // error, no progress. `timeout_read` bounds the gap between reads (not the whole
            // transfer), so a slow but live link still succeeds while a dead one fails loudly.
            let response = ureq::builder()
                .try_proxy_from_env(true)
                .timeout_connect(Duration::from_secs(30))
                .timeout_read(Duration::from_secs(60))
                .build()
                .get(&url)
                .call()
                .map_err(|e| {
                    format!(
                        "Failed to download the TTS-off sherpa-onnx archive from {url}: {e}\n\
                         If this is a timeout or a connection error, suspect a local proxy: this \
                         crate honours HTTP_PROXY/HTTPS_PROXY. Stalling partway through a 106 MiB \
                         transfer is a known failure mode; clearing the proxy for this build or a \
                         pre-downloaded archive via SHERPA_ONNX_ARCHIVE_DIR both work around it."
                    )
                })?;
            let mut reader = response.into_reader();
            if let Err(err) = write_reader_atomically(&mut reader, &archive_path) {
                // Do not leave a partial file for the next build to trip over.
                let _ = fs::remove_file(temp_path_for(&archive_path));
                return Err(err);
            }

            // V-Trim delta 3. Never leave an unverified 106 MiB file in the cache: the next
            // build would find it, skip the download, and unpack it without looking.
            if let Err(err) = verify_pinned_sha256(&archive_path) {
                let _ = fs::remove_file(&archive_path);
                return Err(err);
            }
        }
    }

    if extracted_dir.exists() {
        fs::remove_dir_all(&extracted_dir)?;
    }

    let unpack_result: Result<(), DynError> = (|| {
        let tar_file = File::open(&archive_path)?;
        let decoder = BzDecoder::new(tar_file);
        let mut archive = Archive::new(decoder);
        archive.unpack(&cache_root)?;
        Ok(())
    })();
    if let Err(err) = unpack_result {
        let _ = fs::remove_file(&archive_path);
        let _ = fs::remove_dir_all(&extracted_dir);
        return Err(format!(
            "Failed to unpack cached archive {}: {err}",
            archive_path.display()
        )
        .into());
    }

    if !lib_dir.is_dir() {
        return Err(format!(
            "Downloaded archive did not contain a lib directory: {}",
            lib_dir.display()
        )
        .into());
    }

    // V-Trim delta 4, fresh-extraction half. The archive's hash was checked just above, so this
    // cannot realistically fail -- which is why it is worth having: it is the one check that also
    // covers a cache that was populated by hand.
    assert_placeholders_are_placeholders(&lib_dir)?;

    eprintln!("Extracted TTS-off sherpa-onnx libs to {}", extracted_dir.display());

    Ok(lib_dir)
}

// ---- V-Trim additions: pinned-hash verification and the placeholder assertion ----------------

/// Lowercase hex SHA-256 of a file, streamed so a 106 MiB archive does not land in memory.
fn sha256_of_file(path: &Path) -> Result<String, DynError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = io::Read::read(&mut file, &mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Refuse anything that is not the pinned TTS-off archive. Both the download and a local
/// SHERPA_ONNX_ARCHIVE_DIR copy go through here, so a swapped archive cannot build either way.
fn verify_pinned_sha256(path: &Path) -> Result<(), DynError> {
    let found = sha256_of_file(path)?;
    if found == PINNED_ARCHIVE_SHA256 {
        return Ok(());
    }

    Err(format!(
        "{} has the wrong sha256.\n  expected: {PINNED_ARCHIVE_SHA256}\n  actual:   {found}\n\
         That is not the TTS-off archive this project pins. Linking it would put GPL \
         espeak-ng back into the binary. Refusing to build.",
        path.display()
    )
    .into())
}

/// The three stub libs must exist -- SHERPA_ONNX_STATIC_LIBS names them unconditionally, so the
/// files have to be there for the link to resolve -- and must still be placeholders. A real
/// espeak-ng.lib is 823,674 bytes; the placeholder is 896.
fn assert_placeholders_are_placeholders(lib_dir: &Path) -> Result<(), DynError> {
    for name in STUB_LIBS {
        let path = lib_dir.join(name);
        if !path.is_file() {
            return Err(format!(
                "{name} is missing from {}.\n\
                 That directory is not a TTS-off lib set. Delete it and rebuild so the archive is \
                 re-extracted from the pinned source.",
                lib_dir.display()
            )
            .into());
        }

        let len = fs::metadata(&path)?.len();
        if len > STUB_MAX_BYTES {
            return Err(format!(
                "{} is {len} bytes, expected at most {STUB_MAX_BYTES}.\n\
                 That is the real upstream library, not an empty placeholder: this lib set came \
                 from the official archive, which links GPL-3.0-or-later espeak-ng.\n\
                 Delete {} and rebuild -- the archive will be re-extracted from the pinned \
                 TTS-off source.",
                path.display(),
                lib_dir.parent().unwrap_or(lib_dir).display()
            )
            .into());
        }
    }

    Ok(())
}

fn archive_name(
    link_mode: LinkMode,
    target_os: &str,
    target_arch: &str,
) -> Result<String, DynError> {
    let version = env!("CARGO_PKG_VERSION");
    let name = match (link_mode, target_os, target_arch) {
        (LinkMode::Static, "linux", "x86_64") => {
            format!("sherpa-onnx-v{version}-linux-x64-static-lib.tar.bz2")
        }
        (LinkMode::Static, "linux", "aarch64") => {
            format!("sherpa-onnx-v{version}-linux-aarch64-static-lib.tar.bz2")
        }
        (LinkMode::Static, "macos", "x86_64") => {
            format!("sherpa-onnx-v{version}-osx-x64-static-lib.tar.bz2")
        }
        (LinkMode::Static, "macos", "aarch64") => {
            format!("sherpa-onnx-v{version}-osx-arm64-static-lib.tar.bz2")
        }
        (LinkMode::Static, "windows", "x86_64") => {
            format!("sherpa-onnx-v{version}-win-x64-static-MT-Release-lib.tar.bz2")
        }
        (LinkMode::Shared, "linux", "x86_64") => {
            format!("sherpa-onnx-v{version}-linux-x64-shared-lib.tar.bz2")
        }
        (LinkMode::Shared, "linux", "aarch64") => {
            format!("sherpa-onnx-v{version}-linux-aarch64-shared-cpu-lib.tar.bz2")
        }
        (LinkMode::Shared, "macos", "x86_64") => {
            format!("sherpa-onnx-v{version}-osx-x64-shared-lib.tar.bz2")
        }
        (LinkMode::Shared, "macos", "aarch64") => {
            format!("sherpa-onnx-v{version}-osx-arm64-shared-lib.tar.bz2")
        }
        (LinkMode::Shared, "windows", "x86_64") => {
            format!("sherpa-onnx-v{version}-win-x64-shared-MT-Release-lib.tar.bz2")
        }
        _ => return Err(format!(
            "Unsupported target for sherpa-onnx prebuilt libs: os={target_os}, arch={target_arch}"
        )
        .into()),
    };

    Ok(name)
}

fn emit_shared_link_directives() {
    println!("cargo:rustc-link-lib=dylib=sherpa-onnx-c-api");
    println!("cargo:rustc-link-lib=dylib=onnxruntime");
}

fn emit_static_link_directives(target_os: &str) {
    for lib in SHERPA_ONNX_STATIC_LIBS {
        println!("cargo:rustc-link-lib=static={lib}");
    }

    match target_os {
        "linux" => {
            println!("cargo:rustc-link-lib=dylib=stdc++");
            println!("cargo:rustc-link-lib=dylib=m");
            println!("cargo:rustc-link-lib=dylib=pthread");
            println!("cargo:rustc-link-lib=dylib=dl");
        }
        "macos" => {
            println!("cargo:rustc-link-lib=dylib=c++");
            println!("cargo:rustc-link-lib=framework=Foundation");
        }
        _ => {}
    }
}

fn target_dir_from_out_dir(out_dir: &Path) -> Result<PathBuf, DynError> {
    if let Ok(explicit_target_dir) = env::var("CARGO_TARGET_DIR") {
        return Ok(PathBuf::from(explicit_target_dir));
    }

    if let Some(target_dir) = out_dir
        .ancestors()
        .find(|path| path.file_name() == Some(OsStr::new("target")))
    {
        return Ok(target_dir.to_path_buf());
    }

    Ok(out_dir.to_path_buf())
}

fn emit_relative_rpath(target_os: &str) {
    match target_os {
        "linux" => println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN"),
        "macos" => println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path"),
        _ => {}
    }
}

fn profile_output_dirs() -> Result<[PathBuf; 2], DynError> {
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let profile = env::var("PROFILE")?;
    let profile_dir = out_dir
        .ancestors()
        .find(|path| path.file_name() == Some(OsStr::new(&profile)))
        .ok_or_else(|| {
            format!(
                "Could not locate Cargo profile directory from {}",
                out_dir.display()
            )
        })?
        .to_path_buf();

    Ok([profile_dir.clone(), profile_dir.join("examples")])
}

fn copy_unix_runtime_libs(lib_dir: &Path, target_os: &str) -> Result<(), DynError> {
    let runtime_libs: Vec<PathBuf> = fs::read_dir(lib_dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.file_name()
                .and_then(OsStr::to_str)
                .map(|name| match target_os {
                    "linux" => name.contains(".so"),
                    "macos" => name.ends_with(".dylib"),
                    _ => false,
                })
                .unwrap_or(false)
        })
        .collect();

    if runtime_libs.is_empty() {
        return Err(format!(
            "No shared runtime libraries found in {}",
            lib_dir.display()
        )
        .into());
    }

    let mut copy_plan = Vec::<(PathBuf, OsString)>::new();
    let mut planned_names = HashSet::<OsString>::new();

    for lib in runtime_libs {
        if !lib.exists() {
            continue;
        }

        let lib_name = lib
            .file_name()
            .ok_or_else(|| format!("Invalid runtime library path: {}", lib.display()))?
            .to_os_string();

        let source = fs::canonicalize(&lib).unwrap_or(lib.clone());
        if planned_names.insert(lib_name.clone()) {
            copy_plan.push((source.clone(), lib_name));
        }

        if let Some(source_name) = source.file_name() {
            let source_name = source_name.to_os_string();
            if planned_names.insert(source_name.clone()) {
                copy_plan.push((source.clone(), source_name));
            }
        }
    }

    if copy_plan.is_empty() {
        return Err(format!(
            "No usable shared runtime libraries found in {}",
            lib_dir.display()
        )
        .into());
    }

    for dest_dir in profile_output_dirs()? {
        fs::create_dir_all(&dest_dir)?;
        for (source, dest_name) in &copy_plan {
            let dest = dest_dir.join(dest_name);
            fs::copy(source, &dest)?;
        }
    }

    Ok(())
}

fn temp_path_for(path: &Path) -> PathBuf {
    let mut temp_name = path
        .file_name()
        .map(OsStr::to_os_string)
        .unwrap_or_else(|| OsString::from("tmp"));
    temp_name.push(".part");
    path.with_file_name(temp_name)
}

fn copy_file_atomically(src: &Path, dst: &Path) -> Result<(), DynError> {
    let temp_path = temp_path_for(dst);
    if temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }
    fs::copy(src, &temp_path)?;
    fs::rename(&temp_path, dst)?;
    Ok(())
}

fn write_reader_atomically(reader: &mut dyn io::Read, dst: &Path) -> Result<(), DynError> {
    let temp_path = temp_path_for(dst);
    if temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }

    {
        let mut file = File::create(&temp_path)?;
        io::copy(reader, &mut file)?;
        file.sync_all()?;
    }

    fs::rename(&temp_path, dst)?;
    Ok(())
}

fn copy_windows_runtime_dlls(lib_dir: &Path) -> Result<(), DynError> {
    let dlls: Vec<PathBuf> = fs::read_dir(lib_dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension() == Some(OsStr::new("dll")))
        .collect();

    if dlls.is_empty() {
        println!(
            "cargo:warning=No runtime DLLs found in {}",
            lib_dir.display()
        );
        return Ok(());
    }

    let [profile_dir, examples_dir] = profile_output_dirs()?;
    for dest_dir in [profile_dir.clone(), examples_dir] {
        fs::create_dir_all(&dest_dir)?;
        for dll in &dlls {
            let dest = dest_dir.join(
                dll.file_name()
                    .ok_or_else(|| format!("Invalid DLL path: {}", dll.display()))?,
            );
            fs::copy(dll, &dest)?;
        }
    }

    println!(
        "cargo:warning=Copied Windows runtime DLLs to {} and {}/examples",
        profile_dir.display(),
        profile_dir.display()
    );

    Ok(())
}
