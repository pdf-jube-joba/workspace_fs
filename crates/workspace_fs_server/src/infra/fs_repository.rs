use anyhow::{Context, Result, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};
use tokio::fs;

use crate::{
    domain::{
        path_info::{PathInfo, PathInfoKind},
        workspace_path::WorkspacePath,
    },
    infra::repository_config::RepositoryConfig,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RepositoryError {
    ReservedPath,
    ResolvedPathEscapesRepositoryRoot,
    SymlinkPathNotAllowed,
    ParentDirectoryNotFound,
    ParentPathNotDirectory,
    NotDirectory,
    DirectoryAlreadyExists,
    DirectoryNotFound,
    FileAlreadyExists,
    FileNotFound,
    PathIsDirectory,
    PathIsNotDirectory,
    DirectoryNotEmpty,
    NonUtf8Path,
    InvalidDirectoryEntry,
    Internal(String),
}

pub(crate) struct FsRepository {
    repository_root: Utf8PathBuf,
    mounts: Vec<MountedDirectory>,
    reserved_paths: Vec<WorkspacePath>,
}

struct MountedDirectory {
    alias: WorkspacePath,
    source: WorkspacePath,
}

impl FsRepository {
    pub fn open(path: &Utf8Path, config: &RepositoryConfig) -> Result<Self> {
        let canonical = std::fs::canonicalize(path)
            .with_context(|| format!("failed to resolve repository path: {}", path))?;
        let repository_root = Utf8PathBuf::from_path_buf(canonical)
            .map_err(|_| anyhow!("repository path must be UTF-8"))?;

        if !repository_root.is_dir() {
            bail!("repository path must be a directory");
        }

        Ok(Self {
            repository_root,
            mounts: Self::mounts_from_config(config),
            reserved_paths: Self::reserved_paths_from_config(config),
        })
    }

    pub fn repository_root(&self) -> &Utf8Path {
        &self.repository_root
    }

    fn mounts_from_config(config: &RepositoryConfig) -> Vec<MountedDirectory> {
        config
            .plugin
            .iter()
            .filter_map(|plugin| {
                plugin.mount.as_ref().map(|mount| MountedDirectory {
                    alias: WorkspacePath::from_path_str(mount.trim_start_matches('/'))
                        .expect("validated mount alias should parse"),
                    source: WorkspacePath::from_path_str(&format!(
                        ".repo/{}/generated",
                        plugin.name
                    ))
                    .expect("generated plugin path should parse"),
                })
            })
            .collect()
    }

    fn reserved_paths_from_config(config: &RepositoryConfig) -> Vec<WorkspacePath> {
        let mut reserved_paths = vec![
            WorkspacePath::from_path_str(".repo")
                .expect(".repo should always parse as workspace path"),
        ];
        for prefix in [
            config.serve.plugin_url_prefix.as_str(),
            config.serve.policy_url_prefix.as_str(),
            config.serve.info_url_prefix.as_str(),
        ] {
            reserved_paths.push(
                WorkspacePath::from_path_str(prefix.trim_start_matches('/'))
                    .expect("validated reserved prefix should parse"),
            );
        }
        reserved_paths
    }

    fn resolve_path(
        &self,
        requested_path: &WorkspacePath,
    ) -> std::result::Result<Utf8PathBuf, RepositoryError> {
        if let Some(resolved) = self.resolve_mounted_path(requested_path) {
            return Ok(resolved);
        }
        self.ensure_not_reserved_path(requested_path)?;
        Ok(requested_path.join_to(&self.repository_root))
    }

    fn ensure_parent_directory_exists(
        &self,
        path: &Utf8Path,
    ) -> std::result::Result<(), RepositoryError> {
        let Some(parent) = path.parent() else {
            return Err(RepositoryError::ParentDirectoryNotFound);
        };

        self.ensure_no_symlink_components(parent, false)?;
        if !parent.exists() {
            return Err(RepositoryError::ParentDirectoryNotFound);
        }
        let metadata = std::fs::symlink_metadata(parent.as_std_path()).map_err(|error| {
            RepositoryError::Internal(format!("failed to inspect parent directory: {error}"))
        })?;
        if !metadata.is_dir() {
            return Err(RepositoryError::ParentPathNotDirectory);
        }

        Ok(())
    }

    fn resolve_mounted_path(&self, requested_path: &WorkspacePath) -> Option<Utf8PathBuf> {
        let mount = self
            .mounts
            .iter()
            .find(|mount| requested_path.starts_with(&mount.alias))?;
        let relative = requested_path.strip_prefix(&mount.alias)?;
        let mut resolved = mount.source.join_to(&self.repository_root);
        if !relative.is_empty() {
            resolved.push(relative);
        }
        Some(resolved)
    }

    fn ensure_not_reserved_path(
        &self,
        requested_path: &WorkspacePath,
    ) -> std::result::Result<(), RepositoryError> {
        if self
            .reserved_paths
            .iter()
            .any(|reserved_path| requested_path.starts_with(reserved_path))
        {
            return Err(RepositoryError::ReservedPath);
        }
        Ok(())
    }

    fn ensure_no_symlink_components(
        &self,
        path: &Utf8Path,
        allow_missing_final: bool,
    ) -> std::result::Result<(), RepositoryError> {
        let relative = path
            .strip_prefix(&self.repository_root)
            .map_err(|_| RepositoryError::ResolvedPathEscapesRepositoryRoot)?;
        let mut current = self.repository_root.clone();

        if let Ok(metadata) = std::fs::symlink_metadata(current.as_std_path())
            && metadata.file_type().is_symlink()
        {
            return Err(RepositoryError::SymlinkPathNotAllowed);
        }

        for component in relative.components() {
            current.push(component.as_str());
            match std::fs::symlink_metadata(current.as_std_path()) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() {
                        return Err(RepositoryError::SymlinkPathNotAllowed);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if allow_missing_final && current == path {
                        break;
                    }
                    break;
                }
                Err(error) => {
                    return Err(RepositoryError::Internal(format!(
                        "failed to inspect path: {error}"
                    )));
                }
            }
        }

        Ok(())
    }

    fn read_directory_entries(
        &self,
        directory: &Utf8Path,
    ) -> std::result::Result<Vec<String>, RepositoryError> {
        let mut entries = Vec::new();
        let read_dir = std::fs::read_dir(directory.as_std_path()).map_err(|error| {
            RepositoryError::Internal(format!("failed to read directory: {error}"))
        })?;
        for dir_entry in read_dir {
            let dir_entry = dir_entry.map_err(|error| {
                RepositoryError::Internal(format!("failed to read directory entry: {error}"))
            })?;
            let path = dir_entry.path();
            let utf8 =
                Utf8PathBuf::from_path_buf(path).map_err(|_| RepositoryError::NonUtf8Path)?;
            let metadata = std::fs::symlink_metadata(utf8.as_std_path()).map_err(|error| {
                RepositoryError::Internal(format!("failed to inspect directory entry: {error}"))
            })?;
            if metadata.file_type().is_symlink() {
                return Err(RepositoryError::SymlinkPathNotAllowed);
            }

            let mut entry = utf8
                .file_name()
                .ok_or(RepositoryError::InvalidDirectoryEntry)?
                .to_owned();

            if metadata.is_dir() {
                entry.push('/');
            }

            entries.push(entry);
        }

        entries.sort();
        Ok(entries)
    }

    pub async fn list_directory(
        &self,
        path: &WorkspacePath,
    ) -> std::result::Result<Vec<String>, RepositoryError> {
        let directory = self.resolve_path(path)?;
        self.ensure_no_symlink_components(&directory, false)?;
        if !directory.exists() {
            return Err(RepositoryError::DirectoryNotFound);
        }
        if !directory.is_dir() {
            return Err(RepositoryError::NotDirectory);
        }

        self.read_directory_entries(&directory)
    }

    pub async fn path_info(
        &self,
        path: &WorkspacePath,
    ) -> std::result::Result<PathInfo, RepositoryError> {
        let resolved = self.resolve_path(path)?;
        self.ensure_no_symlink_components(&resolved, false)?;
        if !resolved.exists() {
            return Err(RepositoryError::FileNotFound);
        }
        let metadata = fs::metadata(resolved.as_std_path())
            .await
            .map_err(|error| {
                RepositoryError::Internal(format!("failed to read metadata: {error}"))
            })?;
        let kind = if metadata.is_dir() {
            PathInfoKind::Directory
        } else {
            PathInfoKind::File
        };
        let size = match kind {
            PathInfoKind::File => Some(metadata.len()),
            PathInfoKind::Directory => None,
        };

        Ok(PathInfo::new(
            path.as_str(),
            kind,
            size,
            metadata.modified().ok(),
            metadata.permissions().readonly(),
        ))
    }

    pub async fn create_directory(
        &self,
        path: &WorkspacePath,
    ) -> std::result::Result<(), RepositoryError> {
        let resolved = self.resolve_path(path)?;
        self.ensure_no_symlink_components(&resolved, true)?;

        if resolved.exists() {
            return Err(RepositoryError::DirectoryAlreadyExists);
        }

        self.ensure_parent_directory_exists(&resolved)?;

        fs::create_dir(resolved.as_std_path())
            .await
            .map_err(|error| {
                RepositoryError::Internal(format!("failed to create directory: {error}"))
            })?;
        Ok(())
    }

    pub async fn delete_directory(
        &self,
        path: &WorkspacePath,
    ) -> std::result::Result<(), RepositoryError> {
        let resolved = self.resolve_path(path)?;
        self.ensure_no_symlink_components(&resolved, false)?;

        if !resolved.exists() {
            return Err(RepositoryError::DirectoryNotFound);
        }

        if !resolved.is_dir() {
            return Err(RepositoryError::PathIsNotDirectory);
        }

        if std::fs::read_dir(resolved.as_std_path())
            .map_err(|error| {
                RepositoryError::Internal(format!("failed to read directory: {error}"))
            })?
            .next()
            .is_some()
        {
            return Err(RepositoryError::DirectoryNotEmpty);
        }

        fs::remove_dir(resolved.as_std_path())
            .await
            .map_err(|error| {
                RepositoryError::Internal(format!("failed to delete directory: {error}"))
            })?;
        Ok(())
    }

    pub async fn read_file(
        &self,
        path: &WorkspacePath,
    ) -> std::result::Result<Vec<u8>, RepositoryError> {
        let resolved = self.resolve_path(path)?;
        self.ensure_no_symlink_components(&resolved, false)?;
        if !resolved.exists() {
            return Err(RepositoryError::FileNotFound);
        }
        if resolved.is_dir() {
            return Err(RepositoryError::PathIsDirectory);
        }
        fs::read(resolved.as_std_path())
            .await
            .map_err(|error| RepositoryError::Internal(format!("failed to read file: {error}")))
    }

    pub async fn create_text_file(
        &self,
        path: &WorkspacePath,
        content: &str,
    ) -> std::result::Result<(), RepositoryError> {
        let resolved = self.resolve_path(path)?;
        self.ensure_no_symlink_components(&resolved, true)?;

        if resolved.exists() {
            return Err(RepositoryError::FileAlreadyExists);
        }

        self.ensure_parent_directory_exists(&resolved)?;

        fs::write(resolved.as_std_path(), content)
            .await
            .map_err(|error| {
                RepositoryError::Internal(format!("failed to create file: {error}"))
            })?;
        Ok(())
    }

    pub async fn write_text_file(
        &self,
        path: &WorkspacePath,
        content: &str,
    ) -> std::result::Result<(), RepositoryError> {
        let resolved = self.resolve_path(path)?;
        self.ensure_no_symlink_components(&resolved, false)?;

        if !resolved.exists() {
            return Err(RepositoryError::FileNotFound);
        }

        if resolved.is_dir() {
            return Err(RepositoryError::PathIsDirectory);
        }

        fs::write(resolved.as_std_path(), content)
            .await
            .map_err(|error| RepositoryError::Internal(format!("failed to write file: {error}")))?;
        Ok(())
    }

    pub async fn delete_file(
        &self,
        path: &WorkspacePath,
    ) -> std::result::Result<(), RepositoryError> {
        let resolved = self.resolve_path(path)?;
        self.ensure_no_symlink_components(&resolved, false)?;

        if !resolved.exists() {
            return Err(RepositoryError::FileNotFound);
        }

        if resolved.is_dir() {
            return Err(RepositoryError::PathIsDirectory);
        }

        fs::remove_file(resolved.as_std_path())
            .await
            .map_err(|error| {
                RepositoryError::Internal(format!("failed to delete file: {error}"))
            })?;
        Ok(())
    }
}

