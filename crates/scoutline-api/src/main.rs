//! `scoutline`: the standalone server.
//!
//! ```text
//! scoutline [--config PATH]               run the server
//! scoutline healthcheck [--config PATH]   exit 0 if the local server answers
//! ```
//!
//! The config path defaults to `$SCOUTLINE_CONFIG`, then
//! `/config/scoutline.toml`. If the file is missing, a starter config with
//! a fresh API key is written there.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use scoutline_api::config::{self, Config};
use scoutline_api::{AppState, router};
use tracing::{info, warn};

const DEFAULT_CONFIG: &str = "/config/scoutline.toml";

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let mut args = std::env::args().skip(1);
    let mut command = None;
    let mut config_path = PathBuf::from(
        std::env::var("SCOUTLINE_CONFIG").unwrap_or_else(|_| DEFAULT_CONFIG.to_owned()),
    );
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" | "-c" => match args.next() {
                Some(p) => config_path = p.into(),
                None => return usage("--config needs a path"),
            },
            "healthcheck" | "serve" if command.is_none() => command = Some(arg),
            "-h" | "--help" => return usage(""),
            other => return usage(&format!("unknown argument {other:?}")),
        }
    }

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: starting runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = match command.as_deref() {
        Some("healthcheck") => runtime.block_on(healthcheck(&config_path)),
        _ => runtime.block_on(serve(&config_path)),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn usage(problem: &str) -> ExitCode {
    if !problem.is_empty() {
        eprintln!("error: {problem}");
    }
    eprintln!("usage: scoutline [serve|healthcheck] [--config PATH]");
    if problem.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    }
}

async fn serve(path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        let key = config::random_key()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(path, config::starter_config(&key))
            .with_context(|| format!("writing {}", path.display()))?;
        info!(path = %path.display(), "wrote a starter config; the API key is in that file");
    }

    let mut cfg = Config::load(path)?;
    if let Ok(key) = std::env::var("SCOUTLINE_API_KEY") {
        if !key.trim().is_empty() {
            cfg.api_key = key;
        }
    }
    let engine = cfg.build_engine()?;
    let count = engine.indexers().count();
    if count == 0 {
        warn!(path = %path.display(), "no indexers configured yet; add [[indexer]] blocks and restart");
    }

    let listener = tokio::net::TcpListener::bind(cfg.listen)
        .await
        .with_context(|| format!("listening on {}", cfg.listen))?;
    info!(addr = %cfg.listen, indexers = count, "Scoutline {} ready", env!("CARGO_PKG_VERSION"));

    let app = router(AppState {
        engine,
        api_key: cfg.api_key,
    });
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await?;
    info!("stopped");
    Ok(())
}

/// Resolves on Ctrl-C or SIGTERM (`docker stop`).
async fn shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}

/// Used by the Docker HEALTHCHECK, since the image has no curl.
async fn healthcheck(path: &Path) -> anyhow::Result<()> {
    let port = Config::load(path)
        .map(|c| c.listen.port())
        .unwrap_or(config::DEFAULT_PORT);
    let client = scoutline_net::build_client(&Default::default(), &Default::default())?;
    let resp = client
        .get(format!("http://127.0.0.1:{port}/health"))
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await?;
    anyhow::ensure!(
        resp.status().is_success(),
        "health returned {}",
        resp.status()
    );
    Ok(())
}
