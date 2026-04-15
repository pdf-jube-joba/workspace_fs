mod config;
mod identity;
mod info;
mod path;
mod plugin;
mod policy;
mod repository;
mod workspace;

use std::{env, net::SocketAddr, sync::Arc};

use anyhow::{Result, anyhow, bail};
use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use camino::Utf8PathBuf;
use config::RepositoryConfig;
use identity::{IdentityConfig, RequestIdentity, UserIdentity, capture_identity};
use info::PathInfo;
use policy::PolicyInspection;
use repository::FsRepository;
use tower_http::trace::{DefaultOnRequest, DefaultOnResponse, TraceLayer};
use tracing::Level;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use workspace::WorkspaceService;

#[derive(Clone)]
struct AppState {
    workspace: Arc<WorkspaceService>,
}

#[derive(Debug)]
struct CliOptions {
    repository_path: Utf8PathBuf,
    task: Option<String>,
    task_only: Option<String>,
    skip_deps: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let cli = parse_cli_options()?;
    let repository_root = cli.repository_path.canonicalize_utf8()?;
    let config = Arc::new(RepositoryConfig::load(&repository_root)?);
    let repository = Arc::new(FsRepository::open(&repository_root, &config)?);
    let workspace = Arc::new(WorkspaceService::new(repository, config));

    if let Some(task_name) = &cli.task_only {
        tracing::info!(task = %task_name, skip_deps = cli.skip_deps, "running task without serve");
        workspace.run_task(task_name, cli.skip_deps).await?;
        return Ok(());
    }

    if let Some(task_name) = &cli.task {
        tracing::info!(task = %task_name, skip_deps = cli.skip_deps, "running task before serve");
        workspace.run_task(task_name, cli.skip_deps).await?;
    }

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

fn parse_cli_options() -> Result<CliOptions> {
    parse_cli_args(env::args().skip(1))
}

fn parse_cli_args<I>(args: I) -> Result<CliOptions>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let repository_path = args.next().ok_or_else(|| {
        anyhow!("usage: workspace_fs <repository-path> [--task <name>] [--task-only <name>] [--skip-deps]")
    })?;
    let mut task = None;
    let mut task_only = None;
    let mut skip_deps = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--task" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --task"))?;
                task = Some(value);
            }
            "--task-only" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow!("missing value for --task-only"))?;
                task_only = Some(value);
            }
            "--skip-deps" => {
                skip_deps = true;
            }
            _ => bail!("unknown argument: {arg}"),
        }
    }

    if task.is_some() && task_only.is_some() {
        bail!("--task and --task-only are mutually exclusive");
    }

    if skip_deps && task.is_none() && task_only.is_none() {
        bail!("--skip-deps requires --task or --task-only");
    }

    Ok(CliOptions {
        repository_path: Utf8PathBuf::from(repository_path),
        task,
        task_only,
        skip_deps,
    })
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
        .run_manual_plugin(&name, &identity.user)
        .await
        .map(|_| axum::http::StatusCode::NO_CONTENT.into_response())
        .map_err(workspace::WorkspaceError::internal)
}

async fn get_policy_path_handler(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> Result<Json<PolicyInspection>, workspace::WorkspaceError> {
    state.workspace.inspect_policy(&request_path(&path)).await
}

async fn get_info_root_handler(
    State(state): State<Arc<AppState>>,
) -> Result<Json<PathInfo>, workspace::WorkspaceError> {
    state.workspace.get_path_info("/").await
}

async fn get_info_path_handler(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> Result<Json<PathInfo>, workspace::WorkspaceError> {
    state.workspace.get_path_info(&request_path(&path)).await
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
    format!("/{path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_path_adds_leading_slash() {
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
    fn parse_cli_args_accepts_task_only_with_task() {
        let cli = parse_cli_args([
            "./repo".to_string(),
            "--task-only".to_string(),
            "build".to_string(),
        ])
        .unwrap();

        assert_eq!(cli.repository_path, Utf8PathBuf::from("./repo"));
        assert_eq!(cli.task, None);
        assert_eq!(cli.task_only.as_deref(), Some("build"));
        assert!(!cli.skip_deps);
    }

    #[test]
    fn parse_cli_args_rejects_task_only_without_value() {
        let error = parse_cli_args(["./repo".to_string(), "--task-only".to_string()]).unwrap_err();

        assert_eq!(error.to_string(), "missing value for --task-only");
    }

    #[test]
    fn parse_cli_args_rejects_task_and_task_only_together() {
        let error = parse_cli_args([
            "./repo".to_string(),
            "--task".to_string(),
            "serve-build".to_string(),
            "--task-only".to_string(),
            "one-shot-build".to_string(),
        ])
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "--task and --task-only are mutually exclusive"
        );
    }
}
