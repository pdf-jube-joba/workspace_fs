use anyhow::{Context, Result, anyhow, bail};
use reqwest::Client;

use crate::{
    config::user_config::{UserConfig, UserRepositoryConfig},
    runtime::app::{ServerSupervisor, build_http_client},
};

pub(crate) async fn run_task(
    config: &UserConfig,
    task_name: &str,
    supervisor: &mut ServerSupervisor,
) -> Result<()> {
    let task = config
        .find_task(task_name)
        .ok_or_else(|| anyhow!("task not found in .repo/user.toml: {task_name}"))?;
    let client = build_http_client("task")?;

    for step in &task.step {
        let repository = config.find_repository(&step.repository).ok_or_else(|| {
            anyhow!(
                "repository not found in .repo/user.toml: {}",
                step.repository
            )
        })?;
        let upstream_base = supervisor.upstream_base_for_repository(repository).await?;
        run_task_step(&client, repository, &upstream_base, &step.plugin).await?;
    }

    Ok(())
}

pub(crate) async fn run_repository_plugin(
    config: &UserConfig,
    supervisor: &mut ServerSupervisor,
    repository_name: &str,
    plugin_name: &str,
) -> Result<()> {
    let repository = config
        .find_repository(repository_name)
        .ok_or_else(|| anyhow!("repository not found in .repo/user.toml: {repository_name}"))?;
    let upstream_base = supervisor.upstream_base_for_repository(repository).await?;
    let client = build_http_client("plugin")?;
    run_task_step(&client, repository, &upstream_base, plugin_name).await
}

async fn run_task_step(
    client: &Client,
    repository: &UserRepositoryConfig,
    upstream_base: &str,
    plugin_name: &str,
) -> Result<()> {
    let plugin_url_prefix = repository.upstream_plugin_url_prefix();
    let plugin_url = format!("{}{}/{}/run", upstream_base, plugin_url_prefix, plugin_name);
    let response = client
        .post(plugin_url)
        .header("user-identity", &repository.as_user)
        .send()
        .await
        .with_context(|| format!("failed to invoke plugin over HTTP: {}", plugin_name))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!(
            "plugin invocation failed: repository={} plugin={} status={} body={}",
            repository.name,
            plugin_name,
            status,
            body
        );
    }

    Ok(())
}
