//! Per-topic-partition runtime: write serialization, offsets, retention, and tail cache.

use std::{
    collections::VecDeque,
    sync::{Mutex, MutexGuard},
};

use kalamdb_tables::TopicMessage;

const TAIL_MAX_MESSAGES: usize = 1024;
const TAIL_MAX_BYTES: usize = 1024 * 1024;

/// Contiguous in-memory suffix of recently published messages. Filled only after
/// a durable write. `None` from [`PartitionTail::fetch`] means read the store.
#[derive(Debug, Default)]
pub(crate) struct PartitionTail {
    start:    u64,
    end:      u64,
    bytes:    usize,
    messages: VecDeque<TopicMessage>,
}

impl PartitionTail {
    fn append(&mut self, message: TopicMessage) {
        if !self.messages.is_empty() && message.offset != self.end {
            self.clear();
        }

        if self.messages.is_empty() {
            self.start = message.offset;
            self.end = message.offset.saturating_add(1);
            self.bytes = message.payload.len();
            self.messages.push_back(message);
            self.trim_capacity();
            return;
        }

        self.bytes = self.bytes.saturating_add(message.payload.len());
        self.end = message.offset.saturating_add(1);
        self.messages.push_back(message);
        self.trim_capacity();
    }

    fn append_messages(&mut self, messages: impl IntoIterator<Item = TopicMessage>) {
        for message in messages {
            self.append(message);
        }
    }

    fn trim_below(&mut self, earliest: u64) {
        while let Some(front) = self.messages.front() {
            if front.offset >= earliest {
                break;
            }
            let dropped = self.messages.pop_front().expect("front existed");
            self.bytes = self.bytes.saturating_sub(dropped.payload.len());
            self.start = dropped.offset.saturating_add(1);
        }
        if self.messages.is_empty() {
            self.start = earliest;
            self.end = earliest;
            self.bytes = 0;
        }
    }

    /// `Some(empty)` at `offset == end` is a watermark hit. `None` is a miss.
    fn fetch(&self, offset: u64, limit: usize) -> Option<Vec<TopicMessage>> {
        if self.messages.is_empty() {
            return None;
        }
        if offset == self.end {
            return Some(Vec::new());
        }
        if offset < self.start || offset > self.end {
            return None;
        }
        if limit == 0 {
            return Some(Vec::new());
        }

        let skip = usize::try_from(offset.saturating_sub(self.start)).unwrap_or(usize::MAX);
        Some(self.messages.iter().skip(skip).take(limit).cloned().collect())
    }

    fn trim_capacity(&mut self) {
        while !self.messages.is_empty()
            && (self.messages.len() > TAIL_MAX_MESSAGES || self.bytes > TAIL_MAX_BYTES)
        {
            let dropped = self.messages.pop_front().expect("non-empty");
            self.bytes = self.bytes.saturating_sub(dropped.payload.len());
            self.start = dropped.offset.saturating_add(1);
        }
        if self.messages.is_empty() {
            self.end = self.start;
            self.bytes = 0;
        }
    }

    fn clear(&mut self) {
        self.messages.clear();
        self.start = 0;
        self.end = 0;
        self.bytes = 0;
    }
}

#[derive(Debug, Default)]
pub(crate) struct PartitionWriteState {
    next_offset:    Option<u64>,
    retained_bytes: Option<u64>,
    log_start:      u64,
}

impl PartitionWriteState {
    pub(crate) fn allocate(&mut self, count: u64) -> u64 {
        let start = self.next_offset.unwrap_or(0);
        self.next_offset = Some(start.saturating_add(count));
        start
    }

    pub(crate) fn peek_next(&self) -> Option<u64> {
        self.next_offset
    }

    pub(crate) fn seed(&mut self, next_offset: u64) {
        self.next_offset = Some(next_offset);
    }

    pub(crate) fn add_retained_bytes(&mut self, bytes: u64) {
        if bytes == 0 {
            return;
        }
        self.retained_bytes = Some(self.retained_bytes.unwrap_or(0).saturating_add(bytes));
    }

    pub(crate) fn subtract_retained_bytes(&mut self, bytes: u64) {
        if bytes == 0 {
            return;
        }
        match &mut self.retained_bytes {
            Some(current) => *current = current.saturating_sub(bytes),
            None => self.retained_bytes = Some(0),
        }
    }

    pub(crate) fn set_retained_bytes(&mut self, bytes: u64) {
        self.retained_bytes = Some(bytes);
    }

    pub(crate) fn retained_bytes(&self) -> Option<u64> {
        self.retained_bytes
    }

    pub(crate) fn log_start(&self) -> u64 {
        self.log_start
    }

    pub(crate) fn set_log_start(&mut self, offset: u64) {
        if offset > self.log_start {
            self.log_start = offset;
        }
    }
}

