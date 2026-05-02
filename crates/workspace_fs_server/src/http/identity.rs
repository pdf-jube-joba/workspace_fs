use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

#[derive(Debug, Clone)]
pub struct IdentityConfig;

#[derive(Debug, Clone)]
pub struct UserIdentity(String);

#[derive(Debug, Clone)]
pub struct RequestIdentity {
    pub user: UserIdentity,
}

impl UserIdentity {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn from_headers(headers: &HeaderMap) -> Option<Self> {
        headers
            .get("user-identity")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(Self::new)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Display for UserIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl IdentityConfig {
    pub fn load() -> Self {
        Self
    }
}

pub async fn capture_identity(
    State(_identity): State<IdentityConfig>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let Some(user) =
        UserIdentity::from_headers(request.headers()).filter(|value| !value.is_empty())
    else {
        return (StatusCode::BAD_REQUEST, "missing user-identity header").into_response();
    };

    request.extensions_mut().insert(RequestIdentity { user });
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::Body,
        extract::Extension,
        http::{Request, StatusCode},
        middleware,
        response::IntoResponse,
        routing::get,
    };
    use tower::ServiceExt;

    async fn identity_handler(
        Extension(identity): Extension<RequestIdentity>,
    ) -> impl IntoResponse {
        identity.user.as_str().to_owned()
    }

    fn app() -> Router {
        Router::new()
            .route("/", get(identity_handler).post(identity_handler))
            .layer(middleware::from_fn_with_state(
                IdentityConfig::load(),
                capture_identity,
            ))
    }

    #[tokio::test]
    async fn get_requires_user_identity() {
        let response = app()
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn post_requires_user_identity() {
        let response = app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn get_accepts_user_identity() {
        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header("user-identity", "alice_browser")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
