//! `cargo xtask ci` — run the full CI pipeline locally:
//! fmt check, clippy, tests, and cargo-deny.

use clap::Args;

use super::run_cmd;

#[derive(Args)]
pub struct CiArgs {
    /// Skip cargo-deny check (useful if cargo-deny is not installed).
    #[arg(long)]
    pub skip_deny: bool,
}

pub fn run(args: &CiArgs) -> Result<(), String> {
    println!("=== xtask ci ===\n");

    println!("--- fmt check ---");
    run_cmd("cargo", &["fmt", "--all", "--check"])?;

    println!("\n--- clippy ---");
    run_cmd("cargo", &["clippy", "--workspace", "--", "-D", "warnings"])?;

    println!("\n--- test ---");
    run_cmd("cargo", &["test", "--workspace"])?;

    if !args.skip_deny {
        println!("\n--- cargo deny ---");
        run_cmd("cargo", &["deny", "check"])?;
    } else {
        println!("\n--- cargo deny (skipped) ---");
    }

    println!("\n=== ci passed ===");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ci_args_default() {
        let args = CiArgs { skip_deny: false };
        assert!(!args.skip_deny);
    }

    #[test]
    fn ci_args_skip_deny() {
        let args = CiArgs { skip_deny: true };
        assert!(args.skip_deny);
    }
}
