pub mod benchmarks;
pub mod chat_runtime;
pub mod client;
pub mod comparison;
pub mod config;
pub mod metrics;
pub mod preflight;
pub mod reporter;
pub mod runner;
pub mod soak;
pub mod system_info;
pub mod verdict;

use config::BenchmarkSuite;

pub fn selected_benchmarks(suite: BenchmarkSuite) -> Vec<Box<dyn benchmarks::Benchmark>> {
    match suite {
        BenchmarkSuite::Standard => benchmarks::standard_benchmarks(),
        BenchmarkSuite::ChatRuntime => chat_runtime::benchmarks(),
    }
}
