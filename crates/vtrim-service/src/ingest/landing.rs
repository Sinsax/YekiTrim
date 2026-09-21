//! 把一份**已经躺在项目目录里的素材**落成一个 V-Trim 项目。
//!
//! 从 `server::ingest_worker::ingest_into` 里抽出来的第 4 步（"建项目 + 登记"），
//! 因为这一步**与录播姬无关**：写 `vtrim.toml`、登记 `projects`、刷新占用、审计，
//! B站下载通道要的是同一套。抽出来的第二个理由同样是"两个 worker 不许互相调用"。
//!
//! ## 职责边界（刻意划得很窄）
//!
//! 本模块**只做"登记"**，不做：
//! - ❌ 搬文件 / 合并分片（录播姬的多分片合并是它自己的事）
//! - ❌ 容量预检（那是 [`super::capacity`]）
//! - ❌ 起管线（起管线要 `jobs` + `run_stages`，在 `vtrim-server::pipeline_boot`）
//!
//! 顺序上调用方应当是：**容量预检 → 素材就位 → [`land_project`] → 起管线**。
//!
//! ## 幂等性
//!
//! 同一个 `vtrim.toml` 重复落位不会建出第二个项目（先按 `config_file` 查，命中就复用）。
//! 这一条是"重试"能安全使用的前提：下载重试 / 录播姬重投都靠它。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::db::Db;
use crate::projects;

/// 一次落位要的全部信息。
///
/// 字段刻意保持"纯数据"：没有 `AppState`、没有 config 引用 —— 这样本模块可以被
/// 任何来源（录播姬 / B站 / 将来别的）复用，也能离线测。
#[derive(Debug, Clone)]
pub struct ProjectLanding<'a> {
    /// 项目归属账号。
    pub owner_id: i64,
    /// 项目显示名（列表页第一行）。
    pub name: String,
    /// 来源类型，写进 `projects.source_kind`：`"recorder"` / `"bilibili"` / `"local"` …
    ///
    /// ⚠️ 这个值同时是**预载策略**的键（`service::assets::preload_policy`），
    /// 写错不会报错 —— 只会让弹幕悄悄不预载、热度排序变差。
    pub source_kind: &'a str,
    /// 来源标签（列表页副标题），如「录播姬 · 房间 26320007」。
    pub source_label: Option<&'a str>,
    /// 写进 `projects.room`（**直播间号**）。
    ///
    /// ⚠️ B站来源**必须留 `None`**：这一列会被原样暴露给前端（`summary_json` 的 `"room"`、
    /// 预处理页的 `source.room`）当"房间"展示，塞 bvid 进去是误导。B站的来源标识走
    /// `ingest_sessions.source_ref` + 项目名/标签。
    pub room: Option<&'a str>,
    /// 项目根目录（**必须已经存在且素材已就位**）。
    pub project_root: &'a Path,
    /// `vtrim.toml` 里的 `input_file`（相对 `project_root` 的文件名）。
    pub input_file: &'a str,
    /// `vtrim.toml` 里的 `danmaku_file`（相对文件名；没有就 `None`）。
    pub danmaku_file: Option<&'a str>,
    /// 关联的采集会话（录播姬 session / B站下载会话）；`None` = 不要回写会话表。
    pub session_id: Option<&'a str>,
}

/// 落位结果。
#[derive(Debug, Clone)]
pub struct LandedProject {
    pub project_id: i64,
    /// 写好的 `vtrim.toml`（起管线要用）。
    pub config_path: PathBuf,
    /// `false` = 这个 `vtrim.toml` 早就登记过了，本次只做了会话回写。
    pub created: bool,
}

