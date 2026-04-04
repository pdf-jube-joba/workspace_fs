use std::sync::Arc;

use anyhow::Result;
use axum::Json;
use axum::body::Body;
use axum::{
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use mime_guess::MimeGuess;

use crate::{
    config::RepositoryConfig,
    identity::UserIdentity,
    info::PathInfo,
    path::WorkspacePath,
    plugin::{PluginRunner, PluginTrigger},
    policy::{MethodKind, PolicyInspection, inspect_policy_rules, resolve_policy},
    repository::{FsRepository, Repository},
};

#[derive(Debug)]
pub struct WorkspaceError {
    pub status: StatusCode,
    pub message: String,
}

pub struct WorkspaceService {
    repository: Arc<FsRepository>,
    config: Arc<RepositoryConfig>,
}

impl WorkspaceError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            message: message.into(),
        }
    }

    pub fn internal(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl IntoResponse for WorkspaceError {
    fn into_response(self) -> Response {
        (
            self.status,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            )],
            self.message,
        )
            .into_response()
    }
}

impl WorkspaceService {
    pub fn new(repository: Arc<FsRepository>, config: Arc<RepositoryConfig>) -> Self {
        Self { repository, config }
    }

    pub fn serve_port(&self) -> u16 {
        self.config.serve.port
    }

    pub fn repository_root(&self) -> &camino::Utf8Path {
        self.repository.repository_root()
    }

    pub async fn run_task(&self, task_name: &str, skip_deps: bool) -> Result<()> {
        self.plugin_runner().run_task(task_name, skip_deps).await
    }

    pub async fn run_manual_plugin(
        &self,
        plugin_name: &str,
        user_identity: &UserIdentity,
    ) -> Result<()> {
        self.plugin_runner()
            .run_manual_plugin(plugin_name, user_identity)
            .await
    }

    pub fn plugin_url_prefix(&self) -> &str {
        &self.config.serve.plugin_url_prefix
    }

    pub fn policy_url_prefix(&self) -> &str {
        &self.config.serve.policy_url_prefix
    }

    pub fn info_url_prefix(&self) -> &str {
        &self.config.serve.info_url_prefix
    }

    pub async fn get_root(&self, user_identity: &UserIdentity) -> Result<Response, WorkspaceError> {
        self.get_path("/", user_identity).await
    }

    pub async fn get_path(
        &self,
        url_path: &str,
        user_identity: &UserIdentity,
    ) -> Result<Response, WorkspaceError> {
        let path = self.normalize_request_path(url_path)?;
        self.enforce_policy(MethodKind::Get, &path)?;

        if path.is_directory() {
            return self.directory_response(&path, user_identity).await;
        }

        if self.repository.list_directory(&path).await.is_ok() {
            return Err(WorkspaceError::bad_request(
                "directory path must end with /",
            ));
        }

        let content = match self.repository.read_file(&path).await {
            Ok(content) => content,
            Err(error) => {
                let mapped = self.map_read_error(error);
                tracing::warn!(user = %user_identity, path = %path.as_str(), status = %mapped.status, error = %mapped.message, "read failed");
                return Err(mapped);
            }
        };

        self.run_trigger(PluginTrigger::Get, &path, user_identity)
            .await?;
        Ok(file_response(
            StatusCode::OK,
            &content_type_for_path(&path),
            content,
        ))
    }

    pub async fn create_path(
        &self,
        url_path: &str,
        body: &str,
        user_identity: &UserIdentity,
    ) -> Result<Response, WorkspaceError> {
        let path = self.normalize_request_path(url_path)?;
        self.enforce_policy(MethodKind::Post, &path)?;

        if path.is_directory() {
            match self.repository.create_directory(&path).await {
                Ok(()) => {
                    tracing::info!(user = %user_identity, path = %path.as_str(), "directory created")
                }
                Err(error) => {
                    let mapped = self.map_create_directory_error(error);
                    tracing::warn!(user = %user_identity, path = %path.as_str(), status = %mapped.status, error = %mapped.message, "directory create failed");
                    return Err(mapped);
                }
            }
        } else {
            match self.repository.create_text_file(&path, body).await {
                Ok(()) => {
                    tracing::info!(user = %user_identity, path = %path.as_str(), "file created")
                }
                Err(error) => {
                    let mapped = self.map_create_error(error);
                    tracing::warn!(user = %user_identity, path = %path.as_str(), status = %mapped.status, error = %mapped.message, "file create failed");
                    return Err(mapped);
                }
            }
        }

        self.run_trigger(PluginTrigger::Post, &path, user_identity)
            .await?;
        Ok(StatusCode::CREATED.into_response())
    }

