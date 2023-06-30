//! Error type which can be returned from webserver handler functions

use actix_web::http::StatusCode;
use anyhow::anyhow;
use thiserror::Error;

#[derive(Debug, Error)]
#[allow(missing_docs)]
pub enum HandlerError {
    #[error("General error: {0}")]
    InternalError(anyhow::Error),
    #[error("Tokio join error: {0}")]
    TokioRuntimeError(#[from] tokio::task::JoinError),
    #[error("Failed to find or read image file due to error: {0}")]
    CouldntOpenImageFile(std::io::Error),
    #[error("Failed to load image data due to error: {0}")]
    FailedToLoadImage(std::io::Error),
    #[error("Image hosting is disabled")]
    ImageHostingDisabled(),
    #[error("Illegal file path requested: {0}")]
    FilePathNotAllowed(String),
    #[error("Failed to interpret path due to error: {0}")]
    InvalidPath(std::io::Error),
    #[error("Couldn't find image at {0}")]
    ImageDoesNotExist(String),
    #[error("Even the error handler failed, good luck")]
    ErrorHandlerFailed(),
    #[error("Uploaded file size ({0}B) exceeded the maximum upload size ({1}B)")]
    FileTooLarge(u64, u64),
    #[error("Uploaded file was not an image, was instead of type {0:?}")]
    FileWasNotAnImage(Option<infer::Type>),
    #[error("Failed to extract data from multipart form")]
    FieldReadError { field_name: String, cause: String },
}

impl actix_web::error::ResponseError for HandlerError {
    fn status_code(&self) -> actix_web::http::StatusCode {
        match self {
            HandlerError::InternalError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            HandlerError::CouldntOpenImageFile(_) => StatusCode::NOT_FOUND,
            HandlerError::ImageHostingDisabled() => StatusCode::FORBIDDEN,
            HandlerError::FilePathNotAllowed(_) => StatusCode::FORBIDDEN,
            HandlerError::InvalidPath(_) => StatusCode::INTERNAL_SERVER_ERROR,
            HandlerError::ErrorHandlerFailed() => StatusCode::IM_A_TEAPOT,
            HandlerError::ImageDoesNotExist(_) => StatusCode::NOT_FOUND,
            HandlerError::FailedToLoadImage(_) => StatusCode::INTERNAL_SERVER_ERROR,
            HandlerError::TokioRuntimeError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            HandlerError::FileTooLarge(_, _) => StatusCode::PAYLOAD_TOO_LARGE,
            HandlerError::FileWasNotAnImage(_) => StatusCode::BAD_REQUEST,
            HandlerError::FieldReadError {
                field_name: _,
                cause: _,
            } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<actix_multipart::MultipartError> for HandlerError {
    fn from(value: actix_multipart::MultipartError) -> Self {
        match value {
            actix_multipart::MultipartError::Field { field_name, source } => Self::FieldReadError {
                field_name,
                cause: format!("{}", source),
            },
            _ => Self::InternalError(anyhow!("{}", value)),
        }
    }
}

impl From<anyhow::Error> for HandlerError {
    fn from(err: anyhow::Error) -> HandlerError {
        if let Some(_cause) = err.downcast_ref::<std::io::Error>() {
            if let Ok(e) = err.downcast::<std::io::Error>() {
                HandlerError::CouldntOpenImageFile(e)
            } else {
                HandlerError::ErrorHandlerFailed()
            }
        } else {
            //Fallback
            HandlerError::InternalError(err)
        }
    }
}
