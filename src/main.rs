use anyhow::{Context, Result};
use axum::{
    error_handling::HandleErrorLayer,
    extract::{ConnectInfo, ContentLengthLimit, Extension},
    http::{Request, StatusCode},
    middleware::{self, from_extractor, Next},
    response::IntoResponse,
    routing::put,
    Json, Router,
};
use axum_server::{tls_rustls::RustlsConfig, Handle};
use tokio::{signal, time::Duration};
use tower::{BoxError, ServiceBuilder};
use tower_http::{auth::RequireAuthorizationLayer, trace::TraceLayer};

use std::{borrow::Cow, net::SocketAddr, sync::Arc};

mod config;
mod handler;
mod message;
use crate::{
    config::Config,
    handler::HandlerSender,
    message::{ImseEvent, ImseMessage},
};

async fn ip_restriction<B>(
    req: Request<B>,
    next: Next<B>,
    allowed_ranges: Arc<Vec<ipnet::IpNet>>,
) -> impl IntoResponse {
    let ConnectInfo(remote_addr): &ConnectInfo<SocketAddr> =
        req.extensions().get().expect("ConnectInfo<SocketAddr>");
    if allowed_ranges.is_empty()
        || allowed_ranges
            .iter()
            .any(|range| range.contains(&remote_addr.ip()))
    {
        Ok(next.run(req).await)
    } else {
        tracing::warn!("request rejected from {}", remote_addr);
        Err(StatusCode::FORBIDDEN)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().without_time().init();

    let path = std::env::args_os()
        .nth(1)
        .unwrap_or_else(|| "/usr/local/etc/imserious.toml".into());
    let config = Config::from_path(&path).with_context(|| {
        format!(
            "Failed to load configuration from {}",
            path.to_string_lossy()
        )
    })?;

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
        .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 12525)));
    tracing::info!("listening on {}", addr);
    for net in &config.allow {
        tracing::info!("restricting to {}", net);
    }
    let allow = Arc::new(config.allow.clone());

    let app = Router::new()
        .route(config.endpoint.as_deref().unwrap_or("/notify"), put(notify))
        .route_layer(middleware::from_fn(move |req, next| {
            ip_restriction(req, next, allow.clone())
        }))
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(handle_error))
                .load_shed()
                .concurrency_limit(
                    config
                        .max_connections
                        .or(std::num::NonZeroU16::new(8))
                        .unwrap()
                        .get() as usize,
                )
                .timeout(config.timeout.unwrap_or(Duration::from_secs(5)))
                .layer(TraceLayer::new_for_http())
                .option_layer(
                    config
                        .auth
                        .map(|auth| RequireAuthorizationLayer::basic(&auth.user, &auth.pass)),
                )
                .layer(Extension(Arc::new(handlers)))
                .layer(from_extractor::<ContentLengthLimit<(), 1024>>())
                .into_inner(),
        );

    if let Some(tls) = config.tls {
        let handle = Handle::new();
        let h = handle.clone();
        tokio::spawn(async move {
            shutdown_future().await;
            h.graceful_shutdown(None)
        });
        let tls_config = RustlsConfig::from_pem_file(&tls.cert, &tls.key)
            .await
            .with_context(|| {
                format!(
                    "creating TLS configuration, cert={} key={}",
                    tls.cert, tls.key
                )
            })?;
        axum_server::bind_rustls(addr, tls_config)
            .handle(handle)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await?;
    } else {
        axum::Server::bind(&addr)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(shutdown_future())
            .await?;
    }

    for task in tasks {
        tracing::trace!("handler shutdown {:?}", task.await);
    }

    Ok(())
}

async fn notify(
    Extension(handlers): Extension<Arc<Vec<(ImseEvent, String, HandlerSender)>>>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    Json(mut message): Json<ImseMessage>,
) -> impl IntoResponse {
    tracing::info!(
        "event received from {}: {}::{}",
        remote_addr,
        message.event,
        message.user
    );
    message.remote_addr = Some(remote_addr);
    let message = Arc::new(message);
    for (_, _, handler) in handlers
        .iter()
        .filter(|(event, user, _)| *event == message.event && *user == message.user)
    {
        drop(handler.send(Some(Arc::clone(&message))));
    }

    StatusCode::OK
}

async fn handle_error(error: BoxError) -> impl IntoResponse {
    if error.is::<tower::timeout::error::Elapsed>() {
        return (StatusCode::REQUEST_TIMEOUT, Cow::from("request timed out"));
    }

    if error.is::<tower::load_shed::error::Overloaded>() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Cow::from("service is overloaded, try again later"),
        );
    }

    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Cow::from(format!("Unhandled internal error: {}", error)),
    )
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
