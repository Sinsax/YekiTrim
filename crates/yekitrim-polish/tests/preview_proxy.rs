//! **预览代理的判据**：`polish init` 生成的那份 720p / GOP10 副本。
//!
//! 判据要钉住的是**四条能力**，每条都对应一个"看起来没问题、其实白做/做坏"的形态：
//!
//! 1. **分辨率真的降了**（源 1080p ⇒ 代理 720p）：不降就是白占磁盘。
//! 2. **帧率保持源**（30fps 进 ⇒ 30fps 出）：时基一变，工程里的帧号映射就错
//!    （转译器按**实际媒体**探测时基，代理是它读的那份）。
//! 3. **GOP 真的变密了**（关键帧数 ≪ 帧数）：退回全 I 帧的话，体积是源的 2.2–4.2×
//!    —— 而容量在本地/服务器两种模式下都是约束（见 `render/proxy.rs` 模块头的实测表）。
//! 4. **幂等 + 过期回落**：重复 init 不重压；源被重切过（更新）⇒ 代理作废、回落源。
//!
//! 音轨也要查（`-c:a copy`）：预览的声音来自这条 `<video>`，代理没声音就是"画面对了、声音没了"。
//!
//! 没有 ffmpeg/ffprobe 就跳过（打印原因，不让门禁红）—— 与 `takumi_font_id.rs` 同一条纪律。

use std::path::{Path, PathBuf};
use std::process::Command;

fn have(bin: &str) -> bool {
    Command::new(bin)
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn ff(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("ffmpeg")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("ffmpeg 起不来")
}

/// `ffprobe -show_entries <entries>`，返回 `key=value` 的 Map。
fn probe(path: &Path, stream: &str, entries: &str) -> std::collections::HashMap<String, String> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            stream,
            "-show_entries",
            entries,
            "-of",
            "default=nw=1",
        ])
        .arg(path)
        .output()
        .expect("ffprobe 起不来");
    let mut map = std::collections::HashMap::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some((k, v)) = line.split_once('=') {
            map.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    map
}

fn mtime(p: &Path) -> std::time::SystemTime {
    std::fs::metadata(p).expect("metadata").modified().expect("mtime")
}

