use clap::{Parser, ValueEnum};

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchmarkSuite {
    Standard,
    ChatRuntime,
}

impl BenchmarkSuite {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::ChatRuntime => "chat-runtime",
        }
    }
}

/// CLI configuration for the benchmark tool.
#[derive(Parser, Debug, Clone)]
#[command(name = "kalamdb-bench", about = "KalamDB benchmark & report generator")]
pub struct Config {
    /// KalamDB server URLs (comma-separated).
    /// Example: --urls http://127.0.0.1:2900,http://127.0.0.2:2900
    #[arg(
        long,
        value_delimiter = ',',
        default_value = "http://localhost:2900",
        env = "KALAMDB_URLS"
    )]
    pub urls: Vec<String>,

    /// Username for Basic auth
    #[arg(long, default_value = "admin", env = "KALAMDB_USER")]
    pub user: String,

    /// Password for Basic auth
    #[arg(long, default_value = "kalamdb123", env = "KALAMDB_PASSWORD")]
    pub password: String,

    /// Number of iterations per benchmark operation
    #[arg(long, default_value_t = 100)]
    pub iterations: u32,

    /// Number of warmup iterations (excluded from measurements)
    #[arg(long, default_value_t = 5)]
    pub warmup: u32,

    /// Concurrency level for concurrent benchmarks
    #[arg(long, default_value_t = 10)]
    pub concurrency: u32,

    /// Output directory for reports
    #[arg(long, default_value = "results")]
    pub output_dir: String,

    /// Benchmark suite to run. The standard suite writes to results/ by default;
    /// named suites should pass their own output directory from their runner script.
    #[arg(long, value_enum, default_value_t = BenchmarkSuite::Standard)]
    pub suite: BenchmarkSuite,

    /// Run only benchmarks matching this filter (substring match)
    #[arg(long)]
    pub filter: Option<String>,

    /// Run only these benchmark names (exact match). Repeat flag for multiple benches.
    /// Example: --bench subscriber_scale --bench reconnect_subscribe
    #[arg(long, value_delimiter = ',')]
    pub bench: Vec<String>,

    /// Print all available benchmark names and exit
    #[arg(long, default_value_t = false)]
    pub list_benches: bool,

    /// Namespace to use for benchmark tables
    #[arg(long, default_value = "bench")]
    pub namespace: String,

    /// Maximum number of subscribers for the subscriber_scale benchmark
    #[arg(long, default_value_t = 100_000)]
    pub max_subscribers: u32,
}
