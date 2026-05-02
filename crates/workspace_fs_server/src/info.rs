use serde::Serialize;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PathInfoKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PathInfo {
    pub path: String,
    pub kind: PathInfoKind,
    pub size: Option<u64>,
    pub modified_at: Option<String>,
    pub readonly: bool,
}

impl PathInfo {
    pub fn new(
        path: &str,
        kind: PathInfoKind,
        size: Option<u64>,
        modified_at: Option<std::time::SystemTime>,
        readonly: bool,
    ) -> Self {
        Self {
            path: path.to_owned(),
            kind,
            size,
            modified_at: modified_at.and_then(format_system_time),
            readonly,
        }
    }
}

fn format_system_time(time: std::time::SystemTime) -> Option<String> {
    let datetime = OffsetDateTime::from(time);
    datetime.format(&Rfc3339).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_file_info() {
        let info = PathInfo::new(
            "docs/a.md",
            PathInfoKind::File,
            Some(42),
            Some(std::time::UNIX_EPOCH),
            false,
        );

        assert_eq!(info.path, "docs/a.md");
        assert_eq!(info.kind, PathInfoKind::File);
        assert_eq!(info.size, Some(42));
        assert_eq!(info.modified_at.as_deref(), Some("1970-01-01T00:00:00Z"));
        assert!(!info.readonly);
    }
}
