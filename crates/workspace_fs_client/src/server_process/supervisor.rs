use std::{env, process::Stdio};

use anyhow::{Context, Result, anyhow};
use camino::Utf8PathBuf;
use tokio::{
    net::TcpStream,
    process::{Child, Command},
    time::{Duration, Instant, sleep},
};

use crate::{
    config::user_config::{UserRepositoryConfig, UserServerConfig},
    runtime::app::{ServerSupervisor, SpawnedServer, normalize_upstream_base},
    server_process::{SERVER_BINARY_BASENAME, SERVER_PACKAGE_NAME},
};

impl ServerSupervisor {
    pub(crate) fn new(workspace_root: Utf8PathBuf, repository_root: Utf8PathBuf) -> Self {
        Self {
            workspace_root,
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
        let child = self.spawn_server_process(server).await?;
        self.spawned.insert(
            repository.name.clone(),
            SpawnedServer {
                child,
                upstream_base: upstream_base.clone(),
            },
        );
        Ok(upstream_base)
    }

    async fn spawn_server_process(&self, server: &UserServerConfig) -> Result<Child> {
        self.build_server_binary().await?;
        let binary = self.server_binary_path();
        let mut command = Command::new(binary.as_std_path());
        command
            .arg(self.repository_root.as_str())
            .args(server.cli_args()?)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        let mut child = command.spawn().context("failed to launch server binary")?;
        self.wait_for_server_ready(&mut child, server.port().unwrap_or_default())
            .await?;
        Ok(child)
    }

    async fn build_server_binary(&self) -> Result<()> {
        let status = Command::new("cargo")
            .arg("build")
            .arg("-p")
            .arg(SERVER_PACKAGE_NAME)
            .current_dir(self.workspace_root.as_std_path())
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .await
            .with_context(|| format!("failed to build {SERVER_PACKAGE_NAME}"))?;

        if !status.success() {
            return Err(anyhow!(
                "cargo build for {SERVER_PACKAGE_NAME} failed: {status}"
            ));
        }

        Ok(())
    }

    fn server_binary_path(&self) -> Utf8PathBuf {
        let name = if env::consts::EXE_EXTENSION.is_empty() {
            SERVER_BINARY_BASENAME.to_owned()
        } else {
            format!("{SERVER_BINARY_BASENAME}.{}", env::consts::EXE_EXTENSION)
        };
        self.workspace_root.join("target").join("debug").join(name)
    }

    async fn wait_for_server_ready(&self, child: &mut Child, port: u16) -> Result<()> {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        let deadline = Instant::now() + Duration::from_secs(15);

        loop {
            if let Some(status) = child.try_wait().context("failed to check server status")? {
                return Err(anyhow!("server process exited with {status}"));
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
            terminate_child(&mut spawned.child).await;
        }
        self.spawned.clear();
    }
}

async fn terminate_child(child: &mut Child) {
    if let Ok(Some(_)) = child.try_wait() {
        return;
    }

    if let Err(error) = child.kill().await {
        tracing::warn!(error = %error, pid = child.id().unwrap_or_default(), "failed to kill child process");
        return;
    }

    if let Err(error) = child.wait().await {
        tracing::warn!(error = %error, pid = child.id().unwrap_or_default(), "failed to wait for child process");
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