/// 写 `vtrim.toml` + 登记 `projects` + 刷新占用 + 审计 + 回写会话。
///
/// 用 [`write_project_config_exact`](vtrim_core::config::write_project_config_exact)
/// 而不是会自动发现同名 `.ass` 的那个：这里两侧的来源**都已经自己探测过弹幕**了
/// （录播姬 `find_danmaku` / B站下载），再让配置层去猜一遍只会多一层看不见的副作用。
pub fn land_project(db: &Db, l: &ProjectLanding<'_>) -> Result<LandedProject> {
    let config_path = l.project_root.join("vtrim.toml");
    vtrim_core::config::write_project_config_exact(
        &config_path,
        Path::new(l.input_file),
        l.danmaku_file.map(Path::new),
        None,
    )
    .context("写 vtrim.toml 失败")?;

    let project_id = match db.project_by_config(&config_path)? {
        // 同一个 vtrim.toml 已登记（重试 / 重投）→ 复用，不建第二个项目
        Some(p) => {
            if let Some(sid) = l.session_id {
                db.set_ingest_project(sid, p.id)?;
            }
            return Ok(LandedProject {
                project_id: p.id,
                config_path,
                created: false,
            });
        }
        None => db.create_project(
            l.owner_id,
            &l.name,
            l.room,
            l.source_kind,
            l.source_label,
            l.project_root,
            &config_path,
            l.session_id,
        )?,
    };

    // 占用（源 + 输出）立刻算一次：列表页/存储页不该等到下次巡检才准
    if let Ok(Some(p)) = db.project_by_id(project_id) {
        let _ = projects::refresh_bytes(db, &p);
    }
    if let Some(sid) = l.session_id {
        db.set_ingest_project(sid, project_id)?;
    }
    let _ = db.audit(
        Some(l.owner_id),
        "ingest_project",
        &format!(
            "source={} session={} project={project_id}",
            l.source_kind,
            l.session_id.unwrap_or("-")
        ),
    );
    Ok(LandedProject {
        project_id,
        config_path,
        created: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Db, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("vtrim.db")).unwrap();
        let root = dir.path().join("users/1/bili/BV1xx");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("source.mp4"), b"fake").unwrap();
        (dir, db, root)
    }

    fn landing<'a>(root: &'a Path, kind: &'a str, sid: Option<&'a str>) -> ProjectLanding<'a> {
        ProjectLanding {
            owner_id: 1,
            name: "测试项目".into(),
            source_kind: kind,
            source_label: Some("B站 · BV1xx"),
            room: None,
            project_root: root,
            input_file: "source.mp4",
            danmaku_file: Some("danmaku.xml"),
            session_id: sid,
        }
    }

    #[test]
    fn writes_config_and_registers_project_once() {
        let (dir, db, root) = fixture();
        let uid = db.create_user("u", "h", "user", false, 0).unwrap();
        let _ = uid;
        // 弹幕文件写进项目根，落位要把它记进 vtrim.toml
        std::fs::write(root.join("danmaku.xml"), "<i></i>").unwrap();

        let sid = "bili-BV1xx-p1";
        db.create_ingest_session(sid, "bilibili", 1, Some("BV1xx"), "downloading", Some("detect"))
            .unwrap();

        let mut l = landing(&root, "bilibili", Some(sid));
        let first = land_project(&db, &l).unwrap();
        assert!(first.created);
        assert_eq!(db.list_projects(Some(1)).unwrap().len(), 1);

        // 会话被回写：project_id 指向新项目 + state=finalized
        let s = db.ingest_session(sid).unwrap().unwrap();
        assert_eq!(s.project_id, Some(first.project_id));
        assert_eq!(s.state, "finalized");
        assert_eq!(s.kind, "bilibili", "来源判别列");
        assert_eq!(s.source_ref.as_deref(), Some("BV1xx"));

        // vtrim.toml 真的写了弹幕（否则精修侧会没有弹幕，且不报错）
        let toml_txt = std::fs::read_to_string(&first.config_path).unwrap();
        assert!(toml_txt.contains("danmaku_file = \"danmaku.xml\""), "{toml_txt}");

        // bvid 不能塞进 projects.room（前端把它当房间号展示）
        let p = db.project_by_id(first.project_id).unwrap().unwrap();
        assert_eq!(p.room, None, "B站来源的 room 必须留空");
        assert_eq!(p.source_kind, "bilibili");

        // 再落一次：复用同一个项目，不重复登记
        l.name = "改个名字".into();
        let again = land_project(&db, &l).unwrap();
        assert!(!again.created, "同一个 vtrim.toml 不该建第二个项目");
        assert_eq!(again.project_id, first.project_id);
        assert_eq!(db.list_projects(Some(1)).unwrap().len(), 1);

        drop(dir);
    }

    /// `source_kind` 是预载策略的键：落位写进去的值必须能被策略表认出来。
    #[test]
    fn source_kind_matches_preload_policy_keys() {
        let (dir, db, root) = fixture();
        db.create_user("u", "h", "user", false, 0).unwrap();
        let l = landing(&root, "bilibili", None);
        land_project(&db, &l).unwrap();
        let p = &db.list_projects(Some(1)).unwrap()[0];
        assert_eq!(
            crate::assets::preload_policy(&p.source_kind),
            crate::assets::Preload::Danmaku,
            "bilibili 必须映射到弹幕预载"
        );
        drop(dir);
    }
}
