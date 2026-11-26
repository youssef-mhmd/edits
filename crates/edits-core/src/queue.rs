//! Priority queue with removal by ticket.

use std::collections::{BTreeMap, HashMap};

/// Higher runs first. Jobs with equal priority run newest first, because the
/// newest request is the one closest to what the user is looking at now.
pub type Priority = u32;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Key {
    // Sorted ascending, so store the inverse: the first entry runs next.
    inv_priority: u32,
    inv_seq: u64,
}

impl Key {
    fn new(priority: Priority, seq: u64) -> Self {
        Self { inv_priority: u32::MAX - priority, inv_seq: u64::MAX - seq }
    }
}

pub(crate) struct Queue<T> {
    items: BTreeMap<Key, (u64, T)>,
    keys: HashMap<u64, Key>,
    seq: u64,
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self { items: BTreeMap::new(), keys: HashMap::new(), seq: 0 }
    }
}

impl<T> Queue<T> {
    /// Adds an item under `ticket`.
    pub fn push(&mut self, ticket: u64, priority: Priority, item: T) {
        self.seq += 1;
        let key = Key::new(priority, self.seq);
        self.items.insert(key, (ticket, item));
        self.keys.insert(ticket, key);
    }

    pub fn pop(&mut self) -> Option<(u64, T)> {
        let (_, (ticket, item)) = self.items.pop_first()?;
        self.keys.remove(&ticket);
        Some((ticket, item))
    }

    pub fn remove(&mut self, ticket: u64) -> Option<T> {
        let key = self.keys.remove(&ticket)?;
        self.items.remove(&key).map(|(_, item)| item)
    }

    /// Changes the priority of a queued item. Returns false if it is not queued.
    pub fn set_priority(&mut self, ticket: u64, priority: Priority) -> bool {
        let Some(item) = self.remove(ticket) else { return false };
        self.push(ticket, priority, item);
        true
    }

    /// Removes and returns every item for which `pred` is true.
    pub fn drain_where(&mut self, mut pred: impl FnMut(&T) -> bool) -> Vec<(u64, T)> {
        let doomed: Vec<Key> = self.items.iter().filter(|(_, (_, item))| pred(item)).map(|(k, _)| *k).collect();
        doomed
            .into_iter()
            .filter_map(|k| self.items.remove(&k))
            .inspect(|(ticket, _)| {
                self.keys.remove(ticket);
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn higher_priority_first_then_newest() {
        let mut q = Queue::default();
        q.push(1, 5, "a");
        q.push(2, 9, "b");
        q.push(3, 5, "c");
        assert_eq!(q.pop().map(|x| x.1), Some("b"));
        assert_eq!(q.pop().map(|x| x.1), Some("c"));
        assert_eq!(q.pop().map(|x| x.1), Some("a"));
        assert!(q.pop().is_none());
    }

    #[test]
    fn remove_and_reprioritise() {
        let mut q = Queue::default();
        q.push(1, 1, "a");
        q.push(2, 2, "b");
        q.push(3, 3, "c");
        assert_eq!(q.remove(3), Some("c"));
        assert!(q.set_priority(1, 10));
        assert_eq!(q.pop().map(|x| x.1), Some("a"));
        assert_eq!(q.len(), 1);
        assert_eq!(q.remove(42), None);
    }

    #[test]
    fn drain_where_removes_matches() {
        let mut q = Queue::default();
        for i in 0..10u64 {
            q.push(i, 0, i);
        }
        let gone = q.drain_where(|v| v % 2 == 0);
        assert_eq!(gone.len(), 5);
        assert_eq!(q.len(), 5);
        assert!(q.remove(4).is_none());
        assert_eq!(q.remove(5), Some(5));
    }
}
