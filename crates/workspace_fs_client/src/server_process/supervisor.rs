use anyhow::{Context, Result, anyhow};
use camino::Utf8PathBuf;
use tokio::{
    net::TcpStream,
    task::JoinHandle,
    time::{Duration, Instant, sleep},
};
use workspace_fs_server as server;

use crate::{
    config::user_config::{UserRepositoryConfig, UserServerConfig},
    runtime::app::{ServerSupervisor, SpawnedServer, normalize_upstream_base},
};

impl ServerSupervisor {
    pub(crate) fn new(repository_root: Utf8PathBuf) -> Self {
        Self {
            repository_root,
            spawned: std::collections::HashMap::new(),
        }
    }

    pub(crate) async fn upstream_base_for_repository(
        &mut self,
        repository: &UserRepositoryConfig,
    ) -> Result<String> {
        if let Some(where_) = &repository.where_ {
            return normalize_upstream_base(where_);
        }

        self.ensure_spawned(repository).await
    }

    async fn ensure_spawned(&mut self, repository: &UserRepositoryConfig) -> Result<String> {
        if let Some(existing) = self.spawned.get(&repository.name) {
            return Ok(existing.upstream_base.clone());
        }

        let server = repository.server_config()?;
        let port = server.port().ok_or_else(|| {
            anyhow!(
                "repository.server.port is required when mode = \"spawn\": {}",
                repository.name
            )
        })?;
        let upstream_base = format!("http://127.0.0.1:{port}");
        if is_server_reachable(port).await {
            tracing::info!(
                repository = %repository.name,
                upstream = %upstream_base,
                "reusing existing spawned server"
            );
            return Ok(upstream_base);
        }
        let handle = self.spawn_server_task(server).await?;
        self.spawned.insert(
            repository.name.clone(),
            SpawnedServer {
                handle,
                upstream_base: upstream_base.clone(),
            },
        );
        Ok(upstream_base)
    }

    async fn spawn_server_task(
        &self,
        server_config: &UserServerConfig,
    ) -> Result<JoinHandle<Result<()>>> {
        let cli = self.server_cli_options(server_config)?;
        let port = server_config.port().unwrap_or_default();
        let mut handle = tokio::spawn(async move { server::run(cli).await });
        self.wait_for_server_ready(&mut handle, port).await?;
        Ok(handle)
    }

    fn server_cli_options(&self, server_config: &UserServerConfig) -> Result<server::CliOptions> {
        let mut args = vec![self.repository_root.as_str().to_owned()];
        args.extend(server_config.cli_args()?);
        server::parse_cli_options(args)
            .context("failed to build server CLI options from repository.server settings")
    }

    async fn wait_for_server_ready(
        &self,
        handle: &mut JoinHandle<Result<()>>,
        port: u16,
    ) -> Result<()> {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        let deadline = Instant::now() + Duration::from_secs(15);

        loop {
            if handle.is_finished() {
                return Err(match handle.await {
                    Ok(Ok(())) => anyhow!("server task exited before becoming ready"),
                    Ok(Err(error)) => error.context("server task exited before becoming ready"),
                    Err(error) => anyhow!("server task failed before becoming ready: {error}"),
                });
            }

            match TcpStream::connect(addr).await {
                Ok(stream) => {
                    drop(stream);
                    return Ok(());
                }
                Err(error) if is_connection_pending(&error) => {
                    if Instant::now() >= deadline {
                        return Err(anyhow!("timed out waiting for server to start"));
                    }
                    sleep(Duration::from_millis(100)).await;
                }
                Err(error) => return Err(error).context("failed to connect to spawned server"),
            }
        }
    }

    pub(crate) async fn shutdown_all(&mut self) {
        for spawned in self.spawned.values_mut() {
            terminate_task(&mut spawned.handle).await;
        }
        self.spawned.clear();
    }
}

async fn terminate_task(handle: &mut JoinHandle<Result<()>>) {
    if handle.is_finished() {
        match handle.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(error = %error, "server task exited during shutdown");
            }
            Err(error) => {
                tracing::warn!(error = %error, "server task join failed during shutdown");
            }
        }
        return;
    }

    handle.abort();
    if let Err(error) = handle.await
        && !error.is_cancelled()
    {
        tracing::warn!(error = %error, "server task join failed after abort");
    }
}

fn is_connection_pending(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::NotConnected
    )
}

async fn is_server_reachable(port: u16) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect(addr).await.is_ok()
}
