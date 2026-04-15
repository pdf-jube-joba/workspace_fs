use std::path::{Component, Path};

use anyhow::{Result, anyhow, bail};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkspacePath {
    path: Utf8PathBuf,
    is_directory: bool,
}

impl WorkspacePath {
    pub(crate) fn from_url(path: &str) -> Result<Self> {
        if path == "/" {
            return Ok(Self::from_parts(Utf8PathBuf::new(), true));
        }

        let is_directory = path.trim_end().ends_with('/');
        let normalized_input = path
            .strip_prefix('/')
            .ok_or_else(|| anyhow!("URL path must start with /"))?
            .trim()
            .trim_end_matches('/');
        if normalized_input.is_empty() {
            bail!("path is required");
        }

        let path = Self::from_path_str(normalized_input)?;
        if path.is_reserved() {
            bail!("reserved path");
        }

        Ok(Self {
            path: path.path,
            is_directory,
        })
    }

    pub(crate) fn from_path_str(path: &str) -> Result<Self> {
        let trimmed = path.trim();
        let is_directory = trimmed.ends_with('/');
        let normalized_input = trimmed.trim_end_matches('/');
        if normalized_input.is_empty() {
            return Ok(Self::from_parts(Utf8PathBuf::new(), false));
        }

        let path = Path::new(normalized_input);
        if path.is_absolute() {
            bail!("absolute paths are not allowed");
        }

        let mut normalized = Utf8PathBuf::new();
        for component in path.components() {
            match component {
                Component::Normal(part) => {
                    let part = part.to_str().ok_or_else(|| anyhow!("path must be UTF-8"))?;
                    normalized.push(part);
                }
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    bail!("path escapes repository root");
                }
            }
        }

        Ok(Self::from_parts(normalized, is_directory))
    }

    fn from_parts(path: Utf8PathBuf, is_directory: bool) -> Self {
        let path = if path.as_str().is_empty() {
            Utf8PathBuf::from(".")
        } else {
            path
        };
        Self { path, is_directory }
    }

    pub(crate) fn as_str(&self) -> &str {
        self.path.as_str()
    }

    pub(crate) fn is_directory(&self) -> bool {
        self.is_directory
    }

    pub(crate) fn is_reserved(&self) -> bool {
        matches!(self.path.components().next(), Some(component) if component.as_str() == ".repo")
    }

    pub(crate) fn join_to(&self, base: &Utf8Path) -> Utf8PathBuf {
        base.join(self.as_str())
    }

    pub(crate) fn starts_with(&self, other: &WorkspacePath) -> bool {
        self.path.starts_with(&other.path)
    }

    pub(crate) fn strip_prefix<'a>(&'a self, other: &WorkspacePath) -> Option<&'a str> {
        self.path
            .strip_prefix(&other.path)
            .ok()
            .map(Utf8Path::as_str)
    }

    pub(crate) fn depth(&self) -> usize {
        match self.as_str() {
            "." | "" => 0,
            value => value.split('/').count(),
        }
    }
}

impl std::fmt::Display for WorkspacePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for WorkspacePath {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_url_rejects_reserved_paths() {
        let error = WorkspacePath::from_url("/.repo/config.toml").unwrap_err();

        assert!(error.to_string().contains("reserved path"));
    }

    #[test]
    fn from_url_requires_leading_slash() {
        let error = WorkspacePath::from_url("docs/a.md").unwrap_err();

        assert!(error.to_string().contains("URL path must start with /"));
    }

    #[test]
    fn from_url_strips_leading_slash() {
        let normalized = WorkspacePath::from_url("/docs/").unwrap();

        assert_eq!(normalized.as_str(), "docs");
        assert!(normalized.is_directory());
    }

    #[test]
    fn from_url_root_uses_dot() {
        let normalized = WorkspacePath::from_url("/").unwrap();

        assert_eq!(normalized.as_str(), ".");
        assert!(normalized.is_directory());
    }

    #[test]
    fn from_url_preserves_directory_suffix() {
        let normalized = WorkspacePath::from_url("/docs/").unwrap();

        assert_eq!(normalized.as_str(), "docs");
        assert!(normalized.is_directory());
    }

    #[test]
    fn from_url_rejects_whitespace_only_input() {
        let error = WorkspacePath::from_url("   ").unwrap_err();

        assert!(error.to_string().contains("URL path must start with /"));
    }
}
