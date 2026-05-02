use std::sync::Arc;

use anyhow::Result;
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::{
    domain::{
        path_info::{PathInfo, PathInfoKind},
        policy::{MethodKind, PolicyInspection, inspect_policy_rules, resolve_policy},
        workspace_path::WorkspacePath,
    },
    http::{
        error::HttpError,
        identity::UserIdentity,
        response::{content_type_for_path, file_response, text_response},
    },
    infra::{
        fs_repository::{FsRepository, Repository},
        plugin_runner::PluginRunner,
        repository_config::RepositoryConfig,
    },
};

pub(crate) struct WorkspaceService {
    repository: Arc<FsRepository>,
    config: Arc<RepositoryConfig>,
}

impl WorkspaceService {
    pub(crate) fn new(repository: Arc<FsRepository>, config: Arc<RepositoryConfig>) -> Self {
        Self { repository, config }
    }

    pub(crate) fn serve_port(&self) -> u16 {
        self.config.serve.port
    }

    pub(crate) fn repository_root(&self) -> &camino::Utf8Path {
        self.repository.repository_root()
    }

    pub(crate) fn plugin_url_prefix(&self) -> &str {
        &self.config.serve.plugin_url_prefix
    }

    pub(crate) fn policy_url_prefix(&self) -> &str {
        &self.config.serve.policy_url_prefix
    }

    pub(crate) fn info_url_prefix(&self) -> &str {
        &self.config.serve.info_url_prefix
    }

    pub(crate) async fn run_plugin(
        &self,
        plugin_name: &str,
        user_identity: &UserIdentity,
    ) -> Result<(), HttpError> {
        let plugin = self
            .config
            .find_plugin(plugin_name)
            .ok_or_else(|| HttpError::not_found("plugin not found"))?;
        if !plugin
            .allow
            .iter()
            .any(|candidate| candidate == user_identity.as_str())
        {
            return Err(HttpError::forbidden("plugin execution denied"));
        }
        self.plugin_runner()
            .run_plugin(plugin_name, user_identity)
            .await
            .map_err(HttpError::internal)
    }

    pub(crate) async fn get_root(
        &self,
        user_identity: &UserIdentity,
    ) -> Result<Response, HttpError> {
        self.get_path("/", user_identity).await
    }

    pub(crate) async fn get_path(
        &self,
        url_path: &str,
        user_identity: &UserIdentity,
    ) -> Result<Response, HttpError> {
        let path = self.authorized_path(url_path, MethodKind::Get, user_identity)?;
        let info = self.path_info_for_request(&path).await?;

        match info.kind {
            PathInfoKind::Directory => self.directory_response(&path, user_identity).await,
            PathInfoKind::File => {
                let content = self.repository.read_file(&path).await.map_err(|error| {
                    let mapped = HttpError::from_read_file(error);
                    tracing::warn!(user = %user_identity, path = %path.as_str(), status = %mapped.status, error = %mapped.message, "read failed");
                    mapped
                })?;
                Ok(file_response(
                    StatusCode::OK,
                    &content_type_for_path(&path),
                    content,
                ))
            }
        }
    }

    pub(crate) async fn create_path(
        &self,
        url_path: &str,
        body: &str,
        user_identity: &UserIdentity,
    ) -> Result<Response, HttpError> {
        let path = self.authorized_path(url_path, MethodKind::Post, user_identity)?;

        if path.is_directory() {
            self.repository.create_directory(&path).await.map_err(|error| {
                let mapped = HttpError::from_create_directory(error);
                tracing::warn!(user = %user_identity, path = %path.as_str(), status = %mapped.status, error = %mapped.message, "directory create failed");
                mapped
            })?;
            tracing::info!(user = %user_identity, path = %path.as_str(), "directory created");
        } else {
            self.repository.create_text_file(&path, body).await.map_err(|error| {
                let mapped = HttpError::from_create_file(error);
                tracing::warn!(user = %user_identity, path = %path.as_str(), status = %mapped.status, error = %mapped.message, "file create failed");
                mapped
            })?;
            tracing::info!(user = %user_identity, path = %path.as_str(), "file created");
        }

        Ok(StatusCode::CREATED.into_response())
    }

    pub(crate) async fn update_file(
        &self,
        url_path: &str,
        body: &str,
        user_identity: &UserIdentity,
    ) -> Result<Response, HttpError> {
        let path = self.authorized_path(url_path, MethodKind::Put, user_identity)?;
        reject_directory_path(&path, "cannot update a directory path with PUT")?;

        self.repository.write_text_file(&path, body).await.map_err(|error| {
            let mapped = HttpError::from_write_file(error);
            tracing::warn!(user = %user_identity, path = %path.as_str(), status = %mapped.status, error = %mapped.message, "file update failed");
            mapped
        })?;
        tracing::info!(user = %user_identity, path = %path.as_str(), "file updated");

        Ok(StatusCode::NO_CONTENT.into_response())
    }

