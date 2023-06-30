//! Handlers for imagehost feature, which allows uploaded images to be accessed.
//! All files within the configured screenshot storage directory will be accessible
//! by path.

use std::path::PathBuf;

use actix_files::NamedFile;
use actix_web::{
    web::{self, Data},
    Result,
};
use tracing::instrument;

use crate::conf::Config;

use super::{handler_err::HandlerError, OpenHandles};

#[instrument(skip(_handles))]
/// Handler for /img/<image_path> which returns files from the local filesystem.
pub async fn img(
    _handles: Data<OpenHandles>,
    config: Data<Config>,
    img_loc: web::Path<String>,
) -> Result<actix_files::NamedFile, HandlerError> {
    if let Some(tgt_dir) = &config.target_dir {
        //Work out image file path
        let mut tgt_dir_buf: PathBuf = PathBuf::from(tgt_dir);
        tracing::trace!("Got request for image at {}", img_loc);
        tgt_dir_buf.push(img_loc.to_string());
        tracing::trace!("Returning image at {:?}", tgt_dir_buf);

        //Make sure requested path is a subdirectory of the screenshots dir
        let canonical = tgt_dir_buf
            .canonicalize()
            .map_err(HandlerError::InvalidPath)?;
        if !canonical.starts_with(tgt_dir) {
            tracing::warn!(
                "Got request for path ({}) outside configured directory: {}",
                img_loc.to_string(),
                canonical.display()
            );
            Err(HandlerError::FilePathNotAllowed(img_loc.to_string()))
        } else if canonical.exists() {
            //File exists, so try to open it
            tracing::info!("Returning image {}", canonical.display());
            NamedFile::open(canonical).map_err(HandlerError::InvalidPath)
        } else {
            //File doesn't exist
            tracing::info!("Image not found at {}", canonical.display());
            Err(HandlerError::ImageDoesNotExist(img_loc.to_string()))
        }
    } else {
        Err(HandlerError::ImageHostingDisabled())
    }
}
