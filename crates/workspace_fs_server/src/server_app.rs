use std::{env, net::SocketAddr, sync::Arc};

use anyhow::{Context, Result, anyhow, bail};
use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use camino::Utf8PathBuf;
use tower_http::trace::{DefaultOnRequest, DefaultOnResponse, TraceLayer};
use tracing::Level;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{
    config::{RepositoryConfig, ServeSettingsOverride},
    identity::{IdentityConfig, RequestIdentity, UserIdentity, capture_identity},
    info::PathInfo,
    policy::PolicyInspection,
    repository::FsRepository,
    workspace::{self, WorkspaceService},
};

#[derive(Clone)]
struct AppState {
    workspace: Arc<WorkspaceService>,
}

#[derive(Debug, Clone)]
pub struct CliOptions {
    pub repository_path: Utf8PathBuf,
    pub serve_overrides: ServeSettingsOverride,
}

pub async fn run_from_env() -> Result<()> {
    init_tracing();
    let cli = parse_cli_options(env::args().skip(1))?;
    run(cli).await
}

pub async fn run(cli: CliOptions) -> Result<()> {
    let repository_root = cli.repository_path.canonicalize_utf8()?;
    let config = Arc::new(RepositoryConfig::load_with_serve_overrides(
        &repository_root,
        &cli.serve_overrides,
    )?);
    let repository = Arc::new(FsRepository::open(&repository_root, &config)?);
    let workspace = Arc::new(WorkspaceService::new(repository, config));

    let identity = IdentityConfig::load();
    let state = Arc::new(AppState {
        workspace: workspace.clone(),
    });
    let plugin_run_route = format!("{}/{{name}}/run", workspace.plugin_url_prefix());
    let policy_route = format!("{}/{{*path}}", workspace.policy_url_prefix());
    let info_route = format!("{}/{{*path}}", workspace.info_url_prefix());

    tracing::info!(
        repository = %workspace.repository_root(),
        port = workspace.serve_port(),
        plugin_url_prefix = %workspace.plugin_url_prefix(),
        policy_url_prefix = %workspace.policy_url_prefix(),
        info_url_prefix = %workspace.info_url_prefix(),
        "serve configuration loaded"
    );

    let app = Router::new()
        .route("/", get(root_handler))
        .route(&plugin_run_route, post(run_plugin_handler))
        .route(&policy_route, get(get_policy_path_handler))
        .route(workspace.info_url_prefix(), get(get_info_root_handler))
        .route(&info_route, get(get_info_path_handler))
        .route(
            "/{*path}",
            get(get_path_handler)
                .post(post_path_handler)
                .put(put_path_handler)
                .delete(delete_path_handler),
        )
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &axum::http::Request<_>| {
                    let user_identity = UserIdentity::from_headers(request.headers())
                        .map(|value| value.as_str().to_owned())
                        .unwrap_or_default();
                    tracing::info_span!(
                        "http_request",
                        method = %request.method(),
                        path = %request.uri().path(),
                        user_identity = %user_identity,
                    )
                })
                .on_request(DefaultOnRequest::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(middleware::from_fn_with_state(identity, capture_identity))
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], workspace.serve_port()));
    tracing::info!("listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

pub fn parse_cli_options<I>(args: I) -> Result<CliOptions>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let repository_path = args
        .next()
        .ok_or_else(|| anyhow!("usage: workspace_fs <repository-path>"))?;
    let mut serve_overrides = ServeSettingsOverride::default();

    for arg in args {
        let Some((name, value)) = arg.split_once('=') else {
            bail!("unknown argument: {arg}");
        };
        match name {
            "--port" => {
                serve_overrides.port = Some(
                    value
                        .parse()
                        .with_context(|| format!("invalid value for --port: {value}"))?,
                );
            }
            "--plugin-url-prefix" => {
                serve_overrides.plugin_url_prefix = Some(value.to_owned());
            }
            "--policy-url-prefix" => {
                serve_overrides.policy_url_prefix = Some(value.to_owned());
            }
            "--info-url-prefix" => {
                serve_overrides.info_url_prefix = Some(value.to_owned());
            }
            _ => bail!("unknown argument: {arg}"),
        }
    }

    Ok(CliOptions {
        repository_path: Utf8PathBuf::from(repository_path),
        serve_overrides,
    })
}

