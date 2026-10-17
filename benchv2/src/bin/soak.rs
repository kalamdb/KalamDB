use clap::Parser;
use kalamdb_bench::soak::{run, SoakOptions};

#[tokio::main]
async fn main() {
    let options = SoakOptions::parse();
    match run(options).await {
        Ok(report) => std::process::exit(report.exit_code()),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        },
    }
}
