//! Shared buffer for per-sample modulation in FunDSP audio graphs.

use std::sync::{Arc, RwLock};

use crate::utils::buffer::{clear_buffer, TempBuffer};

use fundsp::{
    audionode::AudioNode,
    buffer::{BufferMut, BufferRef},
    combinator::An,
    typenum::{U0, U1},
    Frame,
};

// -------------------------------------------------------------------------------------------------

/// A shared buffer for per-sample modulation values.
///
/// This buffer uses a rw lock around a raw f32 buffer for lock-free thread safety, allowing the
/// modulation matrix to write values and FunDSP VarBuffer nodes to read them.
#[derive(Clone)]
pub struct SharedBuffer {
    buffer: Arc<RwLock<TempBuffer>>,
}

impl SharedBuffer {
    /// Create a new shared buffer initialized with zeros.
    pub fn new(capacity: usize) -> Self {
        let buffer = Arc::new(RwLock::new(TempBuffer::new(capacity)));
        Self { buffer }
    }

    /// Copy written modulation values.
    pub fn read(&self, values: &mut [f32]) {
        let buffer = self
            .buffer
            .read()
            .expect("Failed to access shared fundsp buffer for reading");
        assert!(
            values.len() <= buffer.len(),
            "Tried to read {} prefilled values, but only got {} values",
            values.len(),
            buffer.len()
        );
        buffer.copy_to(values);
    }

    /// Write modulation values for the upcoming block (up to MODULATION_PROCESSOR_BLOCK_SIZE).
    pub fn write(&mut self, values: &[f32]) {
        let mut buffer = self
            .buffer
            .write()
            .expect("Failed to access shared fundsp buffer for writing");
        assert!(
            values.len() <= buffer.capacity(),
            "Cannot write more than {} values",
            buffer.capacity()
        );
        buffer.set_range(0, values.len());
        buffer.copy_from(values);
    }

    /// Clear modulation values.
    pub fn clear(&mut self) {
        let mut buffer = self
            .buffer
            .write()
            .expect("Failed to access shared fundsp buffer for writing");
        buffer.reset_range();
        clear_buffer(buffer.get_mut());
    }
}

// -------------------------------------------------------------------------------------------------

/// FunDSP AudioNode that outputs per-sample values from a SharedBuffer.
///
/// This node has no inputs and one output, reading modulation values
/// from the shared buffer on each sample.
#[derive(Clone)]
pub struct VarBuffer {
    buffer: SharedBuffer,
}

impl VarBuffer {
    /// Create a new VarBuffer node.
    pub fn new(buffer: &SharedBuffer) -> Self {
        Self {
            buffer: buffer.clone(),
        }
    }
}

impl AudioNode for VarBuffer {
    const ID: u64 = 0x766172627566; // "varbuf" in hex
    type Inputs = U0;
    type Outputs = U1;

    #[inline]
    fn tick(&mut self, _input: &Frame<f32, Self::Inputs>) -> Frame<f32, Self::Outputs> {
        // tick() doesn't know sample index, so it should never be used
        unreachable!("Can't use tick in VarBuffer")
    }

    fn process(&mut self, size: usize, _input: &BufferRef, output: &mut BufferMut) {
        // Copy values from the shared buffer - buffers must have been filled previously
        let output = output.channel_f32_mut(0);
        self.buffer.read(&mut output[0..size]);
    }
}

// -------------------------------------------------------------------------------------------------

/// Helper function to create a VarBuffer node wrapped in a fundsp `An`.
///
/// # Arguments
/// * `buffer` - SharedBuffer to read modulation values from
///
/// # Example
/// ```rust
/// use phonic::utils::fundsp::{var_buffer, SharedBuffer};
///
/// let mod_buffer = SharedBuffer::new();
/// let modulation_node = var_buffer(&mod_buffer);
/// ```
pub fn var_buffer(buffer: &SharedBuffer) -> An<VarBuffer> {
    An(VarBuffer::new(buffer))
}
