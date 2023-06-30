#![warn(missing_docs)]

//! A ShareX server which places any recieved images onto the clipboard, as well as optionally
//! saving them to the filesystem.

use clap::Parser;
use yoinkx::{conf, webserver};

#[actix_web::main]
async fn main() {
    //Setup logging
    let subscriber = tracing_subscriber::fmt()
        .compact()
        .with_file(true)
        .with_line_number(true)
        .pretty()
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("Failed to initialize tracing");
    tracing_log::LogTracer::init().expect("Failed to initialize log to tracing forwarder");

    //Read command line arguments
    let args = Args::parse();

    //Load configs
    let config = conf::Config::load(args.conf).expect("Failed to read configuration");
    println!("{:?}", config);

    //Start server
    let _webserver = webserver::start(config).await;
}

#[derive(Parser, Debug)]
#[command(name = "YoinkX")]
#[command(author = "Callum")]
#[command(version = "1.0")]
#[command(about = "ShareX upload server which dumps images to clipboard")]
struct Args {
    #[arg(short = 'f', long = "conf_file", value_hint = clap::ValueHint::FilePath, value_name = "FILE")]
    conf: Option<String>,
}
