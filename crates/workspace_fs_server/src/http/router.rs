use std::sync::Arc;

use anyhow::Result;
use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use tower_http::trace::{DefaultOnRequest, DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::{
    application::workspace_service::WorkspaceService,
    domain::{path_info::PathInfo, policy::PolicyInspection},
    http::{
        error::HttpError,
        identity::{IdentityConfig, RequestIdentity, UserIdentity, capture_identity},
    },
};

#[derive(Clone)]
struct AppState {
    workspace: Arc<WorkspaceService>,
}

pub(crate) fn build_router(workspace: Arc<WorkspaceService>, identity: IdentityConfig) -> Router {
    let state = Arc::new(AppState {
        workspace: workspace.clone(),
    });
    let plugin_run_route = format!("{}/{{name}}/run", workspace.plugin_url_prefix());
    let policy_route = format!("{}/{{*path}}", workspace.policy_url_prefix());
    let info_route = format!("{}/{{*path}}", workspace.info_url_prefix());

    Router::new()
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
        .with_state(state)
}

async fn root_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
) -> Result<Response, HttpError> {
    state.workspace.get_root(&identity.user).await
}

async fn run_plugin_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
    Path(name): Path<String>,
) -> Result<Response, HttpError> {
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
) -> Result<Json<PolicyInspection>, HttpError> {
    state.workspace.inspect_policy(&request_path(&path)).await
}

async fn get_info_root_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
) -> Result<Json<PathInfo>, HttpError> {
    state.workspace.get_path_info("/", &identity.user).await
}

async fn get_info_path_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
    Path(path): Path<String>,
) -> Result<Json<PathInfo>, HttpError> {
    state
        .workspace
        .get_path_info(&request_path(&path), &identity.user)
        .await
}

async fn get_path_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
    Path(path): Path<String>,
) -> Result<Response, HttpError> {
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
) -> Result<Response, HttpError> {
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
) -> Result<Response, HttpError> {
    state
        .workspace
        .update_file(&request_path(&path), &body, &identity.user)
        .await
}

async fn delete_path_handler(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<RequestIdentity>,
    Path(path): Path<String>,
) -> Result<Response, HttpError> {
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
}
