//! Handlers for uploading screenshots from ShareX to the server.

use anyhow::Result;
use std::{
    io::{BufReader, SeekFrom},
    path::{Path, PathBuf},
};
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::OnceCell,
};
use tracing::{debug, instrument};

use actix_multipart::{form::MultipartForm, MultipartError};
use actix_web::{web::Data, HttpRequest};
use anyhow::anyhow;
use image::DynamicImage;

use super::{
    checked_file_stream::{CheckedFileStream, FileCategory},
    handler_err::HandlerError,
    OpenHandles,
};
use crate::conf::Config;

use futures_util::TryStreamExt as _;

// ---------------------------------------------------------- //
// ------------ Multipart form decoding functions ----------- //
// ---------------------------------------------------------- //

#[derive(Debug, MultipartForm)]
#[doc(hidden)]
pub struct ImageUploadForm {
    #[multipart(rename = "img")]
    img_file: MaybeTempImageFile,
}

/// Wrapper for `TempFile` which allows for saving files in custom directories (helpful for
/// bypassing cross-filesystem persist errors), as well as checking that files at least look
/// like an image before accepting the whole thing.
#[derive(Debug)]
pub struct MaybeTempImageFile {
    pub f: File,
    pub path: Option<PathBuf>,
}

impl<'t> actix_multipart::form::FieldReader<'t> for MaybeTempImageFile {
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
            let field_name = field.name().to_owned();
            let mut file_stream = CheckedFileStream::from_field(field)
                .await
                .map_err(HandlerError::to_multipart_err(&field_name))?;

