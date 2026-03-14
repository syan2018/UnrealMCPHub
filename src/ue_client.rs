use std::collections::HashMap;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use http::{HeaderName, HeaderValue};
use rmcp::model::{CallToolRequestParams, JsonObject};
use rmcp::service::{RunningService, ServiceExt};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type ClientSession = RunningService<rmcp::RoleClient, ()>;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolCallOutput {
    pub success: bool,
    pub content: Vec<Value>,
    pub structured_content: Option<Value>,
    pub error: Option<String>,
    pub duration_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EndpointHealth {
    pub healthy: bool,
    pub tool_count: usize,
    pub latency_ms: u128,
}

pub struct UeClient;

impl UeClient {
    pub async fn list_tools(url: &str) -> Result<Vec<ToolDescriptor>> {
        let client = Self::connect(url).await?;
        let tools = client
            .peer()
            .list_all_tools()
            .await
            .with_context(|| format!("failed to list tools for {url}"))?;
        Ok(tools
            .into_iter()
            .map(|tool| ToolDescriptor {
                name: tool.name.to_string(),
                description: tool.description.as_deref().unwrap_or_default().to_string(),
                input_schema: Value::Object((*tool.input_schema).clone()),
            })
            .collect())
    }

    pub async fn call_tool(
        url: &str,
        tool_name: &str,
        arguments: JsonObject,
    ) -> Result<ToolCallOutput> {
        let client = Self::connect(url).await?;
        let started = Instant::now();
        let result = client
            .peer()
            .call_tool(CallToolRequestParams::new(tool_name.to_string()).with_arguments(arguments))
            .await
            .with_context(|| format!("failed to call {tool_name} on {url}"))?;
        Ok(ToolCallOutput {
            success: !result.is_error.unwrap_or(false),
            content: result
                .content
                .into_iter()
                .map(|item| {
                    serde_json::to_value(item)
                        .unwrap_or(Value::String("unserializable-content".into()))
                })
                .collect(),
            structured_content: result.structured_content,
            error: None,
            duration_ms: started.elapsed().as_millis(),
        })
    }

    pub async fn health_check(url: &str) -> Result<EndpointHealth> {
        let started = Instant::now();
        let client = Self::connect(url).await?;
        let tools = client
            .peer()
            .list_all_tools()
            .await
            .with_context(|| format!("failed to list tools for {url}"))?;
        Ok(EndpointHealth {
            healthy: true,
            tool_count: tools.len(),
            latency_ms: started.elapsed().as_millis(),
        })
    }

    async fn connect(url: &str) -> Result<ClientSession> {
        if url.trim().is_empty() {
            bail!("endpoint url is empty");
        }
        let headers = HashMap::<HeaderName, HeaderValue>::new();
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(url.to_string()).custom_headers(headers),
        );
        let client: ClientSession =
            ().serve(transport)
                .await
                .with_context(|| format!("failed to connect to {url}"))?;
        Ok(client)
    }
}
