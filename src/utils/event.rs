use std::collections::VecDeque;

// -------------------------------------------------------------------------------------------------

/// A sample time tagged event.
pub(crate) trait Event {
    fn sample_time(&self) -> u64;
}

// -------------------------------------------------------------------------------------------------

/// Manage processing of sample time tagged events in e.g. [`Source`]s.
///
/// Note: When adding events via `insert_event`, events will be sorted ascending by time, which
/// is necessary for the ordered event processing. If events are added in other ways, ensure
/// that the sorting doesn't get broken!
pub(crate) trait EventProcessor {
    /// The sample time tagged event.
    type Event: Event;

    /// Determine in how many sample times the next event is due.
    fn time_until_next_event(&self, current_time: u64) -> usize {
        self.events()
            .front()
            .map_or(usize::MAX, |e| (e.sample_time() - current_time) as usize)
    }

    /// Add a new event for processing while keeping event list sorted by ascending sample time.
    fn insert_event(&mut self, event: Self::Event) {
        let events = self.events_mut();
        let sample_time = event.sample_time();
        let insert_pos = events
            .make_contiguous()
            .partition_point(|e| e.sample_time() < sample_time);
        events.insert(insert_pos, event);
    }

    /// Process all pending events that are due up to the given time.
    fn process_events(&mut self, current_time: u64) {
        while self
            .events()
            .front()
            .is_some_and(|e| e.sample_time() <= current_time)
        {
            let event = self.events_mut().pop_front().unwrap();
            self.process_event(event);
        }
    }

    /// Access to the event deque.
    fn events(&self) -> &VecDeque<Self::Event>;
    /// Mutable access to the event deque.
    fn events_mut(&mut self) -> &mut VecDeque<Self::Event>;

    /// Process a single due event.
    fn process_event(&mut self, event: Self::Event);
}
