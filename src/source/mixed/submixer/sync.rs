use std::sync::{Condvar, Mutex};

// -------------------------------------------------------------------------------------------------

/// Synchronization primitive for coordinating master and slave threads.
///
/// This sync pattern uses condition variables and barriers only: It provides explicit control
/// over thread wake/sleep cycles with lower latency than e.g. channel-based synchronization.
///
/// The pattern works as follows:
/// 1. Main thread calls `run_slave_threads()` - wakes all workers and waits
/// 2. Workers wake up, process their tasks
/// 3. Workers call `signal_completion()` when done
/// 4. Main thread unblocks when all workers have completed
/// 5. Workers call `wait_for_processing()` to sleep until next wake
pub(crate) struct SlaveThreadSync {
    /// Shared state protected by mutex
    state: Mutex<SyncState>,
    /// Condition variable for signaling workers to wake up
    worker_condvar: Condvar,
    /// Condition variable for signaling main thread that workers are done
    main_condvar: Condvar,
    /// Total number of worker threads
    thread_count: usize,
}

#[derive(Debug)]
struct SyncState {
    /// Number of threads currently waiting for work
    waiting_threads: usize,
    /// Number of threads that have completed their work
    completed_threads: usize,
    /// Current processing round number (incremented each time work is dispatched)
    current_round: u64,
    /// Whether workers should shut down
    should_shutdown: bool,
}

impl SlaveThreadSync {
    /// Create a new synchronization primitive for the given number of worker threads.
    pub fn new(thread_count: usize) -> Self {
        Self {
            state: Mutex::new(SyncState {
                waiting_threads: 0,
                completed_threads: 0,
                current_round: 0,
                should_shutdown: false,
            }),
            worker_condvar: Condvar::new(),
            main_condvar: Condvar::new(),
            thread_count,
        }
    }

    // --- Main thread interface ---

    /// Wake up all slave threads and wait for them to complete processing.
    ///
    /// This is the main entry point for the audio thread. It:
    /// 1. Signals all workers to start processing
    /// 2. Blocks until all workers have completed
    /// 3. Resets state for next processing round
    ///
    /// # Panics
    /// Panics if the mutex is poisoned.
    pub fn run_slave_threads(&self) {
        let mut state = self.state.lock().unwrap();

        // Reset completion counter for this processing round
        state.completed_threads = 0;

        // Increment round number to signal new work
        state.current_round += 1;

        // Wake up all waiting workers
        self.worker_condvar.notify_all();

        // Wait for all workers to complete
        while state.completed_threads < self.thread_count {
            state = self.main_condvar.wait(state).unwrap();
        }
    }

    /// Signal shutdown to all worker threads.
    ///
    /// This wakes up all sleeping workers and tells them to exit.
    pub fn shutdown(&self) {
        let mut state = self.state.lock().unwrap();
        state.should_shutdown = true;
        self.worker_condvar.notify_all();
    }

    // --- Worker thread interface ---

    /// Worker thread waits for the next processing round.
    ///
    /// This puts the worker to sleep until the main thread calls `run_slave_threads()`.
    /// Returns the new round number if the worker should process, or None if it should shut down.
    ///
    /// # Panics
    /// Panics if the mutex is poisoned.
    pub fn wait_for_processing(&self) -> Option<u64> {
        let mut state = self.state.lock().unwrap();

        // Increment waiting counter
        state.waiting_threads += 1;

        // Remember the current round when we started waiting
        let start_round = state.current_round;

        // Wait for a new round or shutdown
        while state.current_round == start_round && !state.should_shutdown {
            state = self.worker_condvar.wait(state).unwrap();
        }

        // Decrement waiting counter
        state.waiting_threads -= 1;

        if !state.should_shutdown {
            Some(state.current_round)
        } else {
            None
        }
    }

    /// Worker thread signals that it has completed processing.
    ///
    /// This increments the completion counter and wakes the main thread if all workers are done.
    ///
    /// # Panics
    /// Panics if the mutex is poisoned.
    pub fn signal_completion(&self) {
        let mut state = self.state.lock().unwrap();

        state.completed_threads += 1;

        // If all workers have completed, wake up the main thread
        if state.completed_threads == self.thread_count {
            self.main_condvar.notify_one();
        }
    }
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, thread, time::Duration};

    #[test]
    fn test_basic_sync() {
        let sync = Arc::new(SlaveThreadSync::new(2));
        let completed = Arc::new(Mutex::new(0));

        // Spawn two worker threads
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let sync = Arc::clone(&sync);
                let completed = Arc::clone(&completed);
                thread::spawn(move || {
                    // Wait for processing signal
                    if sync.wait_for_processing().is_some() {
                        // Do some work
                        thread::sleep(Duration::from_millis(10));
                        *completed.lock().unwrap() += 1;
                        // Signal completion
                        sync.signal_completion();
                    }
                })
            })
            .collect();

        // Give workers time to start waiting
        thread::sleep(Duration::from_millis(50));

        // Run a processing round
        sync.run_slave_threads();

        // Verify all workers completed
        assert_eq!(*completed.lock().unwrap(), 2);

        // Shutdown and join
        sync.shutdown();
        for worker in workers {
            worker.join().unwrap();
        }
    }

    #[test]
    fn test_multiple_rounds() {
        let sync = Arc::new(SlaveThreadSync::new(3));
        let counter = Arc::new(Mutex::new(0));

        // Spawn three worker threads
        let workers: Vec<_> = (0..3)
            .map(|_| {
                let sync = Arc::clone(&sync);
                let counter = Arc::clone(&counter);
                thread::spawn(move || {
                    while let Some(_round) = sync.wait_for_processing() {
                        *counter.lock().unwrap() += 1;
                        sync.signal_completion();
                    }
                })
            })
            .collect();

        // Give workers time to start
        thread::sleep(Duration::from_millis(50));

        // Run multiple processing rounds
        for _ in 0..5 {
            sync.run_slave_threads();
        }

        // Verify all rounds completed
        assert_eq!(*counter.lock().unwrap(), 15); // 3 workers * 5 rounds

        // Shutdown and join
        sync.shutdown();
        for worker in workers {
            worker.join().unwrap();
        }
    }
}
