use std::{
    any::Any,
    ops::DerefMut,
    ptr::NonNull,
    sync::{Arc, Mutex, MutexGuard},
    thread,
};

use basedrop::Owned;
use crossbeam_channel::{Receiver, Sender};

use crate::{MixerId, SourceTime};

use super::super::{MixedSource, SubMixerProcessor};

// -------------------------------------------------------------------------------------------------

/// A bin for collecting mixers assigned to a worker during bin-packing.
#[derive(Debug, Clone)]
struct WorkerTaskBin {
    pub total_weight: usize,
    pub mixer_indices: Vec<usize>,
}

impl WorkerTaskBin {
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

/// Weight and mixer source index for a single mixer within a worker task batch.
#[derive(Debug, Copy, Clone)]
struct WorkerTaskMixerWeight {
    index: usize,
    weight: usize,
}

// -------------------------------------------------------------------------------------------------

/// Creates weighted batches of mixer tasks for parallel processing in the thread pool,
/// using pre-allocated scratch buffers.
#[derive(Debug, Clone)]
struct WorkerTaskBatcher {
    /// Mixer index and weight for bin sorting
    mixers: Vec<WorkerTaskMixerWeight>,
    /// One bin per worker thread
    bins: Vec<WorkerTaskBin>,
}

impl WorkerTaskBatcher {
    pub fn new(thread_count: usize, max_expected_mixers: usize) -> Self {
        // Each bin should hold roughly max_mixers/thread_count, plus headroom for imbalance
        let bin_capacity = (max_expected_mixers / thread_count).max(1) + 8;

        Self {
            mixers: Vec::with_capacity(max_expected_mixers),
            bins: (0..thread_count)
                .map(|_| WorkerTaskBin::new(bin_capacity))
                .collect(),
        }
    }

    #[inline(always)]
    pub fn bins(&self) -> &Vec<WorkerTaskBin> {
        &self.bins
    }

    pub fn clear(&mut self) {
        self.mixers.clear();
        for bin in &mut self.bins {
            bin.clear();
        }
    }

