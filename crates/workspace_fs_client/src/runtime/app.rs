use std::{collections::HashMap, env, sync::Arc};

use anyhow::{Context, Result, anyhow, bail};
use camino::Utf8PathBuf;
use reqwest::Client;
use tokio::{process::Child, signal, sync::watch, task::JoinSet};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{
    config::{cli::CliOptions, user_config::UserConfig},
    proxy::http_proxy,
    repl::runner,
    task_runner::task_runner,
};

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) client: Client,
    pub(crate) repository_name: String,
    pub(crate) upstream_base: String,
    pub(crate) user_identity: String,
}

pub(crate) struct SpawnedServer {
    pub(crate) child: Child,
    pub(crate) upstream_base: String,
}

pub(crate) struct ServerSupervisor {
    pub(crate) workspace_root: Utf8PathBuf,
    pub(crate) repository_root: Utf8PathBuf,
    pub(crate) spawned: HashMap<String, SpawnedServer>,
}

pub async fn run_from_env() -> Result<()> {
    init_tracing();
    let cli = parse_cli_options(env::args().skip(1))?;
    run(cli).await
}

pub async fn run(cli: CliOptions) -> Result<()> {
    let repository_root = resolve_repository_root(cli.repository_path.as_ref())?;
    let config = UserConfig::load(&repository_root)?;
    let workspace_root = workspace_fs_root();
    let mut supervisor = ServerSupervisor::new(workspace_root, repository_root.clone());

    let task_name = cli.task.clone().or(cli.task_only.clone());
    if let Some(task_name) = task_name.as_deref() {
        task_runner::run_task(&config, task_name, &mut supervisor).await?;
        if cli.task_only.is_some() {
            supervisor.shutdown_all().await;
            return Ok(());
        }
    }

    if cli.repl {
        runner::run_repl(&config, &mut supervisor).await?;
        supervisor.shutdown_all().await;
        return Ok(());
    }

    let repositories = config.repositories_to_start(cli.repository_name.as_deref())?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let shutdown_for_signal = shutdown_tx.clone();
    tokio::spawn(async move {
        let _ = signal::ctrl_c().await;
        let _ = shutdown_for_signal.send(true);
    });

    let mut join_set = JoinSet::new();
    for repository in repositories {
        let upstream_base = supervisor.upstream_base_for_repository(repository).await?;
        let state = Arc::new(AppState {
            client: build_http_client("proxy")?,
            repository_name: repository.name.clone(),
            upstream_base,
            user_identity: repository.as_user.clone(),
        });

        tracing::info!(
            repository = %repository_root,
            target_repository = %state.repository_name,
            listen_port = repository.port,
            upstream = %state.upstream_base,
            user_identity = %state.user_identity,
            "client proxy configuration loaded"
        );

        join_set.spawn(http_proxy::run_proxy_server(
            repository.port,
            state,
            shutdown_rx.clone(),
        ));
    }

    let mut first_error: Option<anyhow::Error> = None;
    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                if first_error.is_none() {
                    first_error = Some(error);
                }
                let _ = shutdown_tx.send(true);
            }
            Err(error) => {
                if first_error.is_none() {
                    first_error = Some(anyhow!("proxy server task failed: {error}"));
                }
                let _ = shutdown_tx.send(true);
            }
        }
    }

    supervisor.shutdown_all().await;
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(())
}

pub fn parse_cli_options<I>(args: I) -> Result<CliOptions>
where
    I: IntoIterator<Item = String>,
{
    crate::config::cli::parse_cli_options(args)
}

fn init_tracing() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            env::var("RUST_LOG").unwrap_or_else(|_| "workspace_fs=info,tower_http=info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();
}

fn resolve_repository_root(path: Option<&Utf8PathBuf>) -> Result<Utf8PathBuf> {
    let base = match path {
        Some(path) => path.clone(),
        None => Utf8PathBuf::from_path_buf(
            std::env::current_dir().context("failed to get current directory")?,
        )
        .map_err(|_| anyhow!("current directory must be UTF-8"))?,
    };
    Ok(base.canonicalize_utf8()?)
}

pub(crate) fn normalize_upstream_base(input: &str) -> Result<String> {
    let trimmed = input.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        bail!("repository.where must not be empty");
    }
    if trimmed.starts_with("https://") {
        bail!("https is not supported for repository.where");
    }
    if let Some((scheme, _)) = trimmed.split_once("://") {
        if scheme != "http" {
            bail!("unsupported scheme for repository.where: {scheme}");
        }
        return Ok(trimmed.to_owned());
    }
    Ok(format!("http://{trimmed}"))
}

pub(crate) fn build_http_client(context: &'static str) -> Result<Client> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .with_context(|| format!("failed to build {context} client"))
}

fn workspace_fs_root() -> Utf8PathBuf {
    let manifest_dir = Utf8PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if let Some(root) = manifest_dir.parent().and_then(|path| path.parent()) {
        return root.to_owned();
    }
    manifest_dir
}
