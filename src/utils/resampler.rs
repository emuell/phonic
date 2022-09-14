use crate::error::Error;

// -------------------------------------------------------------------------------------------------

#[derive(Copy, Clone)]
#[allow(dead_code)]
pub enum ResamplingQuality {
    SincBestQuality = libsamplerate_sys::SRC_SINC_BEST_QUALITY as isize,
    SincMediumQuality = libsamplerate_sys::SRC_SINC_MEDIUM_QUALITY as isize,
    SincFastest = libsamplerate_sys::SRC_SINC_FASTEST as isize,
    ZeroOrderHold = libsamplerate_sys::SRC_ZERO_ORDER_HOLD as isize,
    Linear = libsamplerate_sys::SRC_LINEAR as isize,
}

// -------------------------------------------------------------------------------------------------

#[derive(Copy, Clone)]
pub struct ResamplingSpec {
    pub input_rate: u32,
    pub output_rate: u32,
    pub channels: usize,
}

impl ResamplingSpec {
    pub fn output_size(&self, input_size: usize) -> usize {
        (self.output_rate as f64 / self.input_rate as f64 * input_size as f64) as usize
    }

    #[allow(dead_code)]
    pub fn input_size(&self, output_size: usize) -> usize {
        (self.input_rate as f64 / self.output_rate as f64 * output_size as f64) as usize
    }

    pub fn ratio(&self) -> f64 {
        self.output_rate as f64 / self.input_rate as f64
    }
}

// -------------------------------------------------------------------------------------------------

pub struct AudioResampler {
    pub spec: ResamplingSpec,
    state: *mut libsamplerate_sys::SRC_STATE,
}

impl AudioResampler {
    pub fn new(quality: ResamplingQuality, spec: ResamplingSpec) -> Result<Self, Error> {
        let mut error_int = 0i32;
        let state = unsafe {
            libsamplerate_sys::src_new(
                quality as i32,
                spec.channels as i32,
                &mut error_int as *mut i32,
            )
        };
        if error_int != 0 {
            Err(Error::ResamplingError(error_int))
        } else {
            Ok(Self { state, spec })
        }
    }

    pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<(usize, usize), Error> {
        // NB: when input is empty, libsamplerate only flushes pending output
        let mut src = libsamplerate_sys::SRC_DATA {
            data_in: if input.is_empty() {
                std::ptr::null()
            } else {
                input.as_ptr()
            },
            data_out: output.as_mut_ptr(),
            input_frames: (input.len() / self.spec.channels) as i32,
            output_frames: (output.len() / self.spec.channels) as i32,
            src_ratio: self.spec.ratio(),
            end_of_input: 0, // TODO: Use this.
            input_frames_used: 0,
            output_frames_gen: 0,
        };
        let error_int = unsafe { libsamplerate_sys::src_process(self.state, &mut src as *mut _) };
        if error_int != 0 {
            Err(Error::ResamplingError(error_int))
        } else {
            let input_len = src.input_frames_used as usize * self.spec.channels;
            let output_len = src.output_frames_gen as usize * self.spec.channels;
            Ok((input_len, output_len))
        }
    }
}

impl Drop for AudioResampler {
    fn drop(&mut self) {
        unsafe { libsamplerate_sys::src_delete(self.state) };
    }
}

unsafe impl Send for AudioResampler {}
