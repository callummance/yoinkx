//! Configuration management

use std::path::PathBuf;

use anyhow::Result;
use config::Environment;
use dotenvy::dotenv;
use serde_derive::Deserialize;

/// Struct containing configuration for both the image uploader and the
/// imagehost
#[derive(Default, Debug, Deserialize, Clone)]
pub struct Config {
    /// The target location for images to be stored (if enabled)
    pub target_dir: Option<String>,
    /// Enable imagehost functionality
    pub enable_imagehost: bool,
    /// Enable using regex to attempt to guess the name of the program
    /// that was screenshotted, and store images in a corresponding subdirectory
    pub enable_subdirectories: bool,
    /// Override the regex used to extract the program name
    pub subdirectory_regex: String,
    /// The IP and port(s) (in <IP>:[<PORT>] format) that should be listened on.
    pub bind: Vec<String>,
    /// Maximum allowable size for uploaded images in bytes
    pub max_image_size: u64,
}

static ENV_PREFIX: &str = "YOINKX";
static DEFAULT_SUBDIR_REGEX: &str = r"(?P<subdir>.*)_[\d\w]{10}.[\w]+";

impl Config {
    /// Load the configuration from dotenv file, env vars or a config file.
    pub fn load(file_path: Option<String>) -> Result<Self> {
        //Load env
        if let Err(e) = dotenv() {
            //print warning that env file was not found
            println!("{}", e);
        }

        let mut config_builder = config::Config::builder()
            .set_default("target_dir", None::<Option<String>>)?
            .set_default("enable_imagehost", false)?
            .set_default("enable_subdirectories", false)?
            .set_default("subdirectory_regex", DEFAULT_SUBDIR_REGEX)?
            .set_default("bind", vec![String::from("localhost:1256")])?
            .set_default("max_image_size", 100_000_000)?
            .add_source(
                Environment::with_prefix(ENV_PREFIX)
                    .try_parsing(true)
                    .list_separator(",")
                    .prefix_separator("_")
                    .with_list_parse_key("bind"),
            );

        if let Some(f) = file_path {
            config_builder = config_builder.add_source(config::File::with_name(&f).required(true));
        }

        let config = config_builder.build()?;

        let mut res: Self = config.try_deserialize()?;
        res.check_options();

        Ok(res)
    }

    fn check_options(&mut self) {
        //Canonicalize tgt_dir for comparisons later
        self.target_dir.as_mut().map(|tgt_dir_str| {
            let path = PathBuf::from(tgt_dir_str.as_str());
            let canonicalized = path
                .canonicalize()
                .expect("Invalid path provided for image storage");
            canonicalized
                .to_str()
                .expect("Image path contained invalid unicode")
                .to_owned()
        });
        //Only allow imagehost if tgt_dir is set
        if self.enable_imagehost && self.target_dir.is_none() {
            println!("Cannot enable imagehost unless target dir is set");
            self.enable_imagehost = false;
        }
    }
}
