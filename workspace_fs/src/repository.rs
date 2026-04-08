use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use camino::{Utf8Path, Utf8PathBuf};
use tokio::fs;

use crate::{
    config::RepositoryConfig,
    info::{PathInfo, PathInfoKind},
    path::WorkspacePath,
};

// .repo 以外を書き換えるインターフェースの提供
#[async_trait]
pub trait Repository: Send + Sync {
    async fn list_directory(&self, path: &WorkspacePath) -> Result<Vec<String>>;
    async fn path_info(&self, path: &WorkspacePath) -> Result<PathInfo>;
    async fn create_directory(&self, path: &WorkspacePath) -> Result<()>;
    async fn delete_directory(&self, path: &WorkspacePath) -> Result<()>;
    async fn read_file(&self, path: &WorkspacePath) -> Result<Vec<u8>>;
    async fn create_text_file(&self, path: &WorkspacePath, content: &str) -> Result<()>;
    async fn write_text_file(&self, path: &WorkspacePath, content: &str) -> Result<()>;
    async fn delete_file(&self, path: &WorkspacePath) -> Result<()>;
}

pub struct FsRepository {
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

    fn resolve_path(&self, requested_path: &WorkspacePath) -> Result<Utf8PathBuf> {
        if let Some(resolved) = self.resolve_mounted_path(requested_path) {
            return Ok(resolved);
        }
        self.ensure_not_reserved_path(requested_path)?;
        Ok(requested_path.join_to(&self.repository_root))
    }

    fn ensure_parent_directory_exists(&self, path: &Utf8Path) -> Result<()> {
        let Some(parent) = path.parent() else {
            bail!("parent directory not found");
        };

        self.ensure_no_symlink_components(parent, false)?;
        let metadata = std::fs::symlink_metadata(parent.as_std_path())
            .context("failed to inspect parent directory")?;
        if !metadata.is_dir() {
            bail!("parent path is not a directory");
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

    fn ensure_not_reserved_path(&self, requested_path: &WorkspacePath) -> Result<()> {
        if self
            .reserved_paths
            .iter()
            .any(|reserved_path| requested_path.starts_with(reserved_path))
        {
            bail!("reserved path");
        }
        Ok(())
    }

    fn ensure_no_symlink_components(
        &self,
        path: &Utf8Path,
        allow_missing_final: bool,
    ) -> Result<()> {
        let relative = path
            .strip_prefix(&self.repository_root)
            .map_err(|_| anyhow!("resolved path escapes repository root"))?;
        let mut current = self.repository_root.clone();

        if let Ok(metadata) = std::fs::symlink_metadata(current.as_std_path())
            && metadata.file_type().is_symlink()
        {
            bail!("symlink path is not allowed");
        }

        for component in relative.components() {
            current.push(component.as_str());
            match std::fs::symlink_metadata(current.as_std_path()) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() {
                        bail!("symlink path is not allowed");
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if allow_missing_final && current == path {
                        break;
                    }
                    break;
                }
                Err(error) => {
                    return Err(error).context("failed to inspect path");
                }
            }
        }

        Ok(())
    }

    fn read_directory_entries(&self, directory: &Utf8Path) -> Result<Vec<String>> {
        let mut entries = Vec::new();
        for dir_entry in std::fs::read_dir(directory.as_std_path())? {
            let dir_entry = dir_entry?;
            let path = dir_entry.path();
            let utf8 = Utf8PathBuf::from_path_buf(path).map_err(|_| anyhow!("non-UTF-8 path"))?;
            let metadata = std::fs::symlink_metadata(utf8.as_std_path())
                .context("failed to inspect directory entry")?;
            if metadata.file_type().is_symlink() {
                bail!("symlink path is not allowed");
            }

            let mut entry = utf8
                .file_name()
                .ok_or_else(|| anyhow!("invalid directory entry"))?
                .to_owned();

            if metadata.is_dir() {
                entry.push('/');
            }

            entries.push(entry);
        }

        entries.sort();
        Ok(entries)
    }
}

#[async_trait]
impl Repository for FsRepository {
    async fn list_directory(&self, path: &WorkspacePath) -> Result<Vec<String>> {
        let directory = self.resolve_path(path)?;
        self.ensure_no_symlink_components(&directory, false)?;
        if !directory.is_dir() {
            bail!("not a directory");
        }

        self.read_directory_entries(&directory)
    }

    async fn path_info(&self, path: &WorkspacePath) -> Result<PathInfo> {
        let resolved = self.resolve_path(path)?;
        self.ensure_no_symlink_components(&resolved, false)?;
        let metadata = fs::metadata(resolved.as_std_path())
            .await
            .context("failed to read metadata")?;
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

    async fn create_directory(&self, path: &WorkspacePath) -> Result<()> {
        let resolved = self.resolve_path(path)?;
        self.ensure_no_symlink_components(&resolved, true)?;

        if resolved.exists() {
            bail!("directory already exists");
        }

        self.ensure_parent_directory_exists(&resolved)?;

        fs::create_dir(resolved.as_std_path())
            .await
            .context("failed to create directory")?;
        Ok(())
    }

    async fn delete_directory(&self, path: &WorkspacePath) -> Result<()> {
        let resolved = self.resolve_path(path)?;
        self.ensure_no_symlink_components(&resolved, false)?;

        if !resolved.exists() {
            bail!("directory not found");
        }

        if !resolved.is_dir() {
            bail!("path is not a directory");
        }

        if std::fs::read_dir(resolved.as_std_path())?.next().is_some() {
            bail!("directory is not empty");
        }

        fs::remove_dir(resolved.as_std_path())
            .await
            .context("failed to delete directory")?;
        Ok(())
    }

    async fn read_file(&self, path: &WorkspacePath) -> Result<Vec<u8>> {
        let resolved = self.resolve_path(path)?;
        self.ensure_no_symlink_components(&resolved, false)?;
        fs::read(resolved.as_std_path())
            .await
            .context("failed to read file")
    }

    async fn create_text_file(&self, path: &WorkspacePath, content: &str) -> Result<()> {
        let resolved = self.resolve_path(path)?;
        self.ensure_no_symlink_components(&resolved, true)?;

        if resolved.exists() {
            bail!("file already exists");
        }

        self.ensure_parent_directory_exists(&resolved)?;

        fs::write(resolved.as_std_path(), content)
            .await
            .context("failed to create file")?;
        Ok(())
    }

    async fn write_text_file(&self, path: &WorkspacePath, content: &str) -> Result<()> {
        let resolved = self.resolve_path(path)?;
        self.ensure_no_symlink_components(&resolved, false)?;

        if !resolved.exists() {
            bail!("file not found");
        }

        if resolved.is_dir() {
            bail!("path is a directory");
        }

        fs::write(resolved.as_std_path(), content)
            .await
            .context("failed to write file")?;
        Ok(())
    }

    async fn delete_file(&self, path: &WorkspacePath) -> Result<()> {
        let resolved = self.resolve_path(path)?;
        self.ensure_no_symlink_components(&resolved, false)?;

        if !resolved.exists() {
            bail!("file not found");
        }

        if resolved.is_dir() {
            bail!("path is a directory");
        }

        fs::remove_file(resolved.as_std_path())
            .await
            .context("failed to delete file")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{IgnoreConfig, ServeSettings, TaskConfig};

    fn test_config() -> RepositoryConfig {
        RepositoryConfig {
            name: "repo".into(),
            serve: ServeSettings::default(),
            policy: Vec::new(),
            ignore: IgnoreConfig::default(),
            plugin: Vec::new(),
            task: Vec::<TaskConfig>::new(),
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
