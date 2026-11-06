//! **R4 老工程 round-trip 金样判据**（P2 的前置安全网，2026-10）。
//!
//! # 为什么需要它
//!
//! R4 红线是「**既有工程产物逐字节不变**」。词汇表门禁
//! （`scripts/check-effect-registry.mjs`）管的是"类型/字段在两处实现之间一致"，
//! **管不了"老文件读进来再写出去是否还是那个文件"**。
//! P2 要把事件数据模型从「12 个枚举变体各带自己的字段」改成「公共头 + `kind` + `params`」，
//! **那正是最容易破坏 R4 的一类改动**：兼容写漏一个 `serde(default)`、
//! 或少一个 `skip_serializing_if`，症状都是"老工程一存就多一个字段"或"少一个字段"——
//! **不报错**，只是文件变了。
//!
//! # 金样口径
//!
//! `tests/fixtures/r4-legacy-demo.toml` 取自仓库里**已入库**的演示工程
//! （`docs/wgpu-effect-reference/demo/polish.toml`，233 行、10 种事件类型）。
//!
//! ⚠️ **2026-10-30 从金样里摘掉了 `pulse` / `hue_shift` 两个事件**：它们已分别退役
//! （`pulse` 读时丢弃、`hue_shift` 读时迁移成 `overlay{hue}`），金样若继续带着它们，
//! 本文件就会把"有意退役"误报成"事件被静默丢弃"（字节稳定那条也会随之失配）。
//! 退役本身有专测：`project::legacy::tests`（丢弃/迁移/计数/幂等）。金样只负责
//! **仍然活着的老写法**（`duration` / `entrance` / 键集稳定 / 两次写盘逐字节相同）。
//! 它特意是**老写法**：事件用 `duration`（**没有 `time`+`end`**），
//! 且 `[style.subtitle]` 里带着**已下线的 `entrance`** ——
//! 于是它一次钉住三件事：① `duration` 升级；② 已下线键被吞掉；
//! ③ 回写产物的**键集**稳定。
//!
//! ⚠️ **必须走 `load_project` 这条真读盘路径**，不能自己 `toml::from_str::<ProjectFile>`：
//! 后者会**绕过** `legacy::upgrade_toml_value`（`duration` 升级 + 吞 `entrance`）与
//! `config_version` 剥离 —— 那正是这个金样里最容易漏的两段。
//! （第一版就是这么写的，当场红在 `unknown field \`entrance\``：判据**自己**先撞上了这个坑。）
//!
//! # 判据（四条）
//!
//! 1. **可解析**：金样走真读盘路径能读进来。
//! 2. **`duration` 升级到位**：读出来每个事件的 `end - time` 与原文 `duration` 逐值一致。
//! 3. **回写键集稳定**：`to_toml_commented()` 产物里**不出现 `duration`**、**出现 `end`**、
//!    事件类型集合与原文一致、**不出现已下线的 `entrance`**。
//! 4. **幂等（真正的字节稳定）**：把产物再读一次、再写一次，**两次逐字节相同**。
//!    这一条保证"用户点一次保存"不会产生无意义 diff。

use std::collections::BTreeSet;
use std::path::PathBuf;

use yekitrim_polish::project::{load_project, ProjectFile};
use yekitrim_render_core::types::Event;

/// 金样原文（`crates/yekitrim-polish/tests/fixtures/`）。
fn golden_text() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/r4-legacy-demo.toml");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("读不到金样 {}：{e}", path.display()))
}

/// 把金样落进一个临时工程目录，走**真读盘路径**读回来。
///
/// 返回 `(工程, 临时目录)` —— 临时目录留给调用方兜着，函数返回后由调用方清理。
fn load_golden(tag: &str) -> (ProjectFile, PathBuf) {
    let dir = std::env::temp_dir().join(format!("yekitrim-r4-roundtrip-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时工程目录");
    std::fs::write(dir.join("polish.toml"), golden_text()).expect("写金样");
    let cfg = load_project(&dir).expect("R4 金样必须能走真读盘路径解析");
    (cfg, dir)
}

/// 事件类型名（与 `process::quality::event_kind` 同口径的**只读**副本）。
///
/// ⚠️ 刻意**不复用**那个函数（它是私有的 `fn`，且语义是"哪些事件画在同一视觉通道上"）。
/// 这里要的是"金样原文里写的那个 `type` 值"，所以 `element` 一律回 `"element"`
/// （原文的 `type` 字面值就是它，不按内部 `kind` 再分）。金样里没有 `element`
/// 事件，这条分支只是把 match 写全。
fn event_type_name(ev: &Event) -> &'static str {
    match ev {
        Event::Sticker { .. } => "sticker",
        Event::Camera { .. } => "camera",
        Event::Shake { .. } => "shake",
        Event::Flash { .. } => "flash",
        Event::Blur { .. } => "blur",
        Event::Vignette { .. } => "vignette",
        Event::HueShift { .. } => "hue_shift",
        Event::Noise { .. } => "noise",
        Event::Pulse { .. } => "pulse",
        Event::Overlay { .. } => "overlay",
        Event::Sfx { .. } => "sfx",
        Event::Element { .. } => "element",
    }
}

