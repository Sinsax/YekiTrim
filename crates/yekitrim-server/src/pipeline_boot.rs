//! 起自动管线的**唯一入口**（从 `ingest_worker` 上提到 crate 根）。
//!
//! ## 为什么上提
//!
//! `spawn_pipeline` 与 [`Pipeline`] 只做两件事：把"档位"翻译成阶段名、往任务队列里塞一个 job。
//! 它**与录播姬无关**，而 B站下载完成后要起的是同一条管线。
//! 如果留在 `ingest_worker` 里，`download_worker` 就得 `use crate::ingest_worker::…` ——
//! 那正是计划 §2 禁止的"两个 worker 互相调用"（录播姬那套待分配池 / 空闲窗口语义会顺着
//! 调用链渗进 B站通道，改一边还容易碰坏另一边）。
//!
//! 所以：**共享的东西要么下沉到 `yekitrim-service`，要么上提到 crate 根**。这个文件是后者。

use std::path::PathBuf;
use std::sync::Arc;

use super::model::AppState;

/// 自动策略档位（配置里 `auto_pipeline = "none|transcribe|detect|cut"`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pipeline {
    None,
    Transcribe,
    Detect,
    Cut,
}

impl Pipeline {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "none" | "" => Pipeline::None,
            "transcribe" => Pipeline::Transcribe,
            "cut" => Pipeline::Cut,
            // 未知值按 detect：宁可多做一步，也别让"配置写错"表现成"什么都没跑"
            _ => Pipeline::Detect,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Pipeline::None => "none",
            Pipeline::Transcribe => "transcribe",
            Pipeline::Detect => "detect",
            Pipeline::Cut => "cut",
        }
    }

    /// 管线阶段（`resume` 会跳过已有输出的阶段）。
    pub fn stages(self) -> Vec<String> {
        let names: &[&str] = match self {
            Pipeline::None => &[],
            Pipeline::Transcribe => &["extract", "transcribe"],
            Pipeline::Detect => &["extract", "transcribe", "detect"],
            Pipeline::Cut => &["extract", "transcribe", "detect", "cut"],
        };
        names.iter().map(|s| s.to_string()).collect()
    }
}

/// 启动自动管线任务（新项目 + 自己的 job）。
///
/// 与手动任务同规则：资源域冲突（同项目在跑别的任务 / 转写撞模型下载）就**排队**，
/// 不再因为"别的项目有任务在跑"而整条跳过。
///
/// `origin` 只影响 job 的显示名（`入库后自动处理` / `下载后自动处理`），不参与调度。
pub async fn spawn_pipeline(
    state: &Arc<AppState>,
    owner: i64,
    project_id: i64,
    config_file: PathBuf,
    pipeline: Pipeline,
    origin: &str,
) {
    let stages = pipeline.stages();
    if stages.is_empty() {
        return;
    }
    let kind = format!("ingest_{}", pipeline.as_str());
    let keys = super::scope_key::pipeline_keys(project_id, &stages);
    // 后台优先级：入库/下载自动管线排在用户点按的任务后面（同档内），不抢占已经在跑的。
    // 档位按阶段判定，与手动触发同一条规则 —— 自动触发的转写同样是重活。
    let class = super::jobs::JobClass::for_pipeline(&stages);
    let handle = state
        .jobs
        .create(
            owner,
            super::jobs::JobSpec::new(kind.clone(), origin, keys, class).background(),
        )
        .await;
    let job_id = handle.id.clone();
    state.jobs.attach_project(&job_id, project_id).await;
    let jobs = state.jobs.clone();
    let cancel = state.jobs.cancel_token(&job_id).await;
    let job_for_task = job_id.clone();
    // 自动管线是整段同步代码（转写分钟级）→ 专用任务线程，别占 HTTP worker
    crate::api::spawn_job(jobs.clone(), job_for_task.clone(), move || async move {
        if handle.wait_start().await.is_err() {
            jobs.complete(&job_for_task).await;
            return;
        }
        let result = crate::api::core::run_stages(
            config_file,
            stages,
            false,
            true,
            job_for_task.clone(),
            jobs.clone(),
            cancel,
        )
        .await;
        match result {
            Ok(()) => jobs.complete(&job_for_task).await,
            Err(e) => jobs.fail(&job_for_task, format!("{e:#}")).await,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_levels_map_to_stages() {
        assert!(Pipeline::parse("none").stages().is_empty());
        assert_eq!(Pipeline::parse("transcribe").stages(), vec!["extract", "transcribe"]);
        assert_eq!(
            Pipeline::parse("detect").stages(),
            vec!["extract", "transcribe", "detect"]
        );
        assert_eq!(Pipeline::parse("cut").stages().len(), 4);
        // 未知值按默认 detect
        assert_eq!(Pipeline::parse("whatever"), Pipeline::Detect);
    }
}
