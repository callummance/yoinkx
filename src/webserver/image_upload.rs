//! Handlers for uploading screenshots from ShareX to the server.

use anyhow::Result;
use std::{
    fs::File,
    io::{BufReader, Read},
    path::PathBuf,
};
use tokio::sync::OnceCell;
use tracing::instrument;

use actix_multipart::{
    form::{tempfile::TempFile, MultipartForm},
    MultipartError,
};
use actix_web::{web::Data, HttpRequest};
use anyhow::anyhow;
use image::DynamicImage;

use super::{handler_err::HandlerError, OpenHandles};
use crate::conf::Config;

use futures_util::TryStreamExt as _;

#[derive(Debug, MultipartForm)]
#[doc(hidden)]
pub struct ImageUploadForm {
    #[multipart(rename = "img")]
    img_file: TempFile,
}

/// Wrapper for `TempFile` which allows for saving files in custom directories (helpful for
/// bypassing cross-filesystem persist errors), as well as checking that files at least look
/// like an image before accepting the whole thing.
pub struct TempImageFile(File);

impl<'t> actix_multipart::form::FieldReader<'t> for TempImageFile {
    type Future = futures_core::future::LocalBoxFuture<'t, Result<Self, MultipartError>>;

    fn read_field(
        req: &'t HttpRequest,
        field: actix_multipart::Field,
        limits: &'t mut actix_multipart::form::Limits,
    ) -> Self::Future {
        Box::pin(async move {
            let config_data = req
                .app_data::<Config>()
                .or_else(|| req.app_data::<Data<Config>>().map(|d| d.as_ref()));
            let file_name = field
                .content_disposition()
                .get_filename()
                .map(str::to_owned)
                .unwrap_or("unnamed_screenshot".to_string());
            let field_name = field.name().to_owned();
            let mut size = 0;

            //Make sure we have configs
            if let Some(config) = config_data {
                if let Some(tgt_file) = choose_filename(config, &file_name).await {
                    //If we have a local save dir configured, choose a filename manually
                    todo!()
                } else {
                    //Otherwise, just create a tempfile
                    todo!()
                }
            } else {
                Err(MultipartError::Field {
                    field_name,
                    source: HandlerError::InternalError(anyhow!(
                        "Failed to retrieve config in upload handler"
                    ))
                    .into(),
                })
            }
        })
    }
}

async fn write_to_file(
    multipart_limits: &mut actix_multipart::form::Limits,
    max_size: u64,
    field: &mut actix_multipart::Field,
    tgt_file: &mut tokio::fs::File,
) -> Result<(), MultipartError> {
    let mut size: usize = 0;
    let mut checked_is_image: bool = false;
    let mut infer_buf: Vec<u8> = vec![0; 8192];

    while let Some(chunk) = field.try_next().await? {
        multipart_limits.try_consume_limits(chunk.len(), false)?;
        size += chunk.len();
        //If we have enough data to attempt an infer, do so
        if size >= 8192 {}
    }

    todo!()
}

static SUBDIR_CAPTURE_NAME: &str = "subdir";

#[instrument(skip(handles))]
/// Handler for image upload functionality.
pub async fn upload(
    handles: Data<OpenHandles>,
    config: Data<Config>,
    req: HttpRequest,
    MultipartForm(form): MultipartForm<ImageUploadForm>,
) -> Result<String, HandlerError> {
    let f = form.img_file;

    check_is_image(&f, &config).await?;
    //Generate file path and save to disk if set
    let mut saved_filename: Option<String> = None;
    let img_reader = if let Some(filename) = f.file_name {
        if let Some(tgt_path) = choose_filename(&config, &filename).await {
            match f.file.persist(&tgt_path) {
                Ok(f) => {
                    saved_filename = Some(format!("{}", tgt_path.display()));
                    image::io::Reader::new(BufReader::new(f))
                }
                Err(e) => {
                    tracing::warn!(
                        error = "Failed to write image to disk",
                        cause = format!("{}", e.error),
                        target_path = &tgt_path.to_string_lossy().into_owned()
                    );
                    //Try to reopen the original tempfile instead for copying to the clipboard
                    image::io::Reader::new(BufReader::new(e.file.into_file()))
                }
            }
        } else {
            //We haven't saved a file to the disk for whatever reason so just use the original tempfile
            image::io::Reader::new(BufReader::new(f.file.into_file()))
        }
    } else {
        image::io::Reader::new(BufReader::new(f.file.into_file()))
    };

    //Copy image to clipboard
    insert_file_to_clipboard(img_reader, handles, &saved_filename).await?;

    //Return file location or some default value
    match saved_filename {
        Some(loc) => Ok(loc),
        None => Ok("clipboard only".to_string()),
    }
}

