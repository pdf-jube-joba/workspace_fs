use axum::{
    body::Body,
    http::{StatusCode, header},
    response::Response,
};
use mime_guess::MimeGuess;

use crate::domain::workspace_path::WorkspacePath;

pub(crate) fn text_response(status: StatusCode, body: String) -> Response {
    file_response(status, "text/plain; charset=utf-8", body.into_bytes())
}

pub(crate) fn file_response(status: StatusCode, content_type: &str, body: Vec<u8>) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .expect("response builder should accept binary body")
}

pub(crate) fn content_type_for_path(path: &WorkspacePath) -> String {
    let mime = MimeGuess::from_path(path.as_str())
        .first_raw()
        .unwrap_or("application/octet-stream");

    if mime.starts_with("text/") {
        return format!("{mime}; charset=utf-8");
    }

    mime.to_string()
}

#[cfg(test)]
mod tests {
    use axum::{body::to_bytes, http::header::CONTENT_TYPE};

    use super::*;

    #[tokio::test]
    async fn file_response_uses_html_mime_and_binary_body() {
        let response = file_response(
            StatusCode::OK,
            &content_type_for_path(
                &WorkspacePath::from_path_str("assets/md_preview.html").unwrap(),
            ),
            b"<h1>x</h1>".to_vec(),
        );
        let headers = response.headers();

        assert_eq!(
            headers.get(CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&body[..], b"<h1>x</h1>");
    }

    #[test]
    fn unknown_extension_falls_back_to_octet_stream() {
        assert_eq!(
            content_type_for_path(&WorkspacePath::from_path_str("assets/blob.custombin").unwrap()),
            "application/octet-stream"
        );
    }
}
