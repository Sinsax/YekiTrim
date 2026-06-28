use std::path::Path;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use crate::types::TranscriptSegment;

pub async fn transcribe(audio: &Path, backend: &str) -> Result<Vec<TranscriptSegment>> {
    match backend {
        "bcut" => bcut_asr(audio).await,
        "kuaishou" => kuai_shou_asr(audio).await,
        _ => bail!("不支持的云端 ASR 后端: {backend}，可选: bcut, kuaishou"),
    }
}

// ── Bcut (必剪 / 哔哩哔哩) ──────────────────────────────────────────

#[derive(Deserialize)]
struct BcutEnvelope<T> {
    code: i64,
    data: Option<T>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct UploadData {
    #[serde(rename = "in_boss_key")]
    in_boss_key: String,
    #[serde(rename = "resource_id")]
    resource_id: String,
    #[serde(rename = "upload_id")]
    upload_id: String,
    #[serde(rename = "upload_urls")]
    upload_urls: Vec<String>,
    #[serde(rename = "per_size")]
    per_size: usize,
    size: usize,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct CommitData {
    #[serde(rename = "download_url")]
    download_url: String,
}

#[derive(Deserialize)]
struct TaskData {
    task_id: String,
}

#[derive(Deserialize)]
struct ResultWrapper {
    code: i64,
    data: Option<ResultData>,
}

#[derive(Deserialize)]
struct ResultData {
    state: u32,
    result: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct Utterance {
    transcript: String,
    start_time: u64,
    end_time: u64,
}

#[derive(Deserialize)]
struct TaskResult {
    utterances: Vec<Utterance>,
}

async fn bcut_asr(audio: &Path) -> Result<Vec<TranscriptSegment>> {
    let mp3 = convert_to_mp3(audio)?;
    let mp3_bytes = std::fs::read(&mp3).context("读取临时 MP3 文件失败")?;
    let file_size = mp3_bytes.len();

    let client = reqwest::Client::new();

    // 1) 申请上传
    tracing::info!("Bcut ASR: 申请上传...");
    let upload_req = serde_json::json!({
        "type": 2, "name": "audio.mp3", "size": file_size,
        "ResourceFileType": "mp3", "model_id": "8",
    });
    let up: UploadData = bcut_post(
        &client,
        "https://member.bilibili.com/x/bcut/rubick-interface/resource/create",
        &upload_req,
    )
    .await?;

    // 2) 分片上传
    tracing::info!("Bcut ASR: 上传 {} 分片...", up.upload_urls.len());
    let mut etags: Vec<String> = Vec::new();
    for (i, url) in up.upload_urls.iter().enumerate() {
        let start = i * up.per_size;
        let end = (start + up.per_size).min(file_size);
        if start >= file_size {
            break;
        }
        let chunk = &mp3_bytes[start..end];

        let resp = client
            .put(url)
            .body(chunk.to_vec())
            .send()
            .await
            .with_context(|| format!("Bcut 分片 {} 上传失败", i))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Bcut 分片 {} 上传失败 ({}): {}", i, status, body);
        }

        if let Some(etag) = resp.headers().get("etag") {
            etags.push(etag.to_str().unwrap_or("").to_string());
        }
    }
    tracing::info!("Bcut ASR: 上传完成 ({} 分片)", etags.len());

    // 3) 提交上传
    tracing::info!("Bcut ASR: 提交上传...");
    let commit_body = serde_json::json!({
        "InBossKey": up.in_boss_key,
        "ResourceId": up.resource_id,
        "Etags": etags.join(","),
        "UploadId": up.upload_id,
        "model_id": "8",
    });
    let _commit: CommitData = bcut_post(
        &client,
        "https://member.bilibili.com/x/bcut/rubick-interface/resource/create/complete",
        &commit_body,
    )
    .await?;

    // 4) 创建转写任务
    tracing::info!("Bcut ASR: 创建转写任务...");
    let task: TaskData = bcut_post(
        &client,
        "https://member.bilibili.com/x/bcut/rubick-interface/task",
        &serde_json::json!({"resource": up.in_boss_key, "model_id": "8"}),
    )
    .await?;

    // 5) 轮询结果
    tracing::info!("Bcut ASR: 等待转写结果...");
    for _ in 0..300 {
        let resp = client
            .get("https://member.bilibili.com/x/bcut/rubick-interface/task/result")
            .query(&[("model_id", "7"), ("task_id", &task.task_id)])
            .header("User-Agent", "Bilibili/1.0.0")
            .send()
            .await
            .context("Bcut 查询结果失败")?;

        if !resp.status().is_success() {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            continue;
        }

        let wr: ResultWrapper = resp.json().await.context("解析 Bcut 结果响应失败")?;

        if wr.code != 0 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            continue;
        }

        let data = match &wr.data {
            Some(d) => d,
            None => {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
        };

        if data.state == 4 {
            std::fs::remove_file(&mp3).ok();

            if let Some(json_str) = &data.result {
                let task_result: TaskResult = serde_json::from_str(json_str)
                    .context("解析 Bcut 转写结果失败")?;

                if task_result.utterances.is_empty() {
                    bail!("Bcut ASR 转写未返回有效语音段");
                }

                let segments: Vec<TranscriptSegment> = task_result
                    .utterances
                    .into_iter()
                    .map(|u| TranscriptSegment {
                        start: u.start_time as f64 / 1000.0,
                        end: u.end_time as f64 / 1000.0,
                        text: u.transcript,
                        confidence: 1.0,
                    })
                    .collect();

                tracing::info!("Bcut ASR: {} 段", segments.len());
                return Ok(segments);
            }
            bail!("Bcut ASR 转写结果为空");
        }

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    std::fs::remove_file(&mp3).ok();
    bail!("Bcut ASR 轮询超时 (300s)");
}

// ── KuaiShou (快手) ──────────────────────────────────────────────────

async fn kuai_shou_asr(audio: &Path) -> Result<Vec<TranscriptSegment>> {
    let mp3 = convert_to_mp3(audio)?;
    let mp3_bytes = std::fs::read(&mp3).context("读取临时 MP3 文件失败")?;

    let size_mb = mp3_bytes.len() as f64 / 1_048_576.0;
    tracing::info!("KuaiShou ASR: 上传音频 ({:.1} MB)...", size_mb);

    let form = reqwest::multipart::Form::new()
        .text("typeId", "1")
        .part(
            "file",
            reqwest::multipart::Part::bytes(mp3_bytes)
                .file_name("test.mp3")
                .mime_str("audio/mpeg")?,
        );

    let resp = reqwest::Client::new()
        .post("https://ai.kuaishou.com/api/effects/subtitle_generate")
        .multipart(form)
        .send()
        .await
        .context("KuaiShou ASR 请求失败")?;

    std::fs::remove_file(&mp3).ok();

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("KuaiShou ASR 返回 {}: {}", status, body);
    }

    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text)?;

    let code = v["code"].as_i64().unwrap_or(-1);
    if code == 501 {
        bail!("KuaiShou ASR 服务暂时不可用 (code=501)，请稍后重试或使用 bcut 后端");
    }
    if code != 0 {
        bail!("KuaiShou ASR 返回错误 code={}: {}", code, v["msg"]);
    }

    let segments_raw = &v["data"]["text"];
    let segments: Vec<TranscriptSegment> = segments_raw
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|item| TranscriptSegment {
                    start: item["start_time"].as_u64().unwrap_or(0) as f64 / 1000.0,
                    end: item["end_time"].as_u64().unwrap_or(0) as f64 / 1000.0,
                    text: item["text"].as_str().unwrap_or("").to_string(),
                    confidence: 1.0,
                })
                .collect()
        })
        .unwrap_or_default();

    if segments.is_empty() {
        bail!("KuaiShou ASR 返回空结果");
    }

    tracing::info!("KuaiShou ASR: {} 段", segments.len());
    Ok(segments)
}