/// 从金样原文**文本级**扒出 `(类型, duration)` 清单。
///
/// ⚠️ 刻意不走结构体：走结构体就**自己证自己**了（解析错了的话两边一起错、判据照样绿）。
fn golden_event_spans() -> Vec<(String, f64)> {
    let raw = golden_text();
    let mut out: Vec<(String, f64)> = Vec::new();
    let mut ty: Option<String> = None;
    let mut dur: Option<f64> = None;
    for line in raw.lines() {
        let line = line.trim();
        if line == "[[segments.events]]" {
            if let (Some(t), Some(d)) = (ty.take(), dur.take()) {
                out.push((t, d));
            }
        } else if let Some(v) = line.strip_prefix("type = ") {
            ty = Some(v.trim().trim_matches('"').to_string());
        } else if let Some(v) = line.strip_prefix("duration = ") {
            dur = v.trim().parse::<f64>().ok();
        }
    }
    if let (Some(t), Some(d)) = (ty.take(), dur.take()) {
        out.push((t, d));
    }
    out
}

/// 金样里的事件类型集合（文本级）。
fn golden_types() -> BTreeSet<String> {
    golden_event_spans().into_iter().map(|(t, _)| t).collect()
}

#[test]
fn r4_golden_loads_and_keeps_every_event() {
    let (cfg, dir) = load_golden("loads");
    let spans = golden_event_spans();

    // 金样本身够不够"厚"：太少就钉不住东西。
    assert!(
        spans.len() >= 10,
        "金样应当有多条事件（实测 {} 条）—— 太少就钉不住东西了",
        spans.len()
    );
    assert!(
        golden_types().len() >= 5,
        "金样应当覆盖多种事件类型（实测 {} 种）",
        golden_types().len()
    );

    let parsed: usize = cfg.segments.iter().map(|s| s.events.len()).sum();
    assert_eq!(
        parsed,
        spans.len(),
        "结构与原文的事件条数必须一致（少了 = 有事件被静默丢弃）"
    );

    // 已下线的 `entrance` 必须被读时吞掉（真读盘路径才会做这件事）。
    // 它没被吞的话，上面那句 `load_project` 就已经红了 —— 这条断言是**点名**：
    // 万一以后有人往 `SubtitleStyle` 里真加回一个叫 `entrance` 的字段，这里会立刻发现口径变了。
    let out = cfg.to_toml_commented().expect("序列化不该失败");
    assert!(
        !out.contains("entrance"),
        "已下线的 `entrance` 不许再出现在回写产物里\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn r4_legacy_duration_upgrades_to_end() {
    let (cfg, dir) = load_golden("duration");
    let spans = golden_event_spans();
    let flat: Vec<&Event> = cfg.segments.iter().flat_map(|s| s.events.iter()).collect();
    assert_eq!(flat.len(), spans.len());

    for (ev, (ty, want_dur)) in flat.iter().zip(spans.iter()) {
        assert_eq!(
            event_type_name(ev),
            ty.as_str(),
            "事件类型顺序/取值与金样原文不一致"
        );
        let end = ev
            .end()
            .unwrap_or_else(|| panic!("`{ty}` 事件读出来后必须有 `end`（legacy 升级没生效？）"));
        let got = end - ev.get_time();
        assert!(
            (got - want_dur).abs() < 1e-9,
            "`{ty}` 的跨度升级错了：原文 duration={want_dur}、算出 {got}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn r4_reserialize_is_byte_stable() {
    let (cfg, dir) = load_golden("stable");
    let out = cfg.to_toml_commented().expect("序列化不该失败");

    // 判据 3：老字段不许再出现；新口径必须在；类型集合一致。
    //
    // ⚠️ 只在**事件段**里查 `duration` —— 顶层也有一个 `duration`
    //    （`ProjectFile.duration` = 工程总时长，与事件的 `time`+`end` 无关）。
    //    第一版写成裸 `out.contains("duration =")`，当场被那个顶层键打红：
    //    判据**过宽**会报假红，与过窄一样是坏的。
    let event_duration = out
        .split("[[segments.events]]")
        .skip(1)
        .any(|chunk| chunk.contains("duration ="));
    assert!(
        !event_duration,
        "**事件**里不许再出现 `duration`（R4：`time`+`end` 是唯一口径）\n{out}"
    );
    assert!(out.contains("end ="), "回写产物里必须有 `end`");

    let dst: BTreeSet<String> = cfg
        .segments
        .iter()
        .flat_map(|s| s.events.iter())
        .map(|e| event_type_name(e).to_string())
        .collect();
    assert_eq!(golden_types(), dst, "事件类型集合在读-写之间变了");

    // 判据 4（真正的"字节稳定"）：再读再写，两次产物逐字节相同。
    // 这里模拟"用户连点两次保存"。
    let dir2 = std::env::temp_dir().join("yekitrim-r4-roundtrip-stable-2");
    let _ = std::fs::remove_dir_all(&dir2);
    std::fs::create_dir_all(&dir2).expect("建第二个临时目录");
    std::fs::write(dir2.join("polish.toml"), &out).expect("写第一次产物");
    let back = load_project(&dir2).expect("自己的产物必须能回读");
    let out2 = back.to_toml_commented().expect("二次序列化不该失败");

    let _ = std::fs::remove_dir_all(&dir2);
    let _ = std::fs::remove_dir_all(&dir);
    if out != out2 {
        let a: Vec<&str> = out.lines().collect();
        let b: Vec<&str> = out2.lines().collect();
        let mut msg = String::from("R4 字节不稳定，逐行差异：\n");
        for i in 0..a.len().max(b.len()) {
            let (x, y) = (a.get(i).copied(), b.get(i).copied());
            if x != y {
                msg.push_str(&format!("  行{}:\n    第一次: {:?}\n    第二次: {:?}\n", i + 1, x, y));
            }
        }
        panic!("{msg}");
    }
}
