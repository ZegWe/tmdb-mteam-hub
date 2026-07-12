pub mod app;
pub mod clients;
mod config;
mod douban;
pub mod http;
mod storage;
mod subscription;
mod tmdb_cache;

use std::net::SocketAddr;

pub use http::router::{build_api_router, build_router};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tmdb_mteam_server=info,tower_http=info,axum=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let app::BootstrappedApp {
        state,
        listen_addr,
        static_dir,
        _subscription_worker: subscription_worker,
    } = app::bootstrap(app::BootstrapOptions::from_env()).await?;
    let router = build_router(state, static_dir);
    let listener = tokio::net::TcpListener::bind(listen_addr).await?;
    tracing::info!("listen http://{}", listen_addr);
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    subscription_worker.shutdown().await;
    Ok(())
}
