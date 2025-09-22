use std::path::PathBuf;

use arg::{parse_args, Args};
use crossbeam_channel::Sender;

use phonic::{outputs::WavOutputDevice, DefaultOutputDevice, Error, PlaybackStatusEvent, Player};

// -------------------------------------------------------------------------------------------------

const DEFAULT_LOG_LEVEL: log::Level = if cfg!(debug_assertions) {
    log::Level::Debug
} else {
    log::Level::Warn
};

// -------------------------------------------------------------------------------------------------

/// Default program arguments for phonic example applications.
#[derive(Args, Debug, Default)]
pub struct Arguments {
    #[arg(short = "o", long = "output")]
    /// Write audio output into the given wav file, instead of using the default audio device.
    pub output_path: Option<PathBuf>,
    #[arg(short = "l", long = "log-level")]
    /// Set logging level to \"debug\", \"info\", \"warn\" or \"error\".
    /// By default \"debug\" in dev builds and \"warn\" in release builds.
    pub log_level: Option<log::Level>,
}

// -------------------------------------------------------------------------------------------------

/// Parse common example arguments and apply the log-level arg to the logger
pub fn parse() -> Arguments {
    // Parse args
    let args = parse_args::<Arguments>();

    // Init logger
    simple_logger::SimpleLogger::new()
        // use default or arg level by default
        .with_level(
            args.log_level
                .unwrap_or(DEFAULT_LOG_LEVEL)
                .to_level_filter(),
        )
        // disable logging in chatty modules
        .with_module_level("symphonia_core", log::LevelFilter::Warn)
        .with_module_level("symphonia_format", log::LevelFilter::Warn)
        .with_module_level("audio_thread_priority", log::LevelFilter::Warn)
        .init()
        .expect("Failed to set logger");
    args
}

// -------------------------------------------------------------------------------------------------

// Create a new player instance using the given argument options.
pub fn new_player<S: Into<Option<Sender<PlaybackStatusEvent>>>>(
    args: &Arguments,
    status_sender: S,
) -> Result<Player, Error> {
    if let Some(output_path) = &args.output_path {
        Ok(Player::new(
            WavOutputDevice::open(output_path)?,
            status_sender,
        ))
    } else {
        Ok(Player::new(DefaultOutputDevice::open()?, status_sender))
    }
}
