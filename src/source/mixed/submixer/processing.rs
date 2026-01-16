use std::{
    ops::DerefMut,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
};

use basedrop::Owned;

use crate::{MixerId, SourceTime};

use super::super::{MixedSource, SubMixerProcessor};
use super::sync::SlaveThreadSync;

// -------------------------------------------------------------------------------------------------

/// A bin for collecting mixers assigned to a worker during bin-packing.
struct BatchingWorkerBin {
    total_weight: usize,
    mixer_indices: Vec<usize>,
}

impl BatchingWorkerBin {
    fn new(capacity_hint: usize) -> Self {
        Self {
            total_weight: 0,
            mixer_indices: Vec::with_capacity(capacity_hint),
        }
    }

    fn clear(&mut self) {
        self.total_weight = 0;
        self.mixer_indices.clear();
    }
}

// -------------------------------------------------------------------------------------------------

/// Pre-allocated scratch buffers for batching, reused each frame to avoid allocations.
struct BatchingScratchBuffers {
    /// Mixers with their weights, for sorting
    weighted_mixers: Vec<(usize, usize)>,
    /// One bin per worker thread
    worker_bins: Vec<BatchingWorkerBin>,
}

impl BatchingScratchBuffers {
    fn new(thread_count: usize, max_mixers: usize) -> Self {
        // Each bin should hold roughly max_mixers/thread_count, plus headroom for imbalance
        let bin_capacity = (max_mixers / thread_count).max(1) + 8;

        Self {
            weighted_mixers: Vec::with_capacity(max_mixers),
            worker_bins: (0..thread_count)
                .map(|_| BatchingWorkerBin::new(bin_capacity))
                .collect(),
        }
    }

