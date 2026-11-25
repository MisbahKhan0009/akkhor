use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;

/// A least-recently-used cache with a byte budget.
///
/// Every cache in the engine (rendered tiles, glyph outlines, decoded
/// images) uses one of these, so memory use has a hard ceiling no matter how
/// big the book is. Inserting past the budget evicts the entries that were
/// used longest ago. Lookups, inserts and evictions are O(log n).
pub struct LruCache<K, V> {
    map: HashMap<K, Entry<V>>,
    /// Last-use tick → key, oldest first.
    order: BTreeMap<u64, K>,
    budget: usize,
    used: usize,
    clock: u64,
    /// Holds a value too big to cache, just long enough to hand it back.
    scratch: Option<V>,
}

struct Entry<V> {
    value: V,
    cost: usize,
    tick: u64,
}

impl<K: Eq + Hash + Clone, V> LruCache<K, V> {
    /// Creates a cache that holds at most `budget` bytes.
    pub fn new(budget: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: BTreeMap::new(),
            budget,
            used: 0,
            clock: 0,
            scratch: None,
        }
    }

    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        let now = self.tick();
        let e = self.map.get_mut(key)?;
        self.order.remove(&e.tick);
        e.tick = now;
        self.order.insert(now, key.clone());
        Some(&e.value)
    }

    /// Inserts `value`, which costs `cost` bytes. A value larger than the
    /// whole budget is not cached at all.
    pub fn insert(&mut self, key: K, value: V, cost: usize) {
        if cost > self.budget {
            return;
        }
        self.remove(&key);
        while self.used + cost > self.budget {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            if let Some(e) = self.map.remove(&oldest) {
                self.used -= e.cost;
            }
        }
        let tick = self.tick();
        self.used += cost;
        self.order.insert(tick, key.clone());
        self.map.insert(key, Entry { value, cost, tick });
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let e = self.map.remove(key)?;
        self.order.remove(&e.tick);
        self.used -= e.cost;
        Some(e.value)
    }

    /// Returns the cached value for `key`, computing and caching it on a
    /// miss. `make` returns the value and its cost in bytes. A value too big
    /// to cache is still returned, but not kept.
    pub fn get_or_insert_with(&mut self, key: K, make: impl FnOnce() -> (V, usize)) -> &V {
        if !self.map.contains_key(&key) {
            let (value, cost) = make();
            if cost > self.budget {
                return self.scratch.insert(value);
            }
            self.insert(key.clone(), value, cost);
        }
        self.get(&key).expect("just inserted")
    }

    pub fn used_bytes(&self) -> usize {
        self.used
    }

    pub fn budget(&self) -> usize {
        self.budget
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
        self.scratch = None;
        self.used = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_least_recently_used() {
        let mut c = LruCache::new(10);
        c.insert("a", 1, 4);
        c.insert("b", 2, 4);
        c.get(&"a");
        c.insert("c", 3, 4); // evicts b, the oldest
        assert!(c.get(&"a").is_some());
        assert!(c.get(&"b").is_none());
        assert!(c.get(&"c").is_some());
        assert!(c.used_bytes() <= 10);
    }

    #[test]
    fn oversized_values_are_returned_but_not_kept() {
        let mut c = LruCache::new(10);
        c.insert("big", 0, 11);
        assert!(c.is_empty());
        assert_eq!(*c.get_or_insert_with("huge", || (7, 100)), 7);
        assert!(c.get(&"huge").is_none(), "not kept");
        assert_eq!(c.used_bytes(), 0);
    }

    #[test]
    fn many_entries_stay_within_budget() {
        let mut c = LruCache::new(1000);
        for i in 0..100_000u32 {
            c.insert(i, i, 10);
            if i % 7 == 0 {
                c.get(&(i / 2));
            }
        }
        assert!(c.used_bytes() <= 1000);
        assert_eq!(c.len(), 100);
        assert_eq!(c.map.len(), c.order.len());
    }
}
