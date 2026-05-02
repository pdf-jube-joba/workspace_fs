use std::{env, net::SocketAddr, sync::Arc};

use anyhow::Result;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{
    application::workspace_service::WorkspaceService,
    http::{
        cli::{CliOptions, parse_cli_options},
        identity::IdentityConfig,
        router::build_router,
    },
    infra::{fs_repository::FsRepository, repository_config::RepositoryConfig},
};

pub async fn run_from_env() -> Result<()> {
    init_tracing();
    let cli = parse_cli_options(env::args().skip(1))?;
    run(cli).await
}

pub(crate) async fn run(cli: CliOptions) -> Result<()> {
    let repository_root = cli.repository_path.canonicalize_utf8()?;
    let config = Arc::new(RepositoryConfig::load_with_serve_overrides(
        &repository_root,
        &cli.serve_overrides,
    )?);
    let repository = Arc::new(FsRepository::open(&repository_root, &config)?);
    let workspace = Arc::new(WorkspaceService::new(repository, config));
    let identity = IdentityConfig::load();

    tracing::info!(
        repository = %workspace.repository_root(),
        port = workspace.serve_port(),
        plugin_url_prefix = %workspace.plugin_url_prefix(),
        policy_url_prefix = %workspace.policy_url_prefix(),
        info_url_prefix = %workspace.info_url_prefix(),
        "serve configuration loaded"
    );

    let app = build_router(workspace.clone(), identity);
    let addr = SocketAddr::from(([127, 0, 0, 1], workspace.serve_port()));
    tracing::info!("listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn init_tracing() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            env::var("RUST_LOG").unwrap_or_else(|_| "workspace_fs=info,tower_http=info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();
}