    fn clear(&mut self) {
        self.weighted_mixers.clear();
        for bin in &mut self.worker_bins {
            bin.clear();
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Create weighted batches of mixers for parallel processing using pre-allocated scratch buffers.
///
/// This implements a greedy bin-packing algorithm that distributes mixers across
/// workers to balance total processing weight per worker. Mixers with higher
/// estimated cost are assigned first to minimize load imbalance.
///
/// IMPORTANT: This function reuses scratch buffers to avoid allocations in the audio thread.
fn create_weighted_batches(
    mixers: &mut [(MixerId, Owned<SubMixerProcessor>)],
    scratch: &mut BatchingScratchBuffers,
) {
    scratch.clear();

    if mixers.is_empty() {
        return;
    }

    // Calculate weight for each mixer and store in pre-allocated buffer
    scratch.weighted_mixers.extend(
        mixers
            .iter()
            .enumerate()
            .map(|(idx, (_, mixer))| (idx, mixer.estimate_processing_weight())),
    );

    // Sort by weight descending (largest first for better packing)
    scratch.weighted_mixers.sort_by(|a, b| b.1.cmp(&a.1));

    // Greedy assignment: assign each mixer to worker with lowest current weight
    for &(mixer_idx, weight) in &scratch.weighted_mixers {
        // Find worker bin with minimum total weight
        let min_bin = scratch
            .worker_bins
            .iter_mut()
            .min_by_key(|bin| bin.total_weight)
            .unwrap();

        min_bin.total_weight += weight;
        min_bin.mixer_indices.push(mixer_idx);
    }
}

// -------------------------------------------------------------------------------------------------

/// Processing parameters for a batch task.
#[derive(Clone, Copy, Default)]
struct WorkerProcessingParams {
    channel_count: usize,
    sample_rate: u32,
    output_len: usize,
    time: SourceTime,
}

/// A batch task containing mixer pointers and metadata for processing.
struct WorkerBatchTask {
    mixer_id: MixerId,
    mixer_ptr: *mut SubMixerProcessor,
}

// SAFETY: BatchTask contains raw pointers but we control their lifetimes
unsafe impl Send for WorkerBatchTask {}

/// Per-worker state that only the owning worker and main thread access.
/// No lock contention between workers since each worker has its own instance.
struct WorkerState {
    /// This worker's batch of tasks
    batch: Mutex<Vec<WorkerBatchTask>>,
    /// Processing parameters
    params: Mutex<WorkerProcessingParams>,
    /// Results from this worker
    results: Mutex<Vec<SubMixerProcessingResult>>,
}

impl WorkerState {
    fn new(capacity_hint: usize) -> Self {
        Self {
            batch: Mutex::new(Vec::with_capacity(capacity_hint)),
            params: Mutex::new(WorkerProcessingParams::default()),
            results: Mutex::new(Vec::with_capacity(capacity_hint)),
        }
    }

    /// Main thread sets the batch and parameters for this worker.
    /// Clears and fills the batch with tasks from the provided iterator.
    fn set_work<I>(&self, tasks: I, params: WorkerProcessingParams)
    where
        I: IntoIterator<Item = WorkerBatchTask>,
    {
        let mut batch = self.batch.lock().unwrap();
        batch.clear();
        batch.extend(tasks);
        *self.params.lock().unwrap() = params;
        self.results.lock().unwrap().clear();
    }

    /// Get worker threads current batch and parameters.
    fn work(&self) -> (&Mutex<Vec<WorkerBatchTask>>, &Mutex<WorkerProcessingParams>) {
        (&self.batch, &self.params)
    }

    /// Main thread takes results from this worker.
    fn take_results(&self, results: &mut Vec<SubMixerProcessingResult>) {
        results.append(&mut *self.results.lock().unwrap())
    }

    /// Worker thread swaps its local results buffer with the stored one.
    fn swap_results(&self, worker_results: &mut Vec<SubMixerProcessingResult>) {
        let mut stored = self.results.lock().unwrap();
        std::mem::swap(&mut *stored, worker_results);
    }
}

// -------------------------------------------------------------------------------------------------

/// Result from processing a mixer.
#[derive(Clone)]
pub(crate) struct SubMixerProcessingResult {
    /// The mixer ID that was processed.
    pub mixer_id: MixerId,
    /// Whether the mixer produced audible output.
    pub is_audible: bool,
}

// -------------------------------------------------------------------------------------------------

/// A real-time safe thread pool for parallel mixer processing.
///
/// This pool pre-spawns worker threads at construction time and promotes them to
/// real-time priority. Workers use condition variable synchronization for explicit
/// sleep/wake cycles, providing lower latency than channel-based approaches.
///
/// Each worker has its own state to eliminate lock contention between workers.
/// Scratch buffers are pre-allocated and reused to avoid allocations in the audio thread.
pub(crate) struct SubMixerThreadPool {
    /// Synchronization primitive for coordinating workers
    sync: Arc<SlaveThreadSync>,
    /// Per-worker state (no contention between workers)
    worker_states: Vec<Arc<WorkerState>>,
    /// Pre-allocated scratch buffers for batching (reused each frame)
    scratch: Mutex<BatchingScratchBuffers>,
    /// Shutdown signal
    shutdown: Arc<AtomicBool>,
    /// Worker thread handles
    workers: Vec<thread::JoinHandle<()>>,
}

impl SubMixerThreadPool {
    const MIN_PARALLEL_MIXERS: usize = 2;
    /// Maximum number of mixers we pre-allocate scratch space for
    const MAX_MIXERS_HINT: usize = 128;

    /// Create a new thread pool with the given configuration.
    ///
    /// This will spawn worker threads immediately and attempt to promote them
    /// to real-time priority. The threads will remain idle until tasks are dispatched.
    pub fn new(thread_count: usize, sample_rate: u32) -> Self {
        let sync = Arc::new(SlaveThreadSync::new(thread_count));
        let shutdown = Arc::new(AtomicBool::new(false));

        // Create per-worker state with capacity hints
        // Each worker can potentially handle max_mixers/thread_count + headroom for load balancing
        let worker_capacity = (Self::MAX_MIXERS_HINT / thread_count).max(1) + 16;
        let worker_states: Vec<_> = (0..thread_count)
            .map(|_| Arc::new(WorkerState::new(worker_capacity)))
            .collect();

        // Pre-allocate scratch buffers for batching
        let scratch = Mutex::new(BatchingScratchBuffers::new(
            thread_count,
            Self::MAX_MIXERS_HINT,
        ));

        let mut workers = Vec::with_capacity(thread_count);

        for (worker_id, worker_state) in worker_states.iter().cloned().enumerate() {
            let sync = Arc::clone(&sync);
            let _shutdown = Arc::clone(&shutdown);
            let results_capacity = worker_capacity; // Capture capacity for worker's results buffer

            let worker = thread::Builder::new()
                .name(format!("phonic-mixer-worker-{}", worker_id))
                .spawn(move || {
                    // Attempt to promote this thread to real-time priority
                    if let Err(err) = audio_thread_priority::promote_current_thread_to_real_time(
                        4096, // buffer size estimate
                        sample_rate,
                    ) {
                        log::warn!(
                            "Failed to promote mixer worker {} to real-time priority: {}",
                            worker_id,
                            err
                        );
                    }

                    // Pre-allocate mix buffer for this worker thread (reused across all tasks)
                    let mut mix_buffer = vec![0.0; MixedSource::MAX_MIX_BUFFER_SAMPLES];

                    // Pre-allocate results buffer for this worker thread (reused each frame)
                    let mut worker_results = Vec::with_capacity(results_capacity);

                    // Worker loop
                    loop {
                        // Wait for processing signal or shutdown
                        if sync.wait_for_processing().is_none() {
                            break; // Shutdown
                        }

                        // Get work from our own worker state
                        let (tasks, params) = worker_state.work();

                        // Lock tasks and params (no contention with other workers)
                        let tasks = tasks.lock().unwrap();
                        let params = params.lock().unwrap();

                        // Process all tasks in this batch
                        worker_results.clear();
                        for task in tasks.iter() {
                            let result = Self::run_task(
                                task.mixer_id,
                                task.mixer_ptr,
                                params.channel_count,
                                params.sample_rate,
                                params.output_len,
                                &params.time,
                                &mut mix_buffer,
                            );
                            worker_results.push(result);
                        }

                        // Move results from worker state to our results list
                        worker_state.swap_results(&mut worker_results);

                        // Signal completion
                        sync.signal_completion();
                    }
                })
                .expect("Failed to spawn mixer worker thread");

            workers.push(worker);
        }

        Self {
            sync,
            worker_states,
            scratch,
            shutdown,
            workers,
        }
    }

    /// Check if parallel processing should be used based on the number of mixers.
    pub(crate) fn should_use_parallel_processing(&self, sub_mixer_count: usize) -> bool {
        sub_mixer_count >= Self::MIN_PARALLEL_MIXERS
    }

    /// Process mixers in parallel using the thread pool.
    ///
    /// This is a synchronous blocking call that:
    /// 1. Creates weighted batches of mixers using pre-allocated scratch buffers
    /// 2. Distributes work to worker states (no lock contention)
    /// 3. Wakes up worker threads
    /// 4. Waits for all workers to complete
    /// 5. Collects results into the provided buffer
    ///
    /// # Safety
    /// The mixers slice must remain valid and unmodified for the duration of this call.
    ///
    /// # Arguments
    /// * `results` - Pre-allocated buffer to fill with results (will be cleared first)
    pub(crate) fn process_mixers_parallel(
        &self,
        mixers: &mut [(MixerId, Owned<SubMixerProcessor>)],
        channel_count: usize,
        sample_rate: u32,
        output_len: usize,
        time: &SourceTime,
        results: &mut Vec<SubMixerProcessingResult>,
    ) {
        // Processing parameters shared by all workers
        let params = WorkerProcessingParams {
            channel_count,
            sample_rate,
            output_len,
            time: *time,
        };

        // Create weighted batches using pre-allocated scratch buffers (no allocation!)
        {
            let mut scratch = self.scratch.lock().unwrap();
            create_weighted_batches(mixers, &mut scratch);

            // Distribute work to each worker's state (no lock contention - we're the only writer)
            for (worker_id, worker_state) in self.worker_states.iter().enumerate() {
                if worker_id < scratch.worker_bins.len() {
                    let bin = &scratch.worker_bins[worker_id];

                    // Build task list from mixer indices (no allocation - iterator directly extends)
                    let tasks = bin.mixer_indices.iter().map(|&idx| {
                        let (mixer_id, mixer) = &mut mixers[idx];
                        WorkerBatchTask {
                            mixer_id: *mixer_id,
                            mixer_ptr: mixer.deref_mut() as *mut SubMixerProcessor,
                        }
                    });

                    worker_state.set_work(tasks, params);
                } else {
                    // No work for this worker
                    worker_state.set_work(std::iter::empty(), params);
                }
            }
        } // scratch lock released here

        // Wake workers and wait for completion
        self.sync.run_slave_threads();

        // Collect results from all workers into the provided buffer (no allocation!)
        results.clear();
        for worker_state in &self.worker_states {
            worker_state.take_results(results);
        }
    }

    /// Run a single mixer task
    fn run_task(
        mixer_id: MixerId,
        mixer_ptr: *mut SubMixerProcessor,
        channel_count: usize,
        sample_rate: u32,
        output_len: usize,
        time: &SourceTime,
        mix_buffer: &mut [f32],
    ) -> SubMixerProcessingResult {
        // SAFETY: mixer_ptr is valid for the duration of processing
        // The main thread waits for all workers before accessing mixers again
        let mixer = unsafe { &mut *mixer_ptr };

        // Temporarily take ownership of mixer's output buffer
        let mut mixer_out_buffer = std::mem::take(&mut mixer.temp_output_buffer);

        let output = &mut mixer_out_buffer[..output_len];
        let mix_buffer = &mut mix_buffer[..output_len];

        // Clear buffers
        output.fill(0.0);
        mix_buffer.fill(0.0);

        // Process the mixer
        let is_audible = mixer.process(output, mix_buffer, channel_count, sample_rate, time);

        // Move back mixer buffer to mixer
        mixer.temp_output_buffer = std::mem::take(&mut mixer_out_buffer);

        // Return result metadata only (no data copy)
        SubMixerProcessingResult {
            mixer_id,
            is_audible,
        }
    }
}

impl Drop for SubMixerThreadPool {
    fn drop(&mut self) {
        // Signal shutdown
        self.shutdown.store(true, Ordering::Relaxed);
        self.sync.shutdown();

        // Wait for all workers to finish
        while let Some(worker) = self.workers.pop() {
            if let Err(e) = worker.join() {
                log::error!("Mixer worker thread panicked: {:?}", e);
            }
        }
    }
}
