mod protocol;
mod server;
mod session_pool;

use server::{DaemonConfig, DaemonServer};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = DaemonConfig::default();

    // Initialize logging — daemon is long-running, use file-based log rotation.
    let _ = std::fs::create_dir_all(&config.log_dir);
    let file_appender = tracing_appender::rolling::daily(&config.log_dir, "daemon.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_ansi(false)
        .init();

    eprintln!(
        "crab-daemon v{} starting on port {}...",
        env!("CARGO_PKG_VERSION"),
        config.port,
    );

    let server = DaemonServer::new(config);

    // Graceful shutdown on Ctrl+C
    let server_handle = &server;
    tokio::select! {
        result = server_handle.run() => {
            if let Err(e) = result {
                eprintln!("daemon error: {e}");
                std::process::exit(1);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\nshutting down...");
            server_handle.shutdown().await;
        }
    }

    Ok(())
}
