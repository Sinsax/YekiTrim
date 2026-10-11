//! 字级时间戳搬运的**端到端**判据（E4b）。
//!
//! 规格与纪律见 `src/parse/charlog.rs` 模块头与 `src/commands.rs::sync_char_timeline`。
//!
//! 三条判据，都是"真跑 `cmd_init`"而不是只测纯函数：
//!
//! 1. **有 `timeline.json` ⇒ 读得到字级时间**（落 `data/char-timeline.json`，逐字对得上）；
//! 2. **删掉那个文件 ⇒ 行为与今天逐字节相同**（R4）：不产生 `data/char-timeline.json`，
//!    且 `data/` 下**其余产物逐字节相同**、文件集合相同（一个都不多、一个都不少）；
//! 3. 缺文件**不阻断 init**（`cmd_init` 返回 Ok）。
//!
//! ⚠️ 样本 `timeline.json` 是**按 `asr::artifacts::TimelineRecord` 的结构造的**，
//! 不是真 ASR 链路产出的（真跑要模型 + 音频，成本高）。字段名/类型有单测钉住
//! （`parse::charlog::tests::matches_timeline_record_shape`）。

use std::path::{Path, PathBuf};

fn tmp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "vtedit-charlog-e2e-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// 造一个最小可跑的片段目录：`clip.mp4`（cmd_init 认它）+ `clip.srt` + `clip.json`。
///
/// 视频是**真**的（ffmpeg 生成 1 秒黑场）—— `cmd_init` 会探测素材、`cmd_parse` 会 ffprobe
/// 时长，假文件会让这两步走不同分支，测出来的就不是真实行为了。
fn make_clip(root: &Path) -> PathBuf {
    // 规范布局：<工程根>/output/clips/<名>
    let clip = root.join("output").join("clips").join("clip-01");
    std::fs::create_dir_all(&clip).expect("建片段目录");

    let mp4 = clip.join("clip.mp4");
    let st = yekitrim_core::process::cmd("ffmpeg")
        .args([
            "-y", "-f", "lavfi", "-i", "color=c=black:s=320x240:d=1",
            "-f", "lavfi", "-i", "anullsrc=r=16000:cl=mono", "-shortest",
            "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac",
        ])
        .arg(&mp4)
        .status()
        .expect("需要 ffmpeg");
    assert!(st.success(), "生成测试视频失败");

    std::fs::write(
        clip.join("clip.srt"),
        "\u{feff}1\n00:00:00,000 --> 00:00:00,500\n你好\n\n2\n00:00:00,500 --> 00:00:01,000\n世界\n",
    )
    .expect("写字幕");
    std::fs::write(clip.join("clip.json"), r#"{"duration":1.0,"title":"t","event_type":"x"}"#)
        .expect("写 clip.json");
    clip
}

/// 造 ASR 的 `asr/timeline.json`（**规范位置**：片段目录上两级 + `asr/`）。
fn write_timeline(clip: &Path, text: &str) {
    let asr = clip.parent().unwrap().parent().unwrap().join("asr");
    std::fs::create_dir_all(&asr).expect("建 asr/");
    let times: Vec<f64> = (0..text.chars().count()).map(|i| i as f64 / 10.0).collect();
    let record = serde_json::json!({
        "text": text,
        "chars": text.chars().count(),
        "times": times,
        "breath_gap": 0.4,
        "breaths": [{ "index": 2, "time": 0.2, "gap": 0.4 }],
    });
    std::fs::write(
        asr.join("timeline.json"),
        serde_json::to_string_pretty(&record).unwrap(),
    )
    .expect("写 timeline.json");
}

/// `data/` 下所有文件的 (文件名 → 内容) —— R4 对拍用的快照。
fn data_snapshot(clip: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = std::fs::read_dir(clip.join("data"))
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_file())
                .map(|e| {
                    (
                        e.file_name().to_string_lossy().to_string(),
                        std::fs::read(e.path()).unwrap_or_default(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[tokio::test]
async fn timeline_is_carried_into_project_data() {
    let root = tmp_root("carry");
    let clip = make_clip(&root);
    write_timeline(&clip, "你好世界");

    yekitrim_polish::commands::cmd_init(&clip)
        .await
        .expect("cmd_init 必须成功");

    let path = clip.join("data").join("char-timeline.json");
    assert!(path.is_file(), "有 timeline.json 就该搬出来: {}", path.display());

    let raw = std::fs::read_to_string(&path).unwrap();
    let log: yekitrim_polish::parse::charlog::CharTimelineLog =
        serde_json::from_str(&raw).expect("搬过来的必须是可回读的同一形状");
    assert_eq!(log.text, "你好世界");
    assert_eq!(log.times.len(), 4);
    assert_eq!(log.breaths.len(), 1, "气口也一起搬");

    // 真接上字幕：两条字幕各拿到自己的逐字时间
    let subs = vec![
        yekitrim_polish::parse::srt::Subtitle {
            index: 1, start: 0.0, end: 0.5, text: "你好".into(), html: String::new(),
        },
        yekitrim_polish::parse::srt::Subtitle {
            index: 2, start: 0.5, end: 1.0, text: "世界".into(), html: String::new(),
        },
    ];
    let aligned = yekitrim_polish::parse::charlog::align_chars_to_subtitles(&log, &subs);
    assert_eq!(aligned.len(), 2, "两条字幕都该拿到字级时间: {aligned:?}");
    assert_eq!(aligned[0].times, vec![0.0, 0.1]);
    assert_eq!(aligned[1].times, vec![0.2, 0.3]);

    let _ = std::fs::remove_dir_all(&root);
}

/// ⚠️ **R4 的核心判据**：删掉 `asr/timeline.json` ⇒ 行为与今天**逐字节相同**。
///
/// 两个工程（有 / 没有 timeline.json）跑完 `cmd_init` 后：
/// ① 没有的那个**不许**有 `data/char-timeline.json`（文件集合逐项相等）；
/// ② `data/` 下**共有文件的内容逐字节相同**（`subtitles.json` / `danmaku.json` /
///    `clip-meta.json` —— 这三个是今天的产物，一个字节都不许变）。
#[tokio::test]
async fn without_timeline_behaviour_is_byte_identical() {
    let root_a = tmp_root("with");
    let root_b = tmp_root("without");
    let clip_a = make_clip(&root_a);
    let clip_b = make_clip(&root_b);
    write_timeline(&clip_a, "你好世界");
    // clip_b 故意不写 timeline.json —— 存量工程的常态

    yekitrim_polish::commands::cmd_init(&clip_a).await.expect("有 timeline 也要成功");
    yekitrim_polish::commands::cmd_init(&clip_b).await.expect("缺 timeline 不许阻断 init");

    let a = data_snapshot(&clip_a);
    let b = data_snapshot(&clip_b);

    // ① 文件集合：b **不许**多出 char-timeline.json
    let names_a: Vec<&str> = a.iter().map(|(n, _)| n.as_str()).collect();
    let names_b: Vec<&str> = b.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names_a.contains(&"char-timeline.json"), "有数据的工程该有它: {names_a:?}");
    assert!(
        !names_b.contains(&"char-timeline.json"),
        "⚠️ 没数据**一个字节都不许多**（落空文件 = R4 破）: {names_b:?}"
    );
    let mut names_a_sorted = names_a.clone();
    let mut names_b_sorted = names_b.clone();
    names_a_sorted.sort_unstable();
    names_b_sorted.sort_unstable();
    let a_without_new: Vec<&str> = names_a_sorted
        .iter()
        .copied()
        .filter(|n| *n != "char-timeline.json")
        .collect();
    assert_eq!(
        a_without_new,
        names_b_sorted,
        "除新增的那一份外，文件集合必须逐项相等"
    );

    // ② 共有文件的内容逐字节相同
    for (name, bytes_b) in &b {
        let (_, bytes_a) = a
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("A 里缺 {name}"));
        assert_eq!(bytes_a, bytes_b, "⚠️ {name} 的内容必须逐字节相同（R4）");
    }

    // ③ 而且真的读不到字级时间（不是"文件在但内容空"）
    assert!(
        yekitrim_polish::parse::charlog::load_char_timeline(&clip_b).is_none(),
        "没有 asr/timeline.json ⇒ 就是没有"
    );

    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_b);
}

/// 重跑 `parse` 是**幂等**的：内容没变就不写（mtime 不动）—— 首页按目录最新 mtime 排序，
/// 无条件重写会把老片段一直顶到最前面。
#[tokio::test]
async fn resync_is_idempotent_and_does_not_touch_mtime() {
    let root = tmp_root("idem");
    let clip = make_clip(&root);
    write_timeline(&clip, "你好世界");

    yekitrim_polish::commands::cmd_init(&clip).await.expect("init");
    let path = clip.join("data").join("char-timeline.json");
    let first = std::fs::metadata(&path).unwrap().modified().unwrap();
    let bytes = std::fs::read(&path).unwrap();

    std::thread::sleep(std::time::Duration::from_millis(1100));
    yekitrim_polish::commands::cmd_parse(&clip).await.expect("重跑 parse");

    assert_eq!(std::fs::read(&path).unwrap(), bytes, "内容必须一样");
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        first,
        "⚠️ 没变化就不许重写（mtime 一动，片段会被顶到列表最前）"
    );

    // 数据源**真的变了** ⇒ 必须跟着更新（否则"幂等"就成了"永远不更新"）
    write_timeline(&clip, "你好世界再见");
    yekitrim_polish::commands::cmd_parse(&clip).await.expect("改过之后再 parse");
    let log: yekitrim_polish::parse::charlog::CharTimelineLog =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(log.text, "你好世界再见", "源变了必须跟上");

    let _ = std::fs::remove_dir_all(&root);
}