    /// Create weighted batches of mixers for parallel processing using pre-allocated scratch buffers.
    ///
    /// This implements a greedy bin-packing algorithm that distributes mixers across
    /// workers to balance total processing weight per worker. Mixers with higher
    /// estimated cost are assigned first to minimize load imbalance.
    ///
    /// It reuses scratch buffers to avoid allocations in the audio thread.
    pub fn update(&mut self, mixers: &mut [(MixerId, Owned<SubMixerProcessor>)]) {
        self.clear();

        if mixers.is_empty() {
            return;
        }

        // Calculate weight for each mixer and store in pre-allocated buffer
        self.mixers
            .extend(mixers.iter().enumerate().map(|(index, (_, mixer))| {
                let weight = mixer.weight();
                WorkerTaskMixerWeight { index, weight }
            }));

        // Sort by weight descending (largest first for better packing)
        self.mixers.sort_by(|a, b| b.weight.cmp(&a.weight));

        // Greedy assignment: assign each mixer to worker with lowest current weight
        for mixer_weight in &self.mixers {
            // Find worker bin with minimum total weight
            let min_bin = self
                .bins
                .iter_mut()
                .min_by_key(|bin| bin.total_weight)
                .unwrap();

            min_bin.total_weight += mixer_weight.weight;
            min_bin.mixer_indices.push(mixer_weight.index);
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Command sent from the main thread to worker threads.
#[derive(Debug, Copy, Clone)]
enum WorkerCommand {
    Process,
    Shutdown,
}

/// Command sent from the worker thread to main thread to signal completion.
type WorkerCompletion = Result<(), Box<dyn Any + Send + 'static>>;

// -------------------------------------------------------------------------------------------------

/// A batch processing task, containing mixer pointers and metadata for processing submixers
/// in a thread pool's worker thread.
#[derive(Debug)]
struct WorkerProcessingTask {
    mixers: Vec<(MixerId, NonNull<SubMixerProcessor>)>,
    channel_count: usize,
    sample_rate: u32,
    output_len: usize,
    time: SourceTime,
}

impl WorkerProcessingTask {
    fn new(capacity_hint: usize) -> Self {
        Self {
            mixers: Vec::with_capacity(capacity_hint),
            channel_count: 2,
            sample_rate: 44100,
            output_len: 0,
            time: SourceTime::default(),
        }
    }
}

// SAFETY: WorkerProcessingTask contains NonNull pointers but we control their lifetimes.
// The main thread creates these pointers from &mut references and waits for all workers
// to complete before accessing the referenced data again.
unsafe impl Send for WorkerProcessingTask {}

// -------------------------------------------------------------------------------------------------

/// Per-worker state that only the owning worker and main thread access.
/// No lock contention between workers since each worker has its own instance.
#[derive(Debug)]
struct WorkerState {
    /// Current work package
    task: Mutex<WorkerProcessingTask>,
    /// Results from the last processed task
    results: Mutex<Vec<SubMixerProcessingResult>>,
    /// Channel to send commands to this worker
    work_sender: Sender<WorkerCommand>,
    /// Channel to receive completion from this worker
    completion_receiver: Receiver<WorkerCompletion>,
}

impl WorkerState {
    fn new(capacity_hint: usize) -> (Self, Receiver<WorkerCommand>, Sender<WorkerCompletion>) {
        let (work_sender, work_receiver) = crossbeam_channel::bounded(0);
        let (completion_sender, completion_receiver) = crossbeam_channel::bounded(0);

        let state = Self {
            task: Mutex::new(WorkerProcessingTask::new(capacity_hint)),
            results: Mutex::new(Vec::with_capacity(capacity_hint)),
            work_sender,
            completion_receiver,
        };

        // Return state + handles for worker thread
        (state, work_receiver, completion_sender)
    }

    /// Get worker thread's current work package.
    #[inline(always)]
    fn task(&self) -> MutexGuard<'_, WorkerProcessingTask> {
        self.task.lock().unwrap()
    }

    /// Set batch and parameters for the worker.
    /// Clears and fills the batch with tasks from the provided iterator.
    fn set_task<I>(
        &self,
        mixers: I,
        channel_count: usize,
        sample_rate: u32,
        output_len: usize,
        time: SourceTime,
    ) where
        I: IntoIterator<Item = (MixerId, NonNull<SubMixerProcessor>)>,
    {
        let mut task = self.task.lock().unwrap();
        task.mixers.clear();
        task.mixers.extend(mixers);
        task.channel_count = channel_count;
        task.sample_rate = sample_rate;
        task.output_len = output_len;
        task.time = time;

        let mut results = self.results.lock().unwrap();
        results.clear();
    }

    /// Main thread takes results from this worker.
    fn take_results(&self, results: &mut Vec<SubMixerProcessingResult>) {
        results.append(&mut *self.results.lock().unwrap())
    }

    /// Worker thread swaps its local results buffer with the stored one.
    fn swap_results(&self, worker_results: &mut Vec<SubMixerProcessingResult>) {
        let mut stored_results = self.results.lock().unwrap();
        std::mem::swap(&mut *stored_results, worker_results);
    }
}

// -------------------------------------------------------------------------------------------------

/// Result from processing a mixer.
#[derive(Debug, Copy, Clone)]
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
/// real-time priority. Workers use channel-based synchronization to coordinate
/// with the main thread.
///
/// Each worker has its own state to eliminate lock contention between workers.
/// Scratch buffers are pre-allocated and reused to avoid allocations in the audio thread.
pub(crate) struct SubMixerThreadPool {
    /// Per-worker state (no contention between workers, includes channels)
    worker_states: Vec<Arc<WorkerState>>,
    /// Worker thread handles
    worker_threads: Vec<thread::JoinHandle<()>>,
    /// Pre-allocated scratch buffers for batching (reused each frame)
    task_batcher: WorkerTaskBatcher,
}

impl SubMixerThreadPool {
    /// Maximum number of mixers we pre-allocate scratch space for.
    pub const MAX_MIXERS_HINT: usize = 128;

    /// Create a new thread pool with the given configuration.
    ///
    /// This will spawn worker threads immediately and attempt to promote them
    /// to real-time priority. The threads will remain idle until tasks are dispatched.
    pub fn new(thread_count: usize, sample_rate: u32) -> Self {
        // Each worker can potentially handle max_mixers/thread_count + headroom for load balancing
        let worker_capacity = (Self::MAX_MIXERS_HINT / thread_count).max(1) + 16;

        // Create worker states and spawn worker threads
        let mut worker_states = Vec::with_capacity(thread_count);
        let mut worker_threads = Vec::with_capacity(thread_count);

        for worker_id in 0..thread_count {
            // Create worker state with channels
            let (state, work_receiver, completion_sender) = WorkerState::new(worker_capacity);
            let worker_state = Arc::new(state);
            worker_states.push(Arc::clone(&worker_state));

            let results_capacity = worker_capacity;
            worker_threads.push(
                thread::Builder::new()
                    .name(format!("phonic-mixer-worker-{}", worker_id))
                    .spawn(move || {
                        let error_sender = completion_sender.clone();
                        if let Err(payload) = std::panic::catch_unwind(move || {
                            Self::run_worker_thread(
                                sample_rate,
                                worker_id,
                                worker_state,
                                work_receiver,
                                completion_sender,
                                results_capacity,
                            )
                        }) {
                            log::error!(
                                "Ouch. Worker thread #{worker_id} panicked: {}",
                                panic_message::panic_message(&payload)
                            );
                            error_sender
                                .send(Err(payload))
                                .expect("Failed to send completion error to main thread")
                        }
                    })
                    .expect("Failed to spawn mixer worker thread"),
            );
        }

        // Pre-allocate scratch buffers for batching
        let task_batcher = WorkerTaskBatcher::new(thread_count, Self::MAX_MIXERS_HINT);

        Self {
            worker_states,
            worker_threads,
            task_batcher,
        }
    }

    /// Check if the thread pool should be used based on the number of processed mixers.
    pub fn should_use_concurrent_processing(&self, sub_mixer_count: usize) -> bool {
        self.worker_threads.len() >= 2 && sub_mixer_count >= 2
    }

    /// Process mixers in parallel in the thread pool.
    ///
    /// This is a synchronous blocking call that:
    /// - Creates weighted batches of mixers using pre-allocated scratch buffers
    /// - Distributes work to worker states (no lock contention)
    /// - Wakes up worker threads
    /// - Waits for all workers to complete
    /// - Collects results into the provided buffer
    ///
    /// # Safety
    /// The mixers slice must remain valid and unmodified for the duration of this call.
    ///
    /// # Arguments
    /// * `results` - Pre-allocated buffer to fill with results (will be cleared first)
    pub fn process(
        &mut self,
        mixers: &mut [(MixerId, Owned<SubMixerProcessor>)],
        channel_count: usize,
        sample_rate: u32,
        output_len: usize,
        time: &SourceTime,
        results: &mut Vec<SubMixerProcessingResult>,
    ) {
        debug_assert!(self.should_use_concurrent_processing(mixers.len()));

        // Divide mixers into weighted task batches accross all workers
        self.task_batcher.update(mixers);

        // Clear results
        results.clear();

        // Assign work from batcher and wake all workers which have tasks
        for (worker_id, worker_state) in self.worker_states.iter().enumerate() {
            if let Some(bin) = self.task_batcher.bins().get(worker_id) {
                if !bin.mixer_indices.is_empty() {
                    // Create and apply task to worker
                    let mixers = bin.mixer_indices.iter().map(|&idx| {
                        let (mixer_id, mixer) = &mut mixers[idx];
                        (*mixer_id, NonNull::from(mixer.deref_mut()))
                    });
                    worker_state.set_task(mixers, channel_count, sample_rate, output_len, *time);
                    // Signal worker to start processing
                    worker_state
                        .work_sender
                        .send(WorkerCommand::Process)
                        .expect("Failed to send process command to mixer worker thread");
                }
            }
        }

        // Wait for completions from workers with tasks and collect results
        for (worker_id, worker_state) in self.worker_states.iter().enumerate() {
            if let Some(bin) = self.task_batcher.bins().get(worker_id) {
                if !bin.mixer_indices.is_empty() {
                    // Wait untill processing finished
                    // NB: Crossbeam allocs here thread local variables once which is just fine.
                    let result = Self::permit_alloc(|| {
                        worker_state
                            .completion_receiver
                            .recv()
                            .expect("Failed to receive message from mixer worker thread")
                    });
                    // Handle errors
                    if let Err(payload) = result {
                        // Forward errors from worker thread to the main audio thread
                        panic!(
                            "Audio worker thread #{worker_id} processing failed: {}",
                            panic_message::panic_message(&payload)
                        );
                    } else {
                        // Collect results
                        worker_state.take_results(results);
                    }
                }
            }
        }
    }

    fn assert_no_alloc<T, F: FnOnce() -> T>(func: F) -> T {
        #[cfg(feature = "assert-allocs")]
        return assert_no_alloc::assert_no_alloc::<T, F>(func);

        #[cfg(not(feature = "assert-allocs"))]
        return func();
    }

    #[inline]
    fn permit_alloc<T, F: FnOnce() -> T>(func: F) -> T {
        #[cfg(feature = "assert-allocs")]
        return assert_no_alloc::permit_alloc::<T, F>(func);

        #[cfg(not(feature = "assert-allocs"))]
        return func();
    }

    fn run_worker_thread(
        sample_rate: u32,
        worker_id: usize,
        worker_state: Arc<WorkerState>,
        work_receiver: Receiver<WorkerCommand>,
        completion_sender: Sender<WorkerCompletion>,
        results_capacity: usize,
    ) {
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

        // Worker loop: Wait for processing signal or shutdown
        loop {
            match work_receiver.recv() {
                Ok(WorkerCommand::Process) => {
                    // Clear results
                    worker_results.clear();

                    Self::assert_no_alloc(|| {
                        // Get work from our own worker state (no contention with other workers)
                        let task = worker_state.task();

                        // Process all tasks from this batch
                        for (mixer_id, mixer_ptr) in &task.mixers {
                            let result = Self::run_task(
                                *mixer_id,
                                *mixer_ptr,
                                task.channel_count,
                                task.sample_rate,
                                task.output_len,
                                &task.time,
                                &mut mix_buffer,
                            );
                            worker_results.push(result);
                        }
                    });

                    // Move results from worker state to results list
                    worker_state.swap_results(&mut worker_results);

                    // Signal completion
                    // NB: Crossbeam allocs here thread local variables once which is just fine.
                    if completion_sender.send(WorkerCompletion::Ok(())).is_err() {
                        log::warn!(
                            "Worker thread #{worker_id} unexpectedly got disconnected from main thread."
                        );
                        break;
                    }
                }
                Ok(WorkerCommand::Shutdown) => {
                    log::info!("Worker thread #{worker_id} is shutting down...");
                    break;
                }
                Err(_) => {
                    log::warn!(
                        "Worker thread #{worker_id} unexpectedly got disconnected from main thread."
                    );
                    break;
                }
            }
        }
    }

    /// Run a single mixer task
    fn run_task(
        mixer_id: MixerId,
        mixer_ptr: NonNull<SubMixerProcessor>,
        channel_count: usize,
        sample_rate: u32,
        output_len: usize,
        time: &SourceTime,
        mix_buffer: &mut [f32],
    ) -> SubMixerProcessingResult {
        // SAFETY: mixer_ptr is valid for the duration of processing.
        // The main thread waits for all workers before accessing mixers again.
        // NonNull guarantees the pointer is non-null.
        let mixer = unsafe { &mut *mixer_ptr.as_ptr() };

        // Temporarily take ownership of mixer's output buffer
        let mut output_buffer = std::mem::take(&mut mixer.output_buffer);

        let output = &mut output_buffer[..output_len];
        let mix_buffer = &mut mix_buffer[..output_len];

        // Clear buffers
        output.fill(0.0);

        // Process the mixer
        let is_audible = mixer.process(output, mix_buffer, channel_count, sample_rate, time);

        // Move back mixer buffer to mixer
        mixer.output_buffer = std::mem::take(&mut output_buffer);

        // Return result metadata only (no data copy)
        SubMixerProcessingResult {
            mixer_id,
            is_audible,
        }
    }
}

impl Drop for SubMixerThreadPool {
    fn drop(&mut self) {
        // Signal shutdown to each worker
        for worker_state in &self.worker_states {
            let _ = worker_state.work_sender.send(WorkerCommand::Shutdown);
        }

        // Wait for all workers to finish
        while let Some(worker) = self.worker_threads.pop() {
            if let Err(payload) = worker.join() {
                log::error!(
                    "Mixer worker thread panicked: {}",
                    panic_message::panic_message(&payload)
                );
            }
        }
    }
}
