use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{OriginalUri, State},
    http::{HeaderMap, HeaderName, Method, Response, StatusCode, Uri, header},
    routing::any,
};
use tokio::sync::watch;

use crate::runtime::app::AppState;

pub(crate) async fn run_proxy_server(
    port: u16,
    state: Arc<AppState>,
    shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let app = Router::new()
        .route("/", any(proxy_handler))
        .route("/{*path}", any(proxy_handler))
        .with_state(state);

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    tracing::info!("client proxy listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(wait_for_shutdown(shutdown))
        .await
        .context("client proxy server failed")
}

async fn proxy_handler(
    State(state): State<Arc<AppState>>,
    method: Method,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Response<Body> {
    match forward_request(state, method, uri, headers, body).await {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(error = %error, "proxy request failed");
            Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::from("proxy request failed"))
                .unwrap()
        }
    }
}

async fn forward_request(
    state: Arc<AppState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response<Body>> {
    let path_and_query = uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/");
    let target_url = format!("{}{}", state.upstream_base, path_and_query);
    let reqwest_method = reqwest::Method::from_bytes(method.as_str().as_bytes())
        .context("unsupported http method")?;

    let mut request = state.client.request(reqwest_method, target_url);
    for (name, value) in &headers {
        if is_skipped_request_header(name) {
            continue;
        }
        request = request.header(name, value);
    }
    request = request.header("user-identity", &state.user_identity);

    let upstream = request
        .body(body)
        .send()
        .await
        .context("upstream request failed")?;
    let status = upstream.status();
    let upstream_headers = upstream.headers().clone();
    let body = upstream
        .bytes()
        .await
        .context("failed to read upstream body")?;

    let mut response = Response::builder().status(status);
    for (name, value) in &upstream_headers {
        if is_skipped_response_header(name) {
            continue;
        }
        response = response.header(name, value);
    }

    response
        .body(Body::from(body))
        .map_err(|error| anyhow!("failed to build proxy response: {error}"))
}

async fn wait_for_shutdown(mut shutdown: watch::Receiver<bool>) {
    loop {
        if *shutdown.borrow() {
            break;
        }
        if shutdown.changed().await.is_err() {
            break;
        }
    }
}

fn is_skipped_request_header(name: &HeaderName) -> bool {
    name == header::HOST || is_hop_by_hop_header(name) || name == "user-identity"
}

fn is_skipped_response_header(name: &HeaderName) -> bool {
    is_hop_by_hop_header(name)
}

fn is_hop_by_hop_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}
