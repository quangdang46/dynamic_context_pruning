use napi_derive::napi;
use napi::bindgen_prelude::*;

use crate::message;

fn parse_compress_args(json_str: &str) -> Result<dcp_core::CompressArgs> {
    let val: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| Error::from_reason(format!("Args parse: {}", e)))?;

    let mode = val.get("mode").and_then(|v| v.as_str()).unwrap_or("range");
    let topic = val
        .get("topic")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    match mode {
        "message" => {
            let content: Vec<dcp_core::MessageEntry> = serde_json::from_value(
                val.get("content").cloned().unwrap_or(serde_json::Value::Array(vec![])),
            )
            .map_err(|e| Error::from_reason(format!("Content parse: {}", e)))?;
            Ok(dcp_core::CompressArgs::Message { topic, content })
        }
        _ => {
            let content: Vec<dcp_core::RangeEntry> = serde_json::from_value(
                val.get("content").cloned().unwrap_or(serde_json::Value::Array(vec![])),
            )
            .map_err(|e| Error::from_reason(format!("Content parse: {}", e)))?;
            Ok(dcp_core::CompressArgs::Range { topic, content })
        }
    }
}

#[napi]
pub struct DcpPruner {
    inner: std::sync::Mutex<dcp_core::ContextPruner>,
}

#[napi]
impl DcpPruner {
    #[napi(constructor)]
    pub fn new(config_json: String) -> Result<Self> {
        let config: dcp_config::Config = serde_json::from_str(&config_json)
            .map_err(|e| Error::from_reason(format!("Config parse: {}", e)))?;
        let pruner = dcp_core::ContextPruner::new(config)
            .map_err(|e| Error::from_reason(format!("Pruner init: {}", e)))?;
        let _ = pruner.save();
        Ok(Self { inner: std::sync::Mutex::new(pruner) })
    }

    /// Transform messages before sending to the LLM.
    /// Input: JSON array of OpenCode format messages.
    /// Output: JSON array of transformed OpenCode format messages.
    #[napi]
    pub fn transform_messages(&self, messages_json: String) -> Result<String> {
        let messages = message::opencode_to_dcp(&messages_json)
            .map_err(|e| Error::from_reason(e))?;
        let mut pruner = self.inner.lock()
            .map_err(|_| Error::from_reason("mutex poisoned".to_string()))?;
        let result = pruner.transform_messages(messages)
            .map_err(|e| Error::from_reason(format!("Transform: {}", e)))?;
        message::dcp_to_opencode(&result)
            .map_err(|e| Error::from_reason(e))
    }

    /// Append DCP system prompt addendum.
    #[napi]
    pub fn transform_system(&self, system: String) -> String {
        if let Ok(pruner) = self.inner.lock() {
            let mut s = system;
            pruner.transform_system(&mut s);
            return s;
        }
        system
    }

    /// Handle compress tool call from the LLM.
    #[napi]
    pub fn handle_compress(&self, args_json: String, messages_json: String) -> Result<String> {
        let args = parse_compress_args(&args_json)?;
        let messages = message::opencode_to_dcp(&messages_json)
            .map_err(|e| Error::from_reason(e))?;
        let mut pruner = self.inner.lock()
            .map_err(|_| Error::from_reason("mutex poisoned".to_string()))?;
        let result = pruner.handle_compress(args, &messages)
            .map_err(|e| Error::from_reason(format!("Compress: {}", e)))?;
        serde_json::to_string(&result)
            .map_err(|e| Error::from_reason(format!("Serialize: {}", e)))
    }

    /// Restore a compressed block to its original messages.
    #[napi]
    pub fn decompress(&self, block_id: u32) -> Result<String> {
        let mut pruner = self.inner.lock()
            .map_err(|_| Error::from_reason("mutex poisoned".to_string()))?;
        let result = pruner.decompress(dcp_types::BlockId(block_id))
            .map_err(|e| Error::from_reason(format!("Decompress: {}", e)))?;
        serde_json::to_string(&result)
            .map_err(|e| Error::from_reason(format!("Serialize: {}", e)))
    }

    /// Re-activate a user-decompressed block for future compression.
    #[napi]
    pub fn recompress(&self, block_id: u32) -> Result<String> {
        let mut pruner = self.inner.lock()
            .map_err(|_| Error::from_reason("mutex poisoned".to_string()))?;
        let result = pruner.recompress(dcp_types::BlockId(block_id))
            .map_err(|e| Error::from_reason(format!("Recompress: {}", e)))?;
        serde_json::to_string(&result)
            .map_err(|e| Error::from_reason(format!("Serialize: {}", e)))
    }

    #[napi]
    pub fn has_pending_work(&self) -> bool {
        self.inner.lock().map(|p| p.has_pending_work()).unwrap_or(false)
    }

    #[napi]
    pub fn stats(&self) -> String {
        if let Ok(pruner) = self.inner.lock() {
            serde_json::to_string(&pruner.stats()).unwrap_or_default()
        } else {
            String::new()
        }
    }

    #[napi]
    pub fn set_session_id(&self, session_id: String) {
        if let Ok(mut pruner) = self.inner.lock() {
            pruner.set_session_id(&session_id);
        }
    }
}

impl Drop for DcpPruner {
    fn drop(&mut self) {
        if let Ok(pruner) = self.inner.lock() {
            let _ = pruner.save();
        }
    }
}
