//! `cargo xtask bench` — run criterion benchmarks.

use clap::Args;

use super::run_cmd;

#[derive(Args)]
pub struct BenchArgs {
    /// Only run benchmarks in a specific crate.
    #[arg(short, long)]
    pub package: Option<String>,
    /// Filter benchmarks by name.
    #[arg(trailing_var_arg = true)]
    pub filter: Vec<String>,
}

pub fn run(args: &BenchArgs) -> Result<(), String> {
    println!("=== xtask bench ===\n");

    let mut cmd_args = vec!["bench"];

    if let Some(pkg) = &args.package {
        cmd_args.push("-p");
        cmd_args.push(pkg);
    } else {
        cmd_args.push("--workspace");
    }

    // Append `--` and filter args if any
    let filter_strs: Vec<&str> = args.filter.iter().map(String::as_str).collect();
    if !filter_strs.is_empty() {
        cmd_args.push("--");
        cmd_args.extend_from_slice(&filter_strs);
    }

    run_cmd("cargo", &cmd_args)?;

    println!("\n=== bench complete ===");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bench_args_defaults() {
        let args = BenchArgs {
            package: None,
            filter: vec![],
        };
        assert!(args.package.is_none());
        assert!(args.filter.is_empty());
    }

    #[test]
    fn bench_args_with_package() {
        let args = BenchArgs {
            package: Some("crab-core".into()),
            filter: vec![],
        };
        assert_eq!(args.package.as_deref(), Some("crab-core"));
    }

    #[test]
    fn bench_args_with_filter() {
        let args = BenchArgs {
            package: None,
            filter: vec!["my_bench".into()],
        };
        assert_eq!(args.filter.len(), 1);
    }
}
