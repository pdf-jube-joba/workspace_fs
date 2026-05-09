use anyhow::Error;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::{domain::workspace_path::PathError, infra::fs_repository::RepositoryError};

#[derive(Debug)]
pub struct HttpError {
    pub status: StatusCode,
    pub message: String,
}

impl HttpError {
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

    pub fn from_request_path(error: Error) -> Self {
        match error.downcast_ref::<PathError>() {
            Some(PathError::UrlPathMustStartWithSlash | PathError::PathRequired) => {
                Self::bad_request(error.to_string())
            }
            Some(
                PathError::AbsolutePathNotAllowed
                | PathError::PathEscapesRepositoryRoot
                | PathError::ReservedPath,
            ) => Self::bad_request(error.to_string()),
            Some(PathError::PathMustBeUtf8) => Self::bad_request(error.to_string()),
            None => Self::internal(error),
        }
    }

    pub(crate) fn from_path_info(error: RepositoryError) -> Self {
        match error {
            RepositoryError::FileNotFound | RepositoryError::DirectoryNotFound => {
                Self::not_found("path not found")
            }
            error => Self::from_repository_error(error),
        }
    }

    pub(crate) fn from_directory_listing(error: RepositoryError) -> Self {
        match error {
            RepositoryError::DirectoryNotFound | RepositoryError::NotDirectory => {
                Self::not_found("directory not found")
            }
            error => Self::from_repository_error(error),
        }
    }

    pub(crate) fn from_read_file(error: RepositoryError) -> Self {
        match error {
            RepositoryError::FileNotFound => Self::not_found("path not found"),
            RepositoryError::PathIsDirectory => Self::bad_request("path is a directory"),
            error => Self::from_repository_error(error),
        }
    }

    pub(crate) fn from_create_file(error: RepositoryError) -> Self {
        match error {
            RepositoryError::FileAlreadyExists => Self::conflict("file already exists"),
            RepositoryError::ParentDirectoryNotFound => {
                Self::not_found("parent directory not found")
            }
            RepositoryError::ParentPathNotDirectory => {
                Self::bad_request("parent path is not a directory")
            }
            error => Self::from_repository_error(error),
        }
    }

    pub(crate) fn from_create_directory(error: RepositoryError) -> Self {
        match error {
            RepositoryError::DirectoryAlreadyExists => Self::conflict("directory already exists"),
            RepositoryError::ParentDirectoryNotFound => {
                Self::not_found("parent directory not found")
            }
            RepositoryError::ParentPathNotDirectory => {
                Self::bad_request("parent path is not a directory")
            }
            error => Self::from_repository_error(error),
        }
    }

    pub(crate) fn from_write_file(error: RepositoryError) -> Self {
        match error {
            RepositoryError::FileNotFound => Self::not_found("file not found"),
            RepositoryError::PathIsDirectory => Self::bad_request("path is a directory"),
            error => Self::from_repository_error(error),
        }
    }

    pub(crate) fn from_delete_file(error: RepositoryError) -> Self {
        match error {
            RepositoryError::FileNotFound => Self::not_found("file not found"),
            RepositoryError::PathIsDirectory => Self::bad_request("path is a directory"),
            error => Self::from_repository_error(error),
        }
    }

    pub(crate) fn from_delete_directory(error: RepositoryError) -> Self {
        match error {
            RepositoryError::DirectoryNotFound => Self::not_found("directory not found"),
            RepositoryError::PathIsNotDirectory => Self::bad_request("path is not a directory"),
            RepositoryError::DirectoryNotEmpty => Self::conflict("directory is not empty"),
            error => Self::from_repository_error(error),
        }
    }

    fn from_repository_error(error: RepositoryError) -> Self {
        match error {
            RepositoryError::ReservedPath
            | RepositoryError::ResolvedPathEscapesRepositoryRoot
            | RepositoryError::SymlinkPathNotAllowed
            | RepositoryError::NonUtf8Path
            | RepositoryError::InvalidDirectoryEntry => Self::bad_request(error.to_string()),
            _ => Self::internal(error),
        }
    }
}

impl IntoResponse for HttpError {
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