impl std::fmt::Display for RepositoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::ReservedPath => "reserved path",
            Self::ResolvedPathEscapesRepositoryRoot => "resolved path escapes repository root",
            Self::SymlinkPathNotAllowed => "symlink path is not allowed",
            Self::ParentDirectoryNotFound => "parent directory not found",
            Self::ParentPathNotDirectory => "parent path is not a directory",
            Self::NotDirectory => "not a directory",
            Self::DirectoryAlreadyExists => "directory already exists",
            Self::DirectoryNotFound => "directory not found",
            Self::FileAlreadyExists => "file already exists",
            Self::FileNotFound => "file not found",
            Self::PathIsDirectory => "path is a directory",
            Self::PathIsNotDirectory => "path is not a directory",
            Self::DirectoryNotEmpty => "directory is not empty",
            Self::NonUtf8Path => "non-UTF-8 path",
            Self::InvalidDirectoryEntry => "invalid directory entry",
            Self::Internal(message) => return f.write_str(message),
        };
        f.write_str(message)
    }
}

impl std::error::Error for RepositoryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::repository_config::{IgnoreConfig, ServeSettings};

    fn test_config() -> RepositoryConfig {
        RepositoryConfig {
            name: "repo".into(),
            serve: ServeSettings::default(),
            policy: Vec::new(),
            ignore: IgnoreConfig::default(),
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

    #[cfg(unix)]
    #[tokio::test]
    async fn read_file_rejects_symlink_ancestor() {
        use std::os::unix::fs::symlink;

        let root = unique_temp_dir("read-symlink");
        let outside = unique_temp_dir("outside-read");
        std::fs::write(outside.join("secret.txt").as_std_path(), "secret").unwrap();
        symlink(outside.as_std_path(), root.join("secret").as_std_path()).unwrap();

        let repository = FsRepository::open(&root, &test_config()).unwrap();
        let error = repository
            .read_file(&WorkspacePath::from_path_str("secret/secret.txt").unwrap())
            .await
            .unwrap_err();

        assert!(error.to_string().contains("symlink path is not allowed"));

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn write_text_file_rejects_symlink_ancestor() {
        use std::os::unix::fs::symlink;

        let root = unique_temp_dir("write-symlink");
        let outside = unique_temp_dir("outside-write");
        std::fs::write(outside.join("target.txt").as_std_path(), "before").unwrap();
        symlink(outside.as_std_path(), root.join("secret").as_std_path()).unwrap();

        let repository = FsRepository::open(&root, &test_config()).unwrap();
        let error = repository
            .write_text_file(
                &WorkspacePath::from_path_str("secret/target.txt").unwrap(),
                "after",
            )
            .await
            .unwrap_err();

        assert!(error.to_string().contains("symlink path is not allowed"));

        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(outside);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reserved_virtual_prefix_is_rejected_as_repository_path() {
        let root = unique_temp_dir("reserved-prefix");
        let repository = FsRepository::open(&root, &test_config()).unwrap();
        let error = repository
            .read_file(&WorkspacePath::from_path_str(".info/cache.txt").unwrap())
            .await
            .unwrap_err();

        assert!(error.to_string().contains("reserved path"));

        let _ = std::fs::remove_dir_all(root);
    }
}
