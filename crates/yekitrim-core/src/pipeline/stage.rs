use async_trait::async_trait;

use crate::types::PipelineContext;

pub const STAGE_AUDIO: u32 = 1;
pub const STAGE_TRANSCRIBE: u32 = 2;
pub const STAGE_DETECT: u32 = 3;
pub const STAGE_CUT: u32 = 4;

#[async_trait]
pub trait Stage: Send + Sync {
    fn name(&self) -> &'static str;
    fn stage_number(&self) -> u32;
    async fn run(&self, ctx: &mut PipelineContext) -> anyhow::Result<()>;
}
