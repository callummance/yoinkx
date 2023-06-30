//! Helpers and types for checking the type and extension of a file

use std::task::Poll;

use actix_multipart::Field;
use derive_more::Display;
use futures_core::Stream;
use futures_util::TryStreamExt;

use super::handler_err::HandlerError;

/// Contains a stream of file chunks, retrieved from a request body, in addition
/// to file type data retrieved from the reported MIME type as well as being inferred
/// from magic numbers in the file header and a filename
pub struct CheckedFileStream {
    inference_buf: Vec<u8>,
    buf_has_been_consumed: bool,
    file_type: FileType,
    base_file_name: String,
    field: Field,
}

const INFERENCE_BUF_LEN: usize = 8192;

impl CheckedFileStream {
    /// Extracts metadata from a Field struct into a new CheckedFileStream
    async fn from_field(mut field: Field) -> Result<Self, HandlerError> {
        let mut inference_buf: Vec<u8> = Vec::with_capacity(INFERENCE_BUF_LEN);
        let buf_has_been_consumed: bool = false;
        let base_file_name = field
            .content_disposition()
            .get_filename()
            .map(str::to_owned)
            .unwrap_or("unnamed screenshot".to_string());

        let mut bytes_copied: usize = 0;

        //Fill inference buffer
        while bytes_copied < INFERENCE_BUF_LEN {
            //Get next bytes chunk from field
            if let Some(chunk) = field.try_next().await? {
                //Copy bytes to buffer
                inference_buf.extend_from_slice(chunk.as_ref());

                //Update bytes copied counter
                bytes_copied += chunk.len();
            } else {
                tracing::debug!(
                    file_len = bytes_copied,
                    "File was shorter than target inference len."
                );
                break;
            }
        }

        //Infer file type
        let file_type: FileType;
        let mimed: FileType = field.content_type().into();
        let magic: FileType = infer::get(&inference_buf).into();

        if mimed != magic {
            if magic.category == FileCategory::Unknown && mimed.category != FileCategory::Unknown {
                file_type = mimed;
            } else {
                tracing::warn!(
                    client_mime = ?mimed,
                    inferred = ?magic,
                    "Client-provided MIME type did not match inferred type; using inferred data"
                );
                file_type = magic;
            }
        } else {
            file_type = mimed;
        }

        //Build struct to return
        Ok(Self {
            inference_buf,
            buf_has_been_consumed,
            file_type,
            base_file_name,
            field,
        })
    }
}

impl Stream for CheckedFileStream {
    type Item = Result<bytes::Bytes, HandlerError>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        //If we haven't returned the inference buffer, do so first
        if !self.buf_has_been_consumed && !self.inference_buf.is_empty() {
            Poll::Ready(Some(Ok(bytes::Bytes::from(
                self.as_ref().inference_buf.clone(),
            ))))
        } else {
            match self.as_mut().field.try_poll_next_unpin(cx) {
                Poll::Ready(Some(res)) => {
                    Poll::Ready(Some(res.map_err(|err| HandlerError::FieldReadError {
                        field_name: self.field.name().to_string(),
                        cause: err.to_string(),
                    })))
                }
                Poll::Ready(None) => Poll::Ready(None),
                Poll::Pending => Poll::Pending,
            }
        }
    }
}

#[derive(PartialEq, Eq, Debug)]
pub struct FileType {
    category: FileCategory,
    mime_type: String,
    file_extension: String,
}

impl From<Option<infer::Type>> for FileType {
    fn from(value: Option<infer::Type>) -> Self {
        if let Some(t) = value {
            let category = match t.matcher_type() {
                infer::MatcherType::Image => FileCategory::Image,
                infer::MatcherType::Text => FileCategory::Text,
                infer::MatcherType::Video => FileCategory::Video,
                _ => FileCategory::Other,
            };
            let mime_type = t.mime_type().to_string();
            let file_extension = t.extension().to_string();

            FileType {
                category,
                mime_type,
                file_extension,
            }
        } else {
            FileType {
                category: FileCategory::Unknown,
                mime_type: "application/octet-stream".to_string(),
                file_extension: "".to_string(),
            }
        }
    }
}

impl From<Option<&mime::Mime>> for FileType {
    fn from(value: Option<&mime::Mime>) -> Self {
        if let Some(t) = value {
            let category = match t.type_() {
                mime::IMAGE => FileCategory::Image,
                mime::VIDEO => FileCategory::Video,
                mime::TEXT => FileCategory::Text,
                _ => FileCategory::Other,
            };
            let mime_type = t.essence_str().to_string();
            let file_extension = t.subtype().to_string();

            FileType {
                category,
                mime_type,
                file_extension,
            }
        } else {
            FileType {
                category: FileCategory::Unknown,
                mime_type: "application/octet-stream".to_string(),
                file_extension: "".to_string(),
            }
        }
    }
}

#[derive(PartialEq, Eq, Debug, Display)]
pub enum FileCategory {
    Image,
    Video,
    Text,
    Other,
    Unknown,
}
