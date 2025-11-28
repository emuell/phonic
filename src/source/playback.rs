// -------------------------------------------------------------------------------------------------

use std::sync::Arc;

use crossbeam_queue::ArrayQueue;

use crate::Error;

use super::{
    super::generator::GeneratorPlaybackMessage, amplified::AmplifiedSourceMessage,
    file::FilePlaybackMessage, panned::PannedSourceMessage, synth::SynthPlaybackMessage,
};

// -------------------------------------------------------------------------------------------------

/// Queues to control real-time audio playback properties of File or Synth sources.
#[derive(Clone)]
pub(crate) enum PlaybackMessageQueue {
    File {
        playback: Arc<ArrayQueue<FilePlaybackMessage>>,
        volume: Arc<ArrayQueue<AmplifiedSourceMessage>>,
        panning: Arc<ArrayQueue<PannedSourceMessage>>,
    },
    Synth {
        playback: Arc<ArrayQueue<SynthPlaybackMessage>>,
        volume: Arc<ArrayQueue<AmplifiedSourceMessage>>,
        panning: Arc<ArrayQueue<PannedSourceMessage>>,
    },
    Generator {
        playback: Arc<ArrayQueue<GeneratorPlaybackMessage>>,
        volume: Arc<ArrayQueue<AmplifiedSourceMessage>>,
        panning: Arc<ArrayQueue<PannedSourceMessage>>,
    },
}

impl PlaybackMessageQueue {
    pub fn send_stop(&self) -> Result<(), Error> {
        match self {
            PlaybackMessageQueue::File { playback, .. } => playback
                .push(FilePlaybackMessage::Stop)
                .map_err(|_msg| Error::SendError("File playback queue is full".to_string())),
            PlaybackMessageQueue::Synth { playback, .. } => playback
                .push(SynthPlaybackMessage::Stop)
                .map_err(|_msg| Error::SendError("Synth playback queue is full".to_string())),
            PlaybackMessageQueue::Generator { playback, .. } => playback
                .push(GeneratorPlaybackMessage::Stop)
                .map_err(|_msg| Error::SendError("Generator playback queue is full".to_string())),
        }
    }

    pub fn volume(&self) -> &Arc<ArrayQueue<AmplifiedSourceMessage>> {
        match self {
            PlaybackMessageQueue::File { volume, .. } => volume,
            PlaybackMessageQueue::Synth { volume, .. } => volume,
            PlaybackMessageQueue::Generator { volume, .. } => volume,
        }
    }

    pub fn panning(&self) -> &Arc<ArrayQueue<PannedSourceMessage>> {
        match self {
            PlaybackMessageQueue::File { panning, .. } => panning,
            PlaybackMessageQueue::Synth { panning, .. } => panning,
            PlaybackMessageQueue::Generator { panning, .. } => panning,
        }
    }
}