            //Make sure we have configs
            if let Some(config) = config_data {
                //Check file is of a valid type
                if !check_is_allowed_type(&file_stream, config).await {
                    return Err(HandlerError::FileWasNotAnImage(file_stream.file_type))
                        .map_err(HandlerError::to_multipart_err(&field_name));
                }

                //Get file name with extension added if not already present
                let file_name = file_stream.get_filename_with_extension();
                if let Some(tgt_file) = choose_filename(config, file_name).await {
                    //If we have a local save dir configured, choose a filename manually
                    let f = write_to_path(limits, &mut file_stream, &tgt_file)
                        .await
                        .map_err(HandlerError::to_multipart_err(&field_name))?;
                    Ok(MaybeTempImageFile {
                        f,
                        path: Some(tgt_file),
                    })
                } else {
                    //Otherwise, just create a tempfile
                    let f = write_to_new_tempfile(limits, &mut file_stream)
                        .await
                        .map_err(HandlerError::to_multipart_err(&field_name))?;
                    Ok(MaybeTempImageFile { f, path: None })
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

async fn write_to_new_tempfile(
    multipart_limits: &mut actix_multipart::form::Limits,
    field: &mut CheckedFileStream,
) -> Result<File, HandlerError> {
    //Create tempfile
    let mut f = tokio::task::spawn_blocking(move || -> Result<File, std::io::Error> {
        let f = tempfile::tempfile()?;

        //Convert to async file handle
        Ok(File::from_std(f))
    })
    .await
    .map_err(HandlerError::TokioRuntimeError)?
    .map_err(HandlerError::FailedToWriteImage)?;

    //Write data
    write_to_file(multipart_limits, field, &mut f).await?;
    Ok(f)
}

async fn write_to_path(
    multipart_limits: &mut actix_multipart::form::Limits,
    field: &mut CheckedFileStream,
    path: impl AsRef<Path>,
) -> Result<File, HandlerError> {
    let mut f: File = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .read(true)
        .open(&path)
        .await
        .map_err(HandlerError::FailedToWriteImage)?;
    debug!("Writing data to path: {}", path.as_ref().display());
    write_to_file(multipart_limits, field, &mut f).await?;
    //Seek back to start of file once written
    f.seek(SeekFrom::Start(0))
        .await
        .map_err(HandlerError::FailedToWriteImage)?;
    //Debug
    debug_file_handle(&mut f).await.unwrap();
    Ok(f)
}

async fn write_to_file(
    multipart_limits: &mut actix_multipart::form::Limits,
    field: &mut CheckedFileStream,
    tgt_file: &mut tokio::fs::File,
) -> Result<(), HandlerError> {
    let mut written_bytes: usize = 0;
    while let Some(chunk) = field.try_next().await? {
        multipart_limits.try_consume_limits(chunk.len(), false)?;
        //Write chunk
        tgt_file
            .write_all(&chunk)
            .await
            .map_err(HandlerError::FailedToWriteImage)?;
        written_bytes += chunk.len();
    }
    debug_file_handle(tgt_file).await;
    debug!("Wrote {} bytes to file system", written_bytes);
    Ok(())
}

async fn check_is_allowed_type(file: &CheckedFileStream, _conf: &Config) -> bool {
    //TODO: allow changing of allowed types from configuration
    file.file_type.category == FileCategory::Image
}

/// Choose a path a file should be saved to based on its original filename. Returns `None` if
/// a local file path is not configured or if errors occurred when trying to ensure the directory
/// exists.
#[instrument]
async fn choose_filename(config: &Config, base_filename: PathBuf) -> Option<PathBuf> {
    if let Some(tgt_dir) = &config.target_dir {
        let filename = base_filename.to_string_lossy();
        let mut tgt_dir: PathBuf = PathBuf::from(tgt_dir);
        //Add subdirectory to path if we get a regex match
        if let Some(subdir_name) = choose_subdirectory(config, &filename).await {
            tgt_dir.push(subdir_name);
        }
        //Make sure directory exists, create it if it doesn't
        if let Err(e) = std::fs::create_dir_all(&tgt_dir) {
            tracing::error!(error = %e, directory = ?tgt_dir, "Failed to create nonexistant directory ");
            return None;
        }
        //If the file already exists, try appending incrementing suffixes until we find one that doesn't already exist
        let mut filename_suffix: u32 = 0;
        let mut tgt_file = tgt_dir.join(&base_filename);
        while tgt_file.exists() {
            let mut suffixed_filename_stem =
                base_filename.file_stem().unwrap_or_default().to_os_string();
            suffixed_filename_stem.push(format!("_{}", filename_suffix));
            filename_suffix += 1;
            let mut try_filename = PathBuf::from(suffixed_filename_stem);
            try_filename.set_extension(base_filename.extension().unwrap_or_default());
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

// ---------------------------------------------------------- //
// --------------- Handler and util functions --------------- //
// ---------------------------------------------------------- //

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

    //Copy image to clipboard
    insert_file_to_clipboard(f.f, handles).await?;

    //Return file location or some default value
    match f.path {
        Some(loc) => Ok(loc.to_string_lossy().to_string()),
        None => Ok("clipboard only".to_string()),
    }
}

#[instrument(skip(handles, file))]
async fn insert_file_to_clipboard(
    file: File,
    handles: Data<OpenHandles>,
) -> Result<(), HandlerError> {
    //Put image into clipboard
    let img = load_image_from_file(file).await?;
    match handles.clip_image(img).await {
        Ok(()) => Ok(()),
        Err(e) => {
            tracing::error!("Failed to place image into clipboard due to error {}", e);
            Err(e.into())
        }
    }
}

async fn load_image_from_file(mut f: File) -> Result<DynamicImage> {
    //Convert file handle to std::fs::File
    let f: std::fs::File = f.into_std().await;
    let reader: image::io::Reader<BufReader<std::fs::File>> =
        image::io::Reader::new(BufReader::new(f));
    tokio::task::spawn_blocking(move || -> Result<DynamicImage> {
        Ok(reader.with_guessed_format()?.decode()?)
    })
    .await?
}

#[instrument]
async fn debug_file_handle(mut f: &mut File) -> Result<()> {
    let mut buf = vec![0; 64];
    tokio::io::AsyncSeekExt::seek(&mut f, SeekFrom::Start(0)).await?;
    f.read_buf(&mut buf).await?;
    tracing::trace!("{:x?}", buf);
    tokio::io::AsyncSeekExt::seek(&mut f, SeekFrom::Start(0)).await?;

    Ok(())
}
