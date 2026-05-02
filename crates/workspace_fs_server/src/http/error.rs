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

    pub fn from_path_info(error: Error) -> Self {
        match repository_error(&error) {
            Some(RepositoryError::FileNotFound | RepositoryError::DirectoryNotFound) => {
                Self::not_found("path not found")
            }
            Some(
                RepositoryError::ReservedPath
                | RepositoryError::ResolvedPathEscapesRepositoryRoot
                | RepositoryError::NonUtf8Path
                | RepositoryError::InvalidDirectoryEntry,
            ) => Self::bad_request(error.to_string()),
            Some(RepositoryError::SymlinkPathNotAllowed) => Self::bad_request(error.to_string()),
            Some(_) => Self::internal(error),
            None => Self::internal(error),
        }
    }

    pub fn from_directory_listing(error: Error) -> Self {
        match repository_error(&error) {
            Some(RepositoryError::DirectoryNotFound | RepositoryError::NotDirectory) => {
                Self::not_found("directory not found")
            }
            Some(
                RepositoryError::ReservedPath | RepositoryError::ResolvedPathEscapesRepositoryRoot,
            ) => Self::bad_request(error.to_string()),
            Some(RepositoryError::SymlinkPathNotAllowed) => Self::bad_request(error.to_string()),
            Some(_) => Self::internal(error),
            None => Self::internal(error),
        }
    }

    pub fn from_read_file(error: Error) -> Self {
        match repository_error(&error) {
            Some(RepositoryError::FileNotFound) => Self::not_found("path not found"),
            Some(RepositoryError::PathIsDirectory) => Self::bad_request("path is a directory"),
            Some(
                RepositoryError::ReservedPath | RepositoryError::ResolvedPathEscapesRepositoryRoot,
            ) => Self::bad_request(error.to_string()),
            Some(RepositoryError::SymlinkPathNotAllowed) => Self::bad_request(error.to_string()),
            Some(_) => Self::internal(error),
            None => Self::internal(error),
        }
    }

    pub fn from_create_file(error: Error) -> Self {
        match repository_error(&error) {
            Some(RepositoryError::FileAlreadyExists) => Self::conflict("file already exists"),
            Some(RepositoryError::ParentDirectoryNotFound) => {
                Self::not_found("parent directory not found")
            }
            Some(RepositoryError::ParentPathNotDirectory) => {
                Self::bad_request("parent path is not a directory")
            }
            Some(
                RepositoryError::ReservedPath | RepositoryError::ResolvedPathEscapesRepositoryRoot,
            ) => Self::bad_request(error.to_string()),
            Some(RepositoryError::SymlinkPathNotAllowed) => Self::bad_request(error.to_string()),
            Some(_) => Self::internal(error),
            None => Self::internal(error),
        }
    }

    pub fn from_create_directory(error: Error) -> Self {
        match repository_error(&error) {
            Some(RepositoryError::DirectoryAlreadyExists) => {
                Self::conflict("directory already exists")
            }
            Some(RepositoryError::ParentDirectoryNotFound) => {
                Self::not_found("parent directory not found")
            }
            Some(RepositoryError::ParentPathNotDirectory) => {
                Self::bad_request("parent path is not a directory")
            }
            Some(
                RepositoryError::ReservedPath | RepositoryError::ResolvedPathEscapesRepositoryRoot,
            ) => Self::bad_request(error.to_string()),
            Some(RepositoryError::SymlinkPathNotAllowed) => Self::bad_request(error.to_string()),
            Some(_) => Self::internal(error),
            None => Self::internal(error),
        }
    }

    pub fn from_write_file(error: Error) -> Self {
        match repository_error(&error) {
            Some(RepositoryError::FileNotFound) => Self::not_found("file not found"),
            Some(RepositoryError::PathIsDirectory) => Self::bad_request("path is a directory"),
            Some(
                RepositoryError::ReservedPath | RepositoryError::ResolvedPathEscapesRepositoryRoot,
            ) => Self::bad_request(error.to_string()),
            Some(RepositoryError::SymlinkPathNotAllowed) => Self::bad_request(error.to_string()),
            Some(_) => Self::internal(error),
            None => Self::internal(error),
        }
    }

    pub fn from_delete_file(error: Error) -> Self {
        match repository_error(&error) {
            Some(RepositoryError::FileNotFound) => Self::not_found("file not found"),
            Some(RepositoryError::PathIsDirectory) => Self::bad_request("path is a directory"),
            Some(
                RepositoryError::ReservedPath | RepositoryError::ResolvedPathEscapesRepositoryRoot,
            ) => Self::bad_request(error.to_string()),
            Some(RepositoryError::SymlinkPathNotAllowed) => Self::bad_request(error.to_string()),
            Some(_) => Self::internal(error),
            None => Self::internal(error),
        }
    }

    pub fn from_delete_directory(error: Error) -> Self {
        match repository_error(&error) {
            Some(RepositoryError::DirectoryNotFound) => Self::not_found("directory not found"),
            Some(RepositoryError::PathIsNotDirectory) => {
                Self::bad_request("path is not a directory")
            }
            Some(RepositoryError::DirectoryNotEmpty) => Self::conflict("directory is not empty"),
            Some(
                RepositoryError::ReservedPath | RepositoryError::ResolvedPathEscapesRepositoryRoot,
            ) => Self::bad_request(error.to_string()),
            Some(RepositoryError::SymlinkPathNotAllowed) => Self::bad_request(error.to_string()),
            Some(_) => Self::internal(error),
            None => Self::internal(error),
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

fn repository_error(error: &Error) -> Option<&RepositoryError> {
    error.downcast_ref::<RepositoryError>()
}
