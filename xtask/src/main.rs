mod cmd;

use clap::Parser;

#[derive(Parser)]
#[command(name = "xtask", about = "Crab Code development automation tasks")]
enum Cli {
    /// Run fmt + clippy + test + deny in sequence
    Ci(cmd::ci::CiArgs),
    /// Bump version, generate changelog, create git tag
    Release(cmd::release::ReleaseArgs),
    /// Run criterion benchmarks
    Bench(cmd::bench::BenchArgs),
    /// Generate code coverage report via cargo-tarpaulin
    Coverage(cmd::coverage::CoverageArgs),
}

fn main() {
    let cli = Cli::parse();
    let result = match cli {
        Cli::Ci(args) => cmd::ci::run(&args),
        Cli::Release(args) => cmd::release::run(&args),
        Cli::Bench(args) => cmd::bench::run(&args),
        Cli::Coverage(args) => cmd::coverage::run(&args),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
