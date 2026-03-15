use std::path::PathBuf;

use anyhow::Result;

pub fn mcphub_state_path() -> PathBuf {
    mcphub::api::state_path()
}

pub async fn sync_endpoint_with_mcphub(
    endpoint_id: &str,
    endpoint_url: &str,
    endpoint_name: &str,
) -> Result<String> {
    let sync =
        mcphub::api::sync_http_endpoint(endpoint_id, endpoint_url, Vec::new(), endpoint_name)
            .await?;

    Ok(format!(
        "registered {} -> {}\ndiscovered {} tool(s) for {}",
        sync.endpoint.id,
        sync.endpoint.target,
        sync.tools.len(),
        sync.endpoint.id
    ))
}
