use clap::Parser;
use std::env;

#[derive(Debug, Clone, Parser)]
#[command(name = "stiglab-agent", about = "Stiglab node agent")]
pub struct AgentConfig {
    /// WebSocket URL of the control plane server
    #[arg(
        long,
        short,
        env = "STIGLAB_SERVER_URL",
        default_value = "ws://localhost:3000/agent/ws"
    )]
    pub server: String,

    /// Name of this node
    #[arg(long, short, env = "STIGLAB_NODE_NAME")]
    pub name: Option<String>,

    /// Maximum concurrent sessions
    #[arg(long, short, env = "STIGLAB_MAX_SESSIONS", default_value = "4")]
    pub max_sessions: u32,

    /// Command to execute for agent sessions
    #[arg(long, env = "STIGLAB_AGENT_COMMAND", default_value = "claude")]
    pub agent_command: String,
}

impl AgentConfig {
    pub fn node_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| {
            env::var("HOSTNAME")
                .or_else(|_| hostname::get().map(|h| h.to_string_lossy().to_string()))
                .unwrap_or_else(|_| "unknown".to_string())
        })
    }
}