/// Write mutex serializes offset assignment + durable write. Tail mutex is
/// separate so a consumer at the watermark does not wait on RocksDB.
pub(crate) struct PartitionRuntime {
    write: Mutex<PartitionWriteState>,
    tail:  Mutex<PartitionTail>,
}

impl PartitionRuntime {
    pub(crate) fn new() -> Self {
        Self {
            write: Mutex::new(PartitionWriteState::default()),
            tail:  Mutex::new(PartitionTail::default()),
        }
    }

    pub(crate) fn lock_write(&self) -> MutexGuard<'_, PartitionWriteState> {
        self.write.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_tail(&self) -> MutexGuard<'_, PartitionTail> {
        self.tail.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn append_to_tail(&self, message: TopicMessage) {
        self.lock_tail().append(message);
    }

    pub(crate) fn append_messages_to_tail(&self, messages: Vec<TopicMessage>) {
        self.lock_tail().append_messages(messages);
    }

    pub(crate) fn fetch_from_tail(&self, offset: u64, limit: usize) -> Option<Vec<TopicMessage>> {
        self.lock_tail().fetch(offset, limit)
    }

    pub(crate) fn trim_tail_below(&self, earliest: u64) {
        self.lock_tail().trim_below(earliest);
    }
}

#[cfg(test)]
mod tests {
    use kalamdb_commons::models::{TopicId, TopicOp};

    use super::*;

    fn msg(offset: u64, payload: &[u8]) -> TopicMessage {
        TopicMessage::new(TopicId::new("t"), 0, offset, payload.to_vec(), None, 1, TopicOp::Insert)
    }

    #[test]
    fn allocate_is_monotonic_and_seedable() {
        let mut state = PartitionWriteState::default();
        assert_eq!(state.peek_next(), None);
        assert_eq!(state.allocate(1), 0);
        assert_eq!(state.allocate(3), 1);
        assert_eq!(state.peek_next(), Some(4));

        state.seed(100);
        assert_eq!(state.allocate(1), 100);
    }

    #[test]
    fn allocate_under_mutex_is_unique() {
        use std::{sync::Arc, thread};

        let runtime = Arc::new(PartitionRuntime::new());
        let mut handles = Vec::new();

        for _ in 0..10 {
            let runtime = Arc::clone(&runtime);
            handles.push(thread::spawn(move || {
                let mut offsets = Vec::with_capacity(100);
                for _ in 0..100 {
                    offsets.push(runtime.lock_write().allocate(1));
                }
                offsets
            }));
        }

        let mut all_offsets: Vec<u64> = Vec::new();
        for handle in handles {
            all_offsets.extend(handle.join().expect("partition allocate thread"));
        }

        all_offsets.sort_unstable();
        all_offsets.dedup();
        assert_eq!(all_offsets.len(), 1000);
        assert_eq!(*all_offsets.first().unwrap(), 0);
        assert_eq!(*all_offsets.last().unwrap(), 999);
    }

    #[test]
    fn fetch_misses_before_cached_start() {
        let mut tail = PartitionTail::default();
        tail.append(msg(10, b"a"));
        tail.append(msg(11, b"b"));
        assert!(tail.fetch(9, 10).is_none());
        let hit = tail.fetch(10, 10).unwrap();
        assert_eq!(hit.len(), 2);
        assert_eq!(hit[0].offset, 10);
        assert_eq!(hit[1].offset, 11);
    }

    #[test]
    fn fetch_at_end_is_empty_hit() {
        let mut tail = PartitionTail::default();
        tail.append(msg(0, b"a"));
        assert!(tail.fetch(1, 10).unwrap().is_empty());
    }

    #[test]
    fn gap_resets_to_new_suffix() {
        let mut tail = PartitionTail::default();
        tail.append(msg(0, b"a"));
        tail.append(msg(5, b"b"));
        assert!(tail.fetch(0, 10).is_none());
        let hit = tail.fetch(5, 10).unwrap();
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].offset, 5);
    }

    #[test]
    fn trim_below_drops_retained_prefix() {
        let mut tail = PartitionTail::default();
        for offset in 0..5 {
            tail.append(msg(offset, b"x"));
        }
        tail.trim_below(3);
        assert!(tail.fetch(2, 10).is_none());
        let hit = tail.fetch(3, 10).unwrap();
        assert_eq!(hit.iter().map(|m| m.offset).collect::<Vec<_>>(), vec![3, 4]);
    }

    #[test]
    fn capacity_trims_oldest() {
        let mut tail = PartitionTail::default();
        for offset in 0..=TAIL_MAX_MESSAGES as u64 {
            tail.append(msg(offset, b"x"));
        }
        assert_eq!(tail.messages.len(), TAIL_MAX_MESSAGES);
        assert_eq!(tail.start, 1);
        assert_eq!(tail.end, TAIL_MAX_MESSAGES as u64 + 1);
        assert!(tail.fetch(0, 1).is_none());
        assert_eq!(tail.fetch(1, 1).unwrap()[0].offset, 1);
    }
}