    pub(crate) async fn delete_path(
        &self,
        url_path: &str,
        user_identity: &UserIdentity,
    ) -> Result<Response, HttpError> {
        let path = self.authorized_path(url_path, MethodKind::Delete, user_identity)?;

        if path.is_directory() {
            self.repository.delete_directory(&path).await.map_err(|error| {
                let mapped = HttpError::from_delete_directory(error);
                tracing::warn!(user = %user_identity, path = %path.as_str(), status = %mapped.status, error = %mapped.message, "directory delete failed");
                mapped
            })?;
            tracing::info!(user = %user_identity, path = %path.as_str(), "directory deleted");
        } else {
            self.repository.delete_file(&path).await.map_err(|error| {
                let mapped = HttpError::from_delete_file(error);
                tracing::warn!(user = %user_identity, path = %path.as_str(), status = %mapped.status, error = %mapped.message, "file delete failed");
                mapped
            })?;
            tracing::info!(user = %user_identity, path = %path.as_str(), "file deleted");
        }

        Ok(StatusCode::NO_CONTENT.into_response())
    }

    pub(crate) async fn inspect_policy(
        &self,
        url_path: &str,
    ) -> Result<Json<PolicyInspection>, HttpError> {
        let path = self.normalize_request_path(url_path)?;
        self.inspect_policy_rules(&path)
            .map(Json)
            .map_err(HttpError::internal)
    }

    pub(crate) async fn get_path_info(
        &self,
        url_path: &str,
        user_identity: &UserIdentity,
    ) -> Result<Json<PathInfo>, HttpError> {
        let path = self.authorized_path(url_path, MethodKind::Get, user_identity)?;
        let info = self.path_info_for_request(&path).await?;
        Ok(Json(info))
    }

    async fn path_info_for_request(&self, path: &WorkspacePath) -> Result<PathInfo, HttpError> {
        let info = self.repository.path_info(path).await.map_err(|error| {
            let mapped = HttpError::from_path_info(error);
            tracing::warn!(path = %path.as_str(), status = %mapped.status, error = %mapped.message, "metadata read failed");
            mapped
        })?;

        match (path.is_directory(), info.kind.clone()) {
            (true, PathInfoKind::File) => {
                Err(HttpError::bad_request("file path must not end with /"))
            }
            (false, PathInfoKind::Directory) => {
                Err(HttpError::bad_request("directory path must end with /"))
            }
            _ => Ok(info),
        }
    }

    async fn directory_response(
        &self,
        path: &WorkspacePath,
        user_identity: &UserIdentity,
    ) -> Result<Response, HttpError> {
        let entries = self.repository.list_directory(path).await.map_err(|error| {
            let mapped = HttpError::from_directory_listing(error);
            tracing::warn!(user = %user_identity, path = %path, status = %mapped.status, error = %mapped.message, "directory listing failed");
            mapped
        })?;
        let entries = self.filter_ignored_entries(path, entries)?;
        Ok(text_response(StatusCode::OK, entries.join("\n")))
    }

