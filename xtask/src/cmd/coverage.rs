//! `cargo xtask coverage` — generate code coverage reports via cargo-tarpaulin.

use clap::Args;

use super::run_cmd;

#[derive(Args)]
pub struct CoverageArgs {
    /// Output format: html, xml, json, or lcov.
    #[arg(short, long, default_value = "html")]
    pub format: String,
    /// Output directory for the report.
    #[arg(short, long, default_value = "target/coverage")]
    pub output: String,
    /// Only measure coverage for a specific crate.
    #[arg(short, long)]
    pub package: Option<String>,
}

pub fn run(args: &CoverageArgs) -> Result<(), String> {
    println!("=== xtask coverage ===\n");

    let mut cmd_args: Vec<String> = vec![
        "tarpaulin".into(),
        "--out".into(),
        args.format.clone(),
        "--output-dir".into(),
        args.output.clone(),
        "--skip-clean".into(),
    ];

    if let Some(pkg) = &args.package {
        cmd_args.push("-p".into());
        cmd_args.push(pkg.clone());
    } else {
        cmd_args.push("--workspace".into());
    }

    let cmd_refs: Vec<&str> = cmd_args.iter().map(String::as_str).collect();
    run_cmd("cargo", &cmd_refs)?;

    println!("\ncoverage report written to {}", args.output);
    println!("\n=== coverage complete ===");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_args_defaults() {
        let args = CoverageArgs {
            format: "html".into(),
            output: "target/coverage".into(),
            package: None,
        };
        assert_eq!(args.format, "html");
        assert_eq!(args.output, "target/coverage");
        assert!(args.package.is_none());
    }

    #[test]
    fn coverage_args_custom() {
        let args = CoverageArgs {
            format: "lcov".into(),
            output: "out/cov".into(),
            package: Some("crab-core".into()),
        };
        assert_eq!(args.format, "lcov");
        assert_eq!(args.output, "out/cov");
        assert_eq!(args.package.as_deref(), Some("crab-core"));
    }
}