async fn bcut_post<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    body: &serde_json::Value,
) -> Result<T> {
    let resp = client
        .post(url)
        .header("User-Agent", "Bilibili/1.0.0 (https://www.bilibili.com)")
        .header("Content-Type", "application/json")
        .json(body)
        .send()
        .await
        .with_context(|| format!("Bcut POST {} 失败", url))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        bail!("Bcut POST {} 返回 {}: {}", url, status, text);
    }

    let envelope: BcutEnvelope<T> =
        resp.json().await.with_context(|| format!("解析 Bcut {} 响应失败", url))?;

    if envelope.code != 0 {
        bail!("Bcut API 返回错误 code={}: {}", envelope.code, url);
    }

    Ok(envelope.data.unwrap())
}

// ── 音频转换 ─────────────────────────────────────────────────────────

fn convert_to_mp3(wav_path: &Path) -> Result<std::path::PathBuf> {
    let mp3_path = std::env::temp_dir().join(format!(
        "vtrim_{}.mp3",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));

    let output = std::process::Command::new("ffmpeg")
        .arg("-y")
        .arg("-loglevel")
        .arg("error")
        .arg("-i")
        .arg(wav_path)
        .arg("-codec:a")
        .arg("libmp3lame")
        .arg("-b:a")
        .arg("128k")
        .arg("-ar")
        .arg("44100")
        .arg("-ac")
        .arg("1")
        .arg(&mp3_path)
        .output()
        .context("ffmpeg 转码到 MP3 失败")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("ffmpeg 转码到 MP3 失败: {}", stderr.trim());
    }

    Ok(mp3_path)
}