pub fn cli_args_for_server(cli: &CliOptions) -> Vec<String> {
    let mut args = vec![cli.repository_path.as_str().to_owned()];
    if let Some(port) = cli.serve_overrides.port {
        args.push(format!("--port={port}"));
    }
    if let Some(plugin_url_prefix) = &cli.serve_overrides.plugin_url_prefix {
        args.push(format!("--plugin-url-prefix={plugin_url_prefix}"));
    }
    if let Some(policy_url_prefix) = &cli.serve_overrides.policy_url_prefix {
        args.push(format!("--policy-url-prefix={policy_url_prefix}"));
    }
    if let Some(info_url_prefix) = &cli.serve_overrides.info_url_prefix {
        args.push(format!("--info-url-prefix={info_url_prefix}"));
    }
    args
}

fn init_tracing() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            env::var("RUST_LOG").unwrap_or_else(|_| "workspace_fs=info,tower_http=info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();
}

async fn root_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
) -> Result<Response, workspace::WorkspaceError> {
    state.workspace.get_root(&identity.user).await
}

async fn run_plugin_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
    Path(name): Path<String>,
) -> Result<Response, workspace::WorkspaceError> {
    state
        .workspace
        .run_plugin(&name, &identity.user)
        .await
        .map(|_| axum::http::StatusCode::NO_CONTENT.into_response())
}

async fn get_policy_path_handler(
    State(state): State<Arc<AppState>>,
    Extension(_identity): Extension<RequestIdentity>,
    Path(path): Path<String>,
) -> Result<Json<PolicyInspection>, workspace::WorkspaceError> {
    state.workspace.inspect_policy(&request_path(&path)).await
}

async fn get_info_root_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
) -> Result<Json<PathInfo>, workspace::WorkspaceError> {
    state.workspace.get_path_info("/", &identity.user).await
}

async fn get_info_path_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
    Path(path): Path<String>,
) -> Result<Json<PathInfo>, workspace::WorkspaceError> {
    state
        .workspace
        .get_path_info(&request_path(&path), &identity.user)
        .await
}

async fn get_path_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
    Path(path): Path<String>,
) -> Result<Response, workspace::WorkspaceError> {
    state
        .workspace
        .get_path(&request_path(&path), &identity.user)
        .await
}

async fn post_path_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
    Path(path): Path<String>,
    body: String,
) -> Result<Response, workspace::WorkspaceError> {
    state
        .workspace
        .create_path(&request_path(&path), &body, &identity.user)
        .await
}

async fn put_path_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
    Path(path): Path<String>,
    body: String,
) -> Result<Response, workspace::WorkspaceError> {
    state
        .workspace
        .update_file(&request_path(&path), &body, &identity.user)
        .await
}

async fn delete_path_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
    Path(path): Path<String>,
) -> Result<Response, workspace::WorkspaceError> {
    state
        .workspace
        .delete_path(&request_path(&path), &identity.user)
        .await
}

fn request_path(path: &str) -> String {
    if path.is_empty() {
        "/".to_string()
    } else {
        format!("/{path}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::UserIdentity;

    #[test]
    fn request_path_root() {
        assert_eq!(request_path(""), "/");
    }

    #[test]
    fn request_path_file() {
        assert_eq!(request_path("docs/a.md"), "/docs/a.md");
    }

    #[test]
    fn user_identity_from_headers_reads_header() {
        let request = axum::http::Request::builder()
            .header("user-identity", "from_browser")
            .body(())
            .unwrap();

        assert_eq!(
            UserIdentity::from_headers(request.headers())
                .unwrap()
                .as_str(),
            "from_browser"
        );
    }

    #[test]
    fn parse_cli_args_accepts_repository_only() {
        let cli = parse_cli_options(["./repo".to_string()]).unwrap();

        assert_eq!(cli.repository_path, Utf8PathBuf::from("./repo"));
        assert!(cli.serve_overrides.port.is_none());
    }

    #[test]
    fn parse_cli_args_accepts_serve_overrides() {
        let cli = parse_cli_options([
            "./repo".to_string(),
            "--port=4010".to_string(),
            "--plugin-url-prefix=/.plugin2".to_string(),
            "--policy-url-prefix=/.policy2".to_string(),
            "--info-url-prefix=/.info2".to_string(),
        ])
        .unwrap();

        assert_eq!(cli.repository_path, Utf8PathBuf::from("./repo"));
        assert_eq!(cli.serve_overrides.port, Some(4010));
        assert_eq!(
            cli.serve_overrides.plugin_url_prefix.as_deref(),
            Some("/.plugin2")
        );
        assert_eq!(
            cli.serve_overrides.policy_url_prefix.as_deref(),
            Some("/.policy2")
        );
        assert_eq!(
            cli.serve_overrides.info_url_prefix.as_deref(),
            Some("/.info2")
        );
    }
}