    pub async fn update_file(
        &self,
        url_path: &str,
        body: &str,
        user_identity: &UserIdentity,
    ) -> Result<Response, WorkspaceError> {
        let path = self.normalize_request_path(url_path)?;
        self.enforce_policy(MethodKind::Put, &path)?;
        reject_directory_path(&path, "cannot update a directory path with PUT")?;

        match self.repository.write_text_file(&path, body).await {
            Ok(()) => tracing::info!(user = %user_identity, path = %path.as_str(), "file updated"),
            Err(error) => {
                let mapped = self.map_write_error(error);
                tracing::warn!(user = %user_identity, path = %path.as_str(), status = %mapped.status, error = %mapped.message, "file update failed");
                return Err(mapped);
            }
        }

        self.run_trigger(PluginTrigger::Put, &path, user_identity)
            .await?;
        Ok(StatusCode::NO_CONTENT.into_response())
    }

    pub async fn delete_path(
        &self,
        url_path: &str,
        user_identity: &UserIdentity,
    ) -> Result<Response, WorkspaceError> {
        let path = self.normalize_request_path(url_path)?;
        self.enforce_policy(MethodKind::Delete, &path)?;

        if path.is_directory() {
            match self.repository.delete_directory(&path).await {
                Ok(()) => {
                    tracing::info!(user = %user_identity, path = %path.as_str(), "directory deleted")
                }
                Err(error) => {
                    let mapped = self.map_delete_directory_error(error);
                    tracing::warn!(user = %user_identity, path = %path.as_str(), status = %mapped.status, error = %mapped.message, "directory delete failed");
                    return Err(mapped);
                }
            }
        } else {
            match self.repository.delete_file(&path).await {
                Ok(()) => {
                    tracing::info!(user = %user_identity, path = %path.as_str(), "file deleted")
                }
                Err(error) => {
                    let mapped = self.map_delete_error(error);
                    tracing::warn!(user = %user_identity, path = %path.as_str(), status = %mapped.status, error = %mapped.message, "file delete failed");
                    return Err(mapped);
                }
            }
        }

        self.run_trigger(PluginTrigger::Delete, &path, user_identity)
            .await?;
        Ok(StatusCode::NO_CONTENT.into_response())
    }

    pub async fn inspect_policy(
        &self,
        url_path: &str,
    ) -> Result<Json<PolicyInspection>, WorkspaceError> {
        let path = self.normalize_request_path(url_path)?;
        self.inspect_policy_rules(&path)
            .map(Json)
            .map_err(WorkspaceError::internal)
    }

    pub async fn get_path_info(&self, url_path: &str) -> Result<Json<PathInfo>, WorkspaceError> {
        let path = self.normalize_request_path(url_path)?;
        self.enforce_policy(MethodKind::Get, &path)?;

        let info = self.repository.path_info(&path).await.map_err(|error| {
            let mapped = self.map_metadata_error(error);
            tracing::warn!(path = %path.as_str(), status = %mapped.status, error = %mapped.message, "metadata read failed");
            mapped
        })?;

        if path.is_directory() && info.kind != crate::info::PathInfoKind::Directory {
            return Err(WorkspaceError::bad_request("file path must not end with /"));
        }
        if !path.is_directory() && info.kind == crate::info::PathInfoKind::Directory {
            return Err(WorkspaceError::bad_request(
                "directory path must end with /",
            ));
        }

        Ok(Json(info))
    }

    async fn directory_response(
        &self,
        path: &WorkspacePath,
        user_identity: &UserIdentity,
    ) -> Result<Response, WorkspaceError> {
        let entries = match self.repository.list_directory(path).await {
            Ok(entries) => entries,
            Err(error) => {
                let message = error.to_string();
                let mapped =
                    if message.contains("not a directory") || message.contains("No such file") {
                        WorkspaceError::not_found("directory not found")
                    } else {
                        WorkspaceError::internal(error)
                    };
                tracing::warn!(user = %user_identity, path = %path, status = %mapped.status, error = %mapped.message, "directory listing failed");
                return Err(mapped);
            }
        };
        self.run_trigger(PluginTrigger::Get, path, user_identity)
            .await?;
        Ok(text_response(StatusCode::OK, entries.join("\n")))
    }

    async fn run_trigger(
        &self,
        trigger: PluginTrigger,
        path: &WorkspacePath,
        user_identity: &UserIdentity,
    ) -> Result<(), WorkspaceError> {
        match self
            .plugin_runner()
            .run_hook_if_matched(trigger, path, user_identity)
            .await
        {
            Ok(()) => Ok(()),
            Err(error) => {
                tracing::warn!(user = %user_identity, path = %path, trigger = %trigger.as_str(), error = %error, "plugin hook failed");
                Err(WorkspaceError::internal(error))
            }
        }
    }

