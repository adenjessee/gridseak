// Tiny rust source used by the runner's parity + polyglot tests.
// Real enough to exercise the parser (struct + impl + free function), small
// enough that the test stays under a second on a cold cache.

pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

pub struct Counter {
    count: u32,
}

impl Counter {
    pub fn new() -> Self {
        Counter { count: 0 }
    }

    pub fn increment(&mut self) {
        self.count += 1;
    }

    pub fn value(&self) -> u32 {
        self.count
    }
}

pub fn greet_n_times(name: &str, n: u32) -> Vec<String> {
    let mut out = Vec::new();
    for _ in 0..n {
        out.push(format!("hello {name}"));
    }
    out
}
