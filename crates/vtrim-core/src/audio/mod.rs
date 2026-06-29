use async_trait::async_trait;
use anyhow::Result;

use crate::config::Config;
use crate::pipeline::{Stage, STAGE_AUDIO};
use crate::types::PipelineContext;

mod extract;

pub struct AudioExtractor;

#[async_trait]
impl Stage for AudioExtractor {
    fn name(&self) -> &'static str {
        "AudioExtractor"
    }

    fn stage_number(&self) -> u32 {
        STAGE_AUDIO
    }

    fn is_enabled(&self, _config: &Config) -> bool {
        true
    }

    async fn run(&self, ctx: &mut PipelineContext) -> Result<()> {
        let audio_path = ctx.output_dir.join("video.wav");

        std::fs::create_dir_all(&ctx.output_dir)?;

        tracing::info!("获取视频信息...");
        let info = extract::get_video_info(&ctx.input_path)?;
        tracing::info!(
            "视频: {}x{}, {}s, codec={}/{}",
            info.width,
            info.height,
            info.duration_secs as u64,
            info.video_codec,
            info.audio_codec,
        );
        ctx.video_info = Some(info);

        tracing::info!("提取音频 → {}", audio_path.display());
        extract::extract_audio(&ctx.input_path, &audio_path)?;
        ctx.audio_path = Some(audio_path);

        tracing::info!("音频提取完成");
        Ok(())
    }
}
