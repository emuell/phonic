use std::time::Duration;

// -------------------------------------------------------------------------------------------------

/// A unique ID for a newly created File or Synth Sources
pub type PlaybackId = usize;

// -------------------------------------------------------------------------------------------------

/// Events send back from File or Synth sources to the user
pub enum PlaybackStatusEvent {
    Position {
        /// Unique id to resolve played back sources.
        id: PlaybackId,
        /// The file path for file based sources, else a name to somewhat identify the source.
        path: String,
        /// Source's actual playback position in wallclock-time.
        position: Duration,
    },
    Stopped {
        /// Unique id to resolve played back sources
        id: PlaybackId,
        /// the file path for file based sources, else a name to somewhat identify the source
        path: String,
        /// true when the source finished playing (e.g. reaching EOF), false when manually stopped
        exhausted: bool,
    },
}