    fn plugin_runner(&self) -> PluginRunner<'_> {
        PluginRunner::new(
            self.repository.repository_root(),
            &self.config.name,
            &self.config,
        )
    }

    fn authorized_path(
        &self,
        url_path: &str,
        method: MethodKind,
        user_identity: &UserIdentity,
    ) -> Result<WorkspacePath, HttpError> {
        let path = self.normalize_request_path(url_path)?;
        self.enforce_not_ignored(&path)?;
        self.enforce_policy(method, &path, user_identity)?;
        Ok(path)
    }

    fn normalize_request_path(&self, path: &str) -> Result<WorkspacePath, HttpError> {
        WorkspacePath::from_url(path).map_err(HttpError::from_request_path)
    }

    fn enforce_policy(
        &self,
        method: MethodKind,
        path: &WorkspacePath,
        user_identity: &UserIdentity,
    ) -> Result<(), HttpError> {
        let allowed = self
            .resolve_policy(method, path, user_identity)
            .map_err(HttpError::internal)?
            .unwrap_or(false);

        if allowed {
            Ok(())
        } else {
            Err(HttpError::forbidden("operation denied by policy"))
        }
    }

    fn enforce_not_ignored(&self, path: &WorkspacePath) -> Result<(), HttpError> {
        if self.is_ignored_path(path) {
            Err(HttpError::forbidden("path ignored by config"))
        } else {
            Ok(())
        }
    }

    fn is_ignored_path(&self, path: &WorkspacePath) -> bool {
        self.config
            .ignore
            .paths
            .iter()
            .any(|ignored| path.as_str() == ignored.as_str() || path.starts_with(ignored))
    }

    fn filter_ignored_entries(
        &self,
        directory: &WorkspacePath,
        entries: Vec<String>,
    ) -> Result<Vec<String>, HttpError> {
        entries
            .into_iter()
            .map(|entry| {
                let path = child_path(directory, &entry).map_err(HttpError::internal)?;
                Ok((entry, path))
            })
            .filter(|result| {
                result
                    .as_ref()
                    .map(|(_, path)| !self.is_ignored_path(path))
                    .unwrap_or(true)
            })
            .map(|result| result.map(|(entry, _)| entry))
            .collect()
    }

    fn resolve_policy(
        &self,
        method: MethodKind,
        path: &WorkspacePath,
        user_identity: &UserIdentity,
    ) -> Result<Option<bool>> {
        resolve_policy(method, &self.config.policy, path, user_identity)
    }

    fn inspect_policy_rules(&self, path: &WorkspacePath) -> Result<PolicyInspection> {
        inspect_policy_rules(&self.config.policy, path)
    }
}

fn reject_directory_path(path: &WorkspacePath, message: &'static str) -> Result<(), HttpError> {
    if path.is_directory() {
        return Err(HttpError::bad_request(message));
    }
    Ok(())
}

fn child_path(directory: &WorkspacePath, entry: &str) -> Result<WorkspacePath> {
    if directory.as_str() == "." {
        WorkspacePath::from_path_str(entry)
    } else {
        WorkspacePath::from_path_str(&format!("{}/{entry}", directory.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use camino::Utf8PathBuf;

    use super::*;
    use crate::infra::repository_config::{IgnoreConfig, Policy, PolicyPermissions, ServeSettings};

    fn test_config_with_ignore(paths: Vec<WorkspacePath>) -> RepositoryConfig {
        RepositoryConfig {
            name: "repo".into(),
            serve: ServeSettings::default(),
            policy: vec![Policy {
                path: WorkspacePath::from_path_str(".").unwrap(),
                permissions: PolicyPermissions {
                    get: vec!["alice_browser".into()],
                    post: Vec::new(),
                    put: Vec::new(),
                    delete: Vec::new(),
                },
            }],
            ignore: IgnoreConfig { paths },
            plugin: Vec::new(),
        }
    }

    fn unique_temp_dir(name: &str) -> Utf8PathBuf {
        let unique = format!(
            "workspace-fs-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).unwrap();
        Utf8PathBuf::from_path_buf(path).unwrap()
    }

    #[test]
    fn path_info_requires_directory_suffix_for_directories() {
        let info = PathInfo::new("docs", PathInfoKind::Directory, None, None, false);

        assert_eq!(info.kind, PathInfoKind::Directory);
    }

    #[tokio::test]
    async fn ignore_hides_listing_entries_and_rejects_direct_access() {
        let root = unique_temp_dir("ignore");
        std::fs::create_dir(root.join(".git").as_std_path()).unwrap();
        std::fs::write(root.join("LICENSE").as_std_path(), "license").unwrap();
        std::fs::write(root.join("README.md").as_std_path(), "readme").unwrap();

        let config = Arc::new(test_config_with_ignore(vec![
            WorkspacePath::from_path_str(".git").unwrap(),
            WorkspacePath::from_path_str("LICENSE").unwrap(),
        ]));
        let repository = Arc::new(FsRepository::open(&root, &config).unwrap());
        let workspace = WorkspaceService::new(repository, config);
        let user = UserIdentity::new("alice_browser");

        let response = workspace.get_root(&user).await.unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let listing = String::from_utf8(body.to_vec()).unwrap();

        assert!(listing.contains("README.md"));
        assert!(!listing.contains(".git/"));
        assert!(!listing.contains("LICENSE"));

        let git_error = workspace.get_path("/.git/", &user).await.unwrap_err();
        assert_eq!(git_error.status, StatusCode::FORBIDDEN);
        let license_error = workspace.get_path("/LICENSE", &user).await.unwrap_err();
        assert_eq!(license_error.status, StatusCode::FORBIDDEN);

        let _ = std::fs::remove_dir_all(root);
    }
}
