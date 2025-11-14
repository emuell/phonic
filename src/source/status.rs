// -------------------------------------------------------------------------------------------------

use std::{any::Any, sync::Arc, time::Duration};

use crate::PlaybackId;

// -------------------------------------------------------------------------------------------------

/// Custom context type for playback status events.
pub type PlaybackStatusContext = Arc<dyn Any + Send + Sync>;

// -------------------------------------------------------------------------------------------------

/// Events send back from File or Synth sources via the player to the user.
pub enum PlaybackStatusEvent {
    Position {
        /// Unique id to resolve played back sources.
        id: PlaybackId,
        /// The file path for file based sources, else a name to somewhat identify the source.
        path: Arc<String>,
        /// Custom, optional context, passed along when starting playback.
        context: Option<PlaybackStatusContext>,
        /// Source's actual playback position in wallclock-time.
        position: Duration,
    },
    Stopped {
        /// Unique id to resolve played back sources
        id: PlaybackId,
        /// the file path for file based sources, else a name to somewhat identify the source
        path: Arc<String>,
        /// Custom, optional context, passed along when starting playback.
        context: Option<PlaybackStatusContext>,
        /// true when the source finished playing (e.g. reaching EOF), false when manually stopped
        exhausted: bool,
    },
}
