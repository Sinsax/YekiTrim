use async_trait::async_trait;

use crate::config::Config;
use crate::types::PipelineContext;

pub const STAGE_AUDIO: u32 = 1;
pub const STAGE_TRANSCRIBE: u32 = 2;
pub const STAGE_DETECT: u32 = 3;
pub const STAGE_CUT: u32 = 4;

pub fn stage_enabled(_num: u32, _config: &Config) -> bool {
    true
}

#[async_trait]
pub trait Stage: Send + Sync {
    fn name(&self) -> &'static str;
    fn stage_number(&self) -> u32;
    fn is_enabled(&self, config: &Config) -> bool;
    async fn run(&self, ctx: &mut PipelineContext) -> anyhow::Result<()>;
}
