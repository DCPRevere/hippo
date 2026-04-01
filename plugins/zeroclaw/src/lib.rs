pub mod client;
pub mod config;
pub mod memory;
pub mod models;
pub mod tools;

pub use client::HippoClient;
pub use config::HippoConfig;
pub use memory::{HippoMemory, Memory, MemoryCategory, MemoryEntry};
pub use tools::{HippoAskTool, HippoRecallTool, HippoRememberTool, Tool, ToolResult};

/// Create a `HippoMemory` instance from the given config.
pub fn create_memory(config: &HippoConfig) -> anyhow::Result<HippoMemory> {
    let client = HippoClient::new(config)?;
    Ok(HippoMemory::new(client))
}

/// Create the three Hippo tools (remember, recall, ask) from the given config.
pub fn create_tools(config: &HippoConfig) -> anyhow::Result<Vec<Box<dyn Tool>>> {
    let client = HippoClient::new(config)?;
    Ok(vec![
        Box::new(HippoRememberTool::new(client.clone())),
        Box::new(HippoRecallTool::new(client.clone())),
        Box::new(HippoAskTool::new(client)),
    ])
}
