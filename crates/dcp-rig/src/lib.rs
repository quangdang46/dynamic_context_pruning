use dcp_config::Config;
use dcp_core::ContextPruner;

pub use dcp_types::{Message as DcpMessage, Part, Role};

#[allow(dead_code)]
pub struct DcpCompletionModel {
    pruner: ContextPruner,
}

impl DcpCompletionModel {
    pub fn new(config: Config) -> Result<Self, anyhow::Error> {
        let pruner = ContextPruner::new(config)?;
        Ok(Self { pruner })
    }

    pub fn with_default() -> Result<Self, anyhow::Error> {
        let config = Config::load_default().unwrap_or_else(|_| Config::default());
        Self::new(config)
    }
}
