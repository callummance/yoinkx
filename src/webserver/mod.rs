//! Initialization and handlers for webserver

mod checked_file_stream;
pub mod handler_err;
pub mod image_upload;
pub mod imagehost;

use std::borrow::Cow;

use actix_web::{
    web::{self, Data},
    App, HttpServer,
};
use image::{DynamicImage, GenericImageView};

use anyhow::Result;
use tokio::sync::Mutex;

/// Struct containing open resource handles, to be passed to all handlers
pub struct OpenHandles {
    clipboard: Mutex<arboard::Clipboard>,
}

impl OpenHandles {
    /// Initialize handles
    pub fn new() -> Result<Self> {
        let clipboard = arboard::Clipboard::new()?;
        let mutex = Mutex::new(clipboard);

        Ok(OpenHandles { clipboard: mutex })
    }

    /// Copy an image to the clipboard.
    pub async fn clip_image(&self, image: DynamicImage) -> Result<()> {
        //Convert image to array of u8s
        let (w, h) = image.dimensions();
        let rbga_buf = image.to_rgba8();
        let pixeldata = rbga_buf.into_raw();
        let imagedata = arboard::ImageData {
            width: u32::try_into(w)?,
            height: u32::try_into(h)?,
            bytes: Cow::from(pixeldata),
        };

        //Unlock clipboard mutex
        let mut clipboard = self.clipboard.lock().await;

        //Put into clipboard
        clipboard.set_image(imagedata)?;

        Ok(())
    }
}

/// Start the webserver
pub async fn start(conf: crate::conf::Config) -> Result<()> {
    //Open clipboard handle
    let clipboard_data = Data::new(OpenHandles::new()?);
    let config_data = Data::new(conf.clone());

    //Start webserver
    let mut server = HttpServer::new(move || {
        let mut app = App::new()
            //Attach state
            .app_data(clipboard_data.clone())
            .app_data(config_data.clone())
            //Add logger middleware
            .wrap(tracing_actix_web::TracingLogger::default())
            //Mount routes
            .service(web::resource("/upload").to(image_upload::upload));
        //Add imagehost route if enabled
        if conf.enable_imagehost {
            app = app.service(web::resource("/img/{path:.*}").to(imagehost::img));
        }
        app
    });
    //Bind to all configured interfaces
    for bind_addr in &conf.bind {
        server = server.bind(bind_addr)?;
    }

    server.run().await?;

    Ok(())
}
