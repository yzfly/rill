use rill_core::{Config, DEFAULT_UPSTREAM};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let origin = std::env::var("RILL_UPSTREAM").unwrap_or_else(|_| DEFAULT_UPSTREAM.into());
    let config = Config::new(&origin)?;
    let address = std::env::var("RILL_LISTEN").unwrap_or_else(|_| {
        std::env::var("PORT")
            .map(|p| format!("0.0.0.0:{p}"))
            .unwrap_or_else(|_| "127.0.0.1:8787".into())
    });
    let listener = tokio::net::TcpListener::bind(&address).await?;
    eprintln!("Rill listening on {}", listener.local_addr()?);
    axum::serve(listener, rill_server::router(config)?)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

async fn shutdown() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _ = interrupt => {}, _ = terminate => {} }
}