#[test]
fn preview_proxy_is_720p_and_keeps_fps_and_dense_keyframes() {
    if !have("ffmpeg") || !have("ffprobe") {
        eprintln!("跳过：这台机器没有 ffmpeg/ffprobe");
        return;
    }
    let dir = std::env::temp_dir().join(format!("vt-preview-proxy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");

    // 源：1920x1080 / 30fps / 2 秒 / 带音轨（GOP 故意放长 = 真实成片的样子）
    let out = ff(
        &dir,
        &[
            "-v", "error", "-y",
            "-f", "lavfi", "-i", "testsrc2=size=1920x1080:rate=30",
            "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000",
            "-t", "2", "-shortest",
            "-c:v", "libx264", "-preset", "ultrafast", "-g", "60", "-pix_fmt", "yuv420p",
            "-c:a", "aac", "clip.mp4",
        ],
    );
    assert!(out.status.success(), "造源失败：{}", String::from_utf8_lossy(&out.stderr));

    // ① 生成
    let proxy = yekitrim_polish::render::proxy::ensure_preview_proxy_with(&dir, "clip.mp4", true)
        .expect("生成不该报错");
    let proxy: PathBuf = proxy.expect("应当生成代理");
    assert!(proxy.is_file(), "代理文件不存在：{}", proxy.display());

    // ② 分辨率真的降了（源 1080p，上限 720）
    let (w, h) = yekitrim_polish::render::get_video_size(&proxy.to_string_lossy())
        .expect("代理应当能被 ffprobe 读出尺寸");
    assert_eq!(h, 720, "代理高度必须是 720（源 1080p）");
    assert!(w > 0 && w % 2 == 0, "宽度要是偶数（yuv420p），实得 {w}");

    // ③ 帧率保持源（30fps 进 ⇒ 30fps 出）：`-fps_mode passthrough`。
    //    这条红了 = 预览的帧号映射会错（工程时基是按**代理**探测的）。
    let v = probe(&proxy, "v:0", "stream=avg_frame_rate,nb_frames,r_frame_rate");
    let rate = v.get("avg_frame_rate").or_else(|| v.get("r_frame_rate")).cloned().unwrap_or_default();
    assert_eq!(rate, "30/1", "代理必须保持源帧率（实得 {rate}）");

    // ④ 音轨还在（`-c:a copy`）
    let a = probe(&proxy, "a:0", "stream=codec_name");
    assert_eq!(
        a.get("codec_name").map(String::as_str),
        Some("aac"),
        "代理必须带音轨（copy 源那份）"
    );

    // ⑤ 关键帧是**密**，不是全 I 帧：2 秒 30fps = 60 帧，GOP10 ⇒ 约 6 个关键帧。
    //    全 I 帧会给出 60 —— 而全 I 帧的体积是源的 2.2–4.2×（容量在这里是硬约束）。
    let kf = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "frame=key_frame", "-of", "csv=p=0"])
        .arg(&proxy)
        .output()
        .expect("ffprobe");
    let total = String::from_utf8_lossy(&kf.stdout).lines().filter(|l| !l.trim().is_empty()).count();
    let keys = String::from_utf8_lossy(&kf.stdout).lines().filter(|l| l.trim() == "1").count();
    assert!(total >= 40, "探到的帧数太少（{total}），判据不可信");
    assert!(
        keys * 5 <= total,
        "关键帧太密（{keys}/{total}）—— 退回全 I 帧的话体积会涨到源的 2.2–4.2 倍"
    );

    // ⑥ 容量：代理不大于源（真内容实测 0.52×；合成图案的比值没那么漂亮，所以用 ≤）
    let src_len = std::fs::metadata(dir.join("clip.mp4")).unwrap().len();
    let proxy_len = std::fs::metadata(&proxy).unwrap().len();
    assert!(
        proxy_len <= src_len,
        "代理比源还大（{proxy_len} > {src_len}）—— 换档位（分辨率/GOP/CRF）而不是硬塞"
    );

    // ⑦ 幂等：再跑一次不该重压（mtime 不变）
    let before = mtime(&proxy);
    let again = yekitrim_polish::render::proxy::ensure_preview_proxy_with(&dir, "clip.mp4", true)
        .expect("第二次也不该报错")
        .expect("第二次仍应返回代理");
    assert_eq!(again, proxy);
    assert_eq!(before, mtime(&proxy), "已是最新的代理不该被重压一遍");

    // ⑧ 可跳过（`enabled = false`，`VTEDIT_NO_PROXY` 走的就是这条）
    let skipped = yekitrim_polish::render::proxy::ensure_preview_proxy_with(&dir, "clip.mp4", false)
        .expect("跳过不该报错");
    assert!(skipped.is_none(), "关掉时不该生成");

    // ⑨ 新鲜度：源被重切过（更新）⇒ 代理作废、预览回落源
    assert_eq!(
        yekitrim_polish::render::proxy::preview_video_rel(&dir, "clip.mp4").as_deref(),
        Some("_proxy_720.mp4"),
        "源没变时应当用代理"
    );
    std::thread::sleep(std::time::Duration::from_millis(1100)); // 让 mtime 明确变大（FAT/低精度文件系统也稳）
    let out = ff(
        &dir,
        &[
            "-v", "error", "-y",
            "-f", "lavfi", "-i", "testsrc2=size=1920x1080:rate=30",
            "-t", "1", "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "clip.mp4",
        ],
    );
    assert!(out.status.success(), "重切源失败");
    assert!(
        yekitrim_polish::render::proxy::preview_video_rel(&dir, "clip.mp4").is_none(),
        "源更新过 ⇒ 旧代理必须作废（否则预览播的是上一版画面）"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// **`polish init` 真的会生成代理**（这条是"生成环节"的接线判据：`cmd_init` 里那一行没了就红）。
///
/// 用真 init 跑一遍：新目录 + `clip.json`（模拟 `yekitrim cut` 的产物）+ `clip.mp4`。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn polish_init_generates_the_preview_proxy() {
    if !have("ffmpeg") || !have("ffprobe") {
        eprintln!("跳过：这台机器没有 ffmpeg/ffprobe");
        return;
    }
    let dir = std::env::temp_dir().join(format!("vt-init-proxy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时目录");
    let out = ff(
        &dir,
        &[
            "-v", "error", "-y",
            "-f", "lavfi", "-i", "testsrc2=size=1920x1080:rate=30",
            "-t", "2", "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p",
            "clip.mp4",
        ],
    );
    assert!(out.status.success(), "造源失败");
    // `clip.json` 让 init 认出"这是 cut 出来的片段"，从而把 source.video 设成 clip.mp4
    std::fs::write(
        dir.join("clip.json"),
        r#"{"title":"proxy-test","duration":2.0}"#,
    )
    .unwrap();

    yekitrim_polish::commands::cmd_init(&dir)
        .await
        .expect("init 应当成功");
    let proxy = dir.join(yekitrim_polish::render::proxy::PREVIEW_PROXY_FILE);
    assert!(
        proxy.is_file(),
        "`polish init` 没生成预览代理（`cmd_init` 里那一行被摘了？）：{}",
        proxy.display()
    );
    let (_, h) = yekitrim_polish::render::get_video_size(&proxy.to_string_lossy()).expect("读代理尺寸");
    assert_eq!(h, 720, "init 生成的代理必须是 720p");

    // 生成的代理要能被预览那条腿选中
    assert_eq!(
        yekitrim_polish::render::proxy::preview_video_for(&dir).as_deref(),
        Some(yekitrim_polish::render::proxy::PREVIEW_PROXY_FILE),
        "init 生成之后，预览那条腿必须能选中它"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
