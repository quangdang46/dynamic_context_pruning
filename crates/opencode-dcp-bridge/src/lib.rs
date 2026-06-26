mod message;
mod pruner;

pub use pruner::DcpPruner;

use napi_derive::napi;

/// Load DCP config from the standard cascade paths.
/// Returns JSON string of the resolved Config.
#[napi]
pub fn load_dcp_config() -> napi::Result<String> {
    let config = dcp_config::Config::load_default()
        .map_err(|e| napi::Error::from_reason(format!("Config load: {}", e)))?;
    serde_json::to_string(&config)
        .map_err(|e| napi::Error::from_reason(format!("Serialize: {}", e)))
}
