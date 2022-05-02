use anyhow::Result;
use axum::{
    extract::{ConnectInfo, Extension},
    http::StatusCode,
    response::IntoResponse,
    routing::put,
    Json, Router,
};
use ipnet::IpNet;
use tokio::signal;

use std::net::SocketAddr;
use std::sync::Arc;

mod config;
mod handler;
mod message;
use crate::{
    config::Config,
    handler::HandlerSender,
    message::{ImseEvent, ImseMessage},
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().without_time().init();

    let config = Config::from_path("/usr/local/etc/imserious.toml")?;

    let mut handlers = vec![];
    let mut tasks = vec![];
    for handler in config.handler.into_iter() {
        tracing::debug!("register handler: {:?}", handler);
        let (tx, task) = handler.clone().into_sender_handle();
        tasks.push(task);
        handlers.push((handler.event, handler.user, tx));
    }

    let addr = config
        .listen
        .unwrap_or(SocketAddr::from(([127, 0, 0, 1], 12525)));
    tracing::info!("listening on {}", addr);
    for net in &config.allow {
        tracing::info!("restricting to {}", net);
    }

    let app = Router::new()
        .route("/notify", put(notify))
        .layer(Extension(Arc::new(handlers)))
        .layer(Extension(Arc::new(config.allow)));

    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_future())
        .await?;

    for task in tasks {
        tracing::trace!("handler shutdown {:?}", task.await);
    }

    Ok(())
}

async fn notify(
    Extension(handlers): Extension<Arc<Vec<(ImseEvent, String, HandlerSender)>>>,
    Extension(allowed_ranges): Extension<Arc<Vec<IpNet>>>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    Json(message): Json<ImseMessage>,
) -> impl IntoResponse {
    if !allowed_ranges.is_empty()
        && !allowed_ranges
            .iter()
            .any(|range| range.contains(&remote_addr.ip()))
    {
        tracing::warn!(
            "event rejected from {}: {}::{}",
            remote_addr,
            message.event,
            message.user
        );
        return StatusCode::FORBIDDEN;
    }

    tracing::info!(
        "event received from {}: {}::{}",
        remote_addr,
        message.event,
        message.user
    );
    let message = Arc::new(message);
    for (_, _, handler) in handlers
        .iter()
        .filter(|(event, user, _)| *event == message.event && *user == message.user)
    {
        drop(handler.send(Some(Arc::clone(&message))));
    }

    StatusCode::OK
}

async fn shutdown_future() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("signal: SIGINT"),
        _ = terminate => tracing::info!("signal: SIGTERM"),
    }
}
