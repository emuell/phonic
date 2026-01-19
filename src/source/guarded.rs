use std::{
    any::Any,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{Arc, Mutex},
};

use super::{Source, SourceTime};

// -------------------------------------------------------------------------------------------------

/// A callback function to handle panics in GuardedSource
pub type PanicHandler = Box<dyn Fn(Box<dyn Any + Send>) + Send + 'static>;

// -------------------------------------------------------------------------------------------------

/// A wrapper source that catches panics from an inner source and reports them via a callback.
///
/// Should only be used for the main player's source.
/// After the wrapped source panicked it is no longer getting called.
pub struct GuardedSource<InputSource: Source + 'static> {
    source: InputSource,
    source_name: &'static str,
    handler: Arc<Mutex<Option<PanicHandler>>>,
    panicked: bool,
}

impl<InputSource: Source + 'static> GuardedSource<InputSource> {
    pub fn new(
        source: InputSource,
        source_name: &'static str,
        handler: Arc<Mutex<Option<PanicHandler>>>,
    ) -> Self {
        let panicked = false;
        Self {
            source,
            source_name,
            handler,
            panicked,
        }
    }
}

impl<InputSource: Source + 'static> Source for GuardedSource<InputSource> {
    fn channel_count(&self) -> usize {
        self.source.channel_count()
    }

    fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }

    fn is_exhausted(&self) -> bool {
        self.source.is_exhausted() || self.panicked
    }

    fn weight(&self) -> usize {
        if self.panicked {
            0
        } else {
            self.source.weight()
        }
    }

    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        if self.panicked {
            return 0;
        }
        match catch_unwind(AssertUnwindSafe(|| self.source.write(output, time))) {
            Ok(written) => written,
            Err(payload) => {
                self.panicked = true;
                if let Some(handler) = self.handler.lock().unwrap().as_ref() {
                    handler(payload);
                } else {
                    log::error!("Ouch. {} source panicked: {:?}", self.source_name, payload);
                }
                0
            }
        }
    }
}
