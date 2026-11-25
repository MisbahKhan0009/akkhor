use std::collections::HashMap;
use std::hash::Hash;

/// A least-recently-used cache with a byte budget.
///
/// Every cache in the engine (rendered tiles, glyph outlines, decoded
/// chapters) uses one of these, so memory use has a hard ceiling no matter
/// how big the book is. Inserting past the budget evicts the entries that
/// were used longest ago.
pub struct LruCache<K, V> {
    map: HashMap<K, Entry<V>>,
    budget: usize,
    used: usize,
    clock: u64,
}

struct Entry<V> {
    value: V,
    cost: usize,
    last_used: u64,
}

impl<K: Eq + Hash + Clone, V> LruCache<K, V> {
    /// Creates a cache that holds at most `budget` bytes.
    pub fn new(budget: usize) -> Self {
        Self {
            map: HashMap::new(),
            budget,
            used: 0,
            clock: 0,
        }
    }

    pub fn get(&mut self, key: &K) -> Option<&V> {
        self.clock += 1;
        let clock = self.clock;
        self.map.get_mut(key).map(|e| {
            e.last_used = clock;
            &e.value
        })
    }

    /// Inserts `value`, which costs `cost` bytes. A value larger than the
    /// whole budget is not cached at all.
    pub fn insert(&mut self, key: K, value: V, cost: usize) {
        if cost > self.budget {
            return;
        }
        if let Some(old) = self.map.remove(&key) {
            self.used -= old.cost;
        }
        while self.used + cost > self.budget {
            if !self.evict_oldest() {
                break;
            }
        }
        self.clock += 1;
        self.used += cost;
        self.map.insert(
            key,
            Entry {
                value,
                cost,
                last_used: self.clock,
            },
        );
    }

    pub fn get_or_insert_with(&mut self, key: K, make: impl FnOnce() -> (V, usize)) -> &V {
        if !self.map.contains_key(&key) {
            let (value, cost) = make();
            if cost <= self.budget {
                self.insert(key.clone(), value, cost);
            } else {
                // Too big to cache: keep it only until the next insert, which
                // evicts it first.
                self.map.insert(
                    key.clone(),
                    Entry {
                        value,
                        cost: 0,
                        last_used: 0,
                    },
                );
            }
        }
        self.get(&key).expect("just inserted")
    }

    fn evict_oldest(&mut self) -> bool {
        let oldest = self
            .map
            .iter()
            .min_by_key(|(_, e)| e.last_used)
            .map(|(k, _)| k.clone());
        match oldest {
            Some(k) => {
                let e = self.map.remove(&k).expect("present");
                self.used -= e.cost;
                true
            }
            None => false,
        }
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
    fn oversized_values_are_not_cached() {
        let mut c = LruCache::new(10);
        c.insert("big", 0, 11);
        assert!(c.is_empty());
    }
}