    fn plugin_runner(&self) -> PluginRunner<'_> {
        PluginRunner::new(
            self.repository.repository_root(),
            &self.config.name,
            &self.config,
        )
    }

    fn normalize_request_path(&self, path: &str) -> Result<WorkspacePath, WorkspaceError> {
        WorkspacePath::from_url(path)
            .map_err(|error| WorkspaceError::bad_request(error.to_string()))
    }

    fn enforce_policy(
        &self,
        method: MethodKind,
        path: &WorkspacePath,
    ) -> Result<(), WorkspaceError> {
        let allowed = self
            .resolve_policy(method, path)
            .map_err(WorkspaceError::internal)?
            .unwrap_or(false);

        if allowed {
            Ok(())
        } else {
            Err(WorkspaceError::forbidden("operation denied by policy"))
        }
    }

    fn resolve_policy(&self, method: MethodKind, path: &WorkspacePath) -> Result<Option<bool>> {
        resolve_policy(method, &self.config.policy, path)
    }

    fn inspect_policy_rules(&self, path: &WorkspacePath) -> Result<PolicyInspection> {
        inspect_policy_rules(&self.config.policy, path)
    }

    fn map_create_error(&self, error: anyhow::Error) -> WorkspaceError {
        let message = error.to_string();
        if message.contains("file already exists") {
            return WorkspaceError::conflict("file already exists");
        }
        if message.contains("parent directory not found") {
            return WorkspaceError::not_found("parent directory not found");
        }
        if message.contains("parent path is not a directory") {
            return WorkspaceError::bad_request("parent path is not a directory");
        }
        map_path_error(error)
    }

    fn map_create_directory_error(&self, error: anyhow::Error) -> WorkspaceError {
        let message = error.to_string();
        if message.contains("directory already exists") {
            return WorkspaceError::conflict("directory already exists");
        }
        if message.contains("parent directory not found") {
            return WorkspaceError::not_found("parent directory not found");
        }
        if message.contains("parent path is not a directory") {
            return WorkspaceError::bad_request("parent path is not a directory");
        }
        map_path_error(error)
    }

    fn map_write_error(&self, error: anyhow::Error) -> WorkspaceError {
        let message = error.to_string();
        if message.contains("file not found") {
            return WorkspaceError::not_found("file not found");
        }
        if message.contains("path is a directory") {
            return WorkspaceError::bad_request("path is a directory");
        }
        map_path_error(error)
    }

    fn map_read_error(&self, error: anyhow::Error) -> WorkspaceError {
        if error_chain_contains(&error, "No such file")
            || error_chain_contains(&error, "os error 2")
        {
            return WorkspaceError::not_found("path not found");
        }
        if error_chain_contains(&error, "Is a directory")
            || error_chain_contains(&error, "os error 21")
        {
            return WorkspaceError::bad_request("path is a directory");
        }
        map_path_error(error)
    }

    fn map_delete_error(&self, error: anyhow::Error) -> WorkspaceError {
        let message = error.to_string();
        if message.contains("file not found") {
            return WorkspaceError::not_found("file not found");
        }
        if message.contains("path is a directory") {
            return WorkspaceError::bad_request("path is a directory");
        }
        map_path_error(error)
    }

    fn map_delete_directory_error(&self, error: anyhow::Error) -> WorkspaceError {
        let message = error.to_string();
        if message.contains("directory not found") {
            return WorkspaceError::not_found("directory not found");
        }
        if message.contains("path is not a directory") {
            return WorkspaceError::bad_request("path is not a directory");
        }
        if message.contains("directory is not empty") {
            return WorkspaceError::conflict("directory is not empty");
        }
        map_path_error(error)
    }

    fn map_metadata_error(&self, error: anyhow::Error) -> WorkspaceError {
        let message = error.to_string();
        if message.contains("No such file") || message.contains("os error 2") {
            return WorkspaceError::not_found("path not found");
        }
        map_path_error(error)
    }
}

fn reject_directory_path(
    path: &WorkspacePath,
    message: &'static str,
) -> Result<(), WorkspaceError> {
    if path.is_directory() {
        return Err(WorkspaceError::bad_request(message));
    }
    Ok(())
}

fn map_path_error(error: anyhow::Error) -> WorkspaceError {
    let message = error.to_string();
    if message.contains("path escapes repository root")
        || message.contains("absolute paths are not allowed")
        || message.contains("reserved path")
    {
        return WorkspaceError::bad_request(message);
    }
    WorkspaceError::internal(error)
}

fn text_response(status: StatusCode, body: String) -> Response {
    file_response(status, "text/plain; charset=utf-8", body.into_bytes())
}

fn file_response(status: StatusCode, content_type: &str, body: Vec<u8>) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(body))
        .expect("response builder should accept binary body")
}

fn content_type_for_path(path: &WorkspacePath) -> String {
    let mime = MimeGuess::from_path(path.as_str())
        .first_raw()
        .unwrap_or("application/octet-stream");

    if mime.starts_with("text/") {
        return format!("{mime}; charset=utf-8");
    }

    mime.to_string()
}

fn error_chain_contains(error: &anyhow::Error, needle: &str) -> bool {
    error
        .chain()
        .any(|cause| cause.to_string().contains(needle))
}

#[cfg(test)]
mod tests {
    use axum::{body::to_bytes, http::header::CONTENT_TYPE};

    use super::*;
    use crate::info::{PathInfo, PathInfoKind};

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

    #[test]
    fn path_info_requires_directory_suffix_for_directories() {
        let info = PathInfo::new("docs", PathInfoKind::Directory, None, None, false);

        assert_eq!(info.kind, PathInfoKind::Directory);
    }
}
