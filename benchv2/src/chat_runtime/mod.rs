pub mod ai;
pub mod chat_realtime_bench;
pub mod common;
pub mod shared;

use crate::benchmarks::Benchmark;

pub fn benchmarks() -> Vec<Box<dyn Benchmark>> {
    vec![Box::new(chat_realtime_bench::ChatRealtimeBench)]
}
