use std::collections::HashMap;
use std::time::Instant;
use parking_lot::Mutex;
use serde_json::Value;

pub struct TokenBucket {
    capacity: f64,
    refill_per_second: f64,
    tokens: f64,
    last: Instant,
}

impl TokenBucket {
    pub fn new(capacity: f64, refill_per_second: f64) -> Self {
        TokenBucket {
            capacity,
            refill_per_second,
            tokens: capacity,
            last: Instant::now(),
        }
    }

    pub fn acquire(&mut self, cost: f64) -> bool {
        if cost <= 0.0 {
            return true;
        }
        self.refill();
        if self.tokens >= cost {
            self.tokens -= cost;
            true
        } else {
            false
        }
    }

    pub fn reset(&mut self) {
        self.tokens = self.capacity;
        self.last = Instant::now();
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last).as_secs_f64();
        if elapsed > 0.0 {
            self.tokens = (self.tokens + elapsed * self.refill_per_second).min(self.capacity);
            self.last = now;
        }
    }
}

pub struct RequestLimiter {
    capacity: i64,
    refill_per_second: f64,
    max_clients: usize,
    buckets: Mutex<HashMap<String, (TokenBucket, u64)>>,
    access_counter: std::sync::atomic::AtomicU64,
    created_at: Mutex<HashMap<String, Instant>>,
}

impl RequestLimiter {
    pub fn new(capacity: i64, refill_per_second: f64, max_clients: usize) -> Self {
        RequestLimiter {
            capacity,
            refill_per_second,
            max_clients: max_clients.max(1),
            buckets: Mutex::new(HashMap::new()),
            access_counter: std::sync::atomic::AtomicU64::new(0),
            created_at: Mutex::new(HashMap::new()),
        }
    }

    pub fn check(&self, client_key: &str) -> bool {
        if client_key.is_empty() {
            return false;
        }

        let mut buckets = self.buckets.lock();
        let counter = self.access_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let (bucket, cnt) = buckets.entry(client_key.to_string()).or_insert_with(|| {
            let mut created = self.created_at.lock();
            created.insert(client_key.to_string(), Instant::now());
            (TokenBucket::new(self.capacity as f64, self.refill_per_second), 0)
        });

        *cnt = counter;
        let allowed = bucket.acquire(1.0);

        while buckets.len() > self.max_clients {
            let to_remove = buckets.iter()
                .min_by_key(|(_, (_, c))| *c)
                .map(|(k, _)| k.clone());
            if let Some(k) = to_remove {
                buckets.remove(&k);
                let mut created = self.created_at.lock();
                created.remove(&k);
            } else {
                break;
            }
        }

        allowed
    }

    pub fn stats(&self) -> Value {
        let buckets = self.buckets.lock();
        let created = self.created_at.lock();
        let oldest = created.values().min().map(|inst| inst.elapsed().as_secs_f64());
        serde_json::json!({
            "active_buckets": buckets.len(),
            "oldest_created_ago_s": oldest,
            "capacity": self.capacity,
            "refill_per_second": self.refill_per_second,
            "max_clients": self.max_clients,
        })
    }
}