#[instrument(skip(handles, img_reader))]
async fn insert_file_to_clipboard(
    img_reader: image::io::Reader<BufReader<File>>,
    handles: Data<OpenHandles>,
    saved_filename: &Option<String>,
) -> Result<(), HandlerError> {
    //Put image into clipboard
    let img = load_image_from_file(img_reader).await?;
    match handles.clip_image(img).await {
        Ok(()) => Ok(()),
        Err(e) => {
            tracing::error!("Failed to place image into clipboard due to error {}", e);
            Err(e.into())
        }
    }
}

async fn load_image_from_file(reader: image::io::Reader<BufReader<File>>) -> Result<DynamicImage> {
    tokio::task::spawn_blocking(move || -> Result<DynamicImage> {
        Ok(reader.with_guessed_format()?.decode()?)
    })
    .await?
}

async fn check_is_image(file: &TempFile, conf: &Config) -> Result<(), HandlerError> {
    let conf = conf.clone();
    //Open a new copy of the file
    let mut f: File = file
        .file
        .reopen()
        .map_err(HandlerError::FailedToLoadImage)?;

    tokio::task::spawn_blocking(move || -> Result<(), HandlerError> {
        //Check file size
        let size: u64 = f.metadata().map_err(HandlerError::FailedToLoadImage)?.len();
        if size > conf.max_image_size {
            Err(HandlerError::FileTooLarge(size, conf.max_image_size))?
        }

        //Check file is an image
        let limit = std::cmp::min(size as usize, 8192);
        let mut bytes: Vec<u8> = vec![0; limit];
        f.read_exact(&mut bytes)
            .map_err(HandlerError::FailedToLoadImage)?;

        if infer::is_image(&bytes) {
            Ok(())
        } else {
            Err(HandlerError::FileWasNotAnImage(infer::get(&bytes)))
        }
    })
    .await?
}

/// Choose a path a file should be saved to based on its original filename. Returns `None` if
/// a local file path is not configured or if errors occurred when trying to ensure the directory
/// exists.
#[instrument]
async fn choose_filename(config: &Config, filename: &str) -> Option<PathBuf> {
    if let Some(tgt_dir) = &config.target_dir {
        let mut tgt_dir: PathBuf = PathBuf::from(tgt_dir);
        //Add subdirectory to path if we get a regex match
        if let Some(subdir_name) = choose_subdirectory(config, filename).await {
            tgt_dir.push(subdir_name);
        }
        //Make sure directory exists, create it if it doesn't
        if let Err(e) = std::fs::create_dir_all(&tgt_dir) {
            tracing::error!(error = %e, directory = ?tgt_dir, "Failed to create nonexistant directory ");
            return None;
        }
        let tgt_filename: PathBuf = PathBuf::from(filename);
        //If the file already exists, try appending incrementing suffixes until we find one that doesn't already exist
        let mut filename_suffix: u32 = 0;
        let mut tgt_file = tgt_dir.join(&tgt_filename);
        while tgt_file.exists() {
            let mut suffixed_filename_stem =
                tgt_filename.file_stem().unwrap_or_default().to_os_string();
            suffixed_filename_stem.push(format!("_{}", filename_suffix));
            filename_suffix += 1;
            let mut try_filename = PathBuf::from(suffixed_filename_stem);
            try_filename.set_extension(tgt_filename.extension().unwrap_or_default());
            tgt_file = tgt_dir.join(try_filename);
        }
        Some(tgt_file)
    } else {
        None
    }
}

static SUBDIR_REGEX: OnceCell<regex::Regex> = OnceCell::const_new();
async fn subdir_regex(config: &Config) -> Result<&regex::Regex> {
    SUBDIR_REGEX
        .get_or_try_init(|| async { regex::Regex::new(&config.subdirectory_regex) })
        .await
        .map_err(anyhow::Error::from)
}

async fn choose_subdirectory(config: &Config, filename: &str) -> Option<String> {
    let regex = subdir_regex(config)
        .await
        .map_err(|e| {
            tracing::error!(%e, "Failed to compile subdirectory calculation regex");
            e
        })
        .ok();

    regex
        .and_then(|regex| regex.captures(filename))
        .and_then(|captures| captures.name(SUBDIR_CAPTURE_NAME))
        .map(|val| val.as_str().to_owned())
}
