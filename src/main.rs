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
    config::{Config, LoggingFormat},
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
        tracing::warn!(%remote_addr, method=%req.method(), uri=%req.uri(), "reject");
        Err(StatusCode::FORBIDDEN)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .unwrap_or_else(|| "/usr/local/etc/imserious.toml".into());
    let config = Config::from_path(&path).with_context(|| {
        format!(
            "Failed to load configuration from {}",
            path.to_string_lossy()
        )
    })?;

    let trace = tracing_subscriber::fmt()
        .with_target(config.log.target)
        .with_level(config.log.display_level)
        .with_ansi(config.log.ansi)
        .with_max_level(config.log.level.inner());

    if config.log.timestamp {
        match config.log.format {
            LoggingFormat::Full => trace.init(),
            LoggingFormat::Compact => trace.compact().init(),
            LoggingFormat::Pretty => trace.pretty().init(),
            LoggingFormat::Json => trace.json().init(),
        }
    } else {
        match config.log.format {
            LoggingFormat::Full => trace.without_time().init(),
            LoggingFormat::Compact => trace.without_time().compact().init(),
            LoggingFormat::Pretty => trace.without_time().pretty().init(),
            LoggingFormat::Json => trace.without_time().json().init(),
        }
    }

    tracing::info!(name=%env!("CARGO_PKG_NAME"), version=%env!("CARGO_PKG_VERSION"), "start");
    let res = run(config).await;
    if let Err(ref error) = res {
        tracing::error!(%error);
        for error_cause in error.chain().skip(1) {
            tracing::error!(%error_cause);
        }
    }
    tracing::info!("exit");
    res
}

async fn run(config: Config) -> Result<()> {
    let mut handlers = vec![];
    let mut tasks = vec![];
    for handler in config.handler {
        tracing::debug!(?handler, "register_handler");
        let (tx, task) = handler.clone().into_sender_handle();
        tasks.push(task);
        handlers.push((handler.event, handler.user, tx));
    }

    let allow = Arc::new(config.allow.clone());

    let app = Router::new()
        .route(config.endpoint.as_deref().unwrap_or("/notify"), put(notify))
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(handle_error))
                .load_shed()
                .concurrency_limit(config.max_connections.map_or(8, |x| x.get()) as usize)
                .timeout(
                    config
                        .timeout
                        .map_or(Duration::from_secs(5), Duration::from)
                )
                .layer(TraceLayer::new_for_http())
                .option_layer(
                    config
                        .auth
                        .map(|auth| RequireAuthorizationLayer::basic(&auth.user, &auth.pass)),
                )
                .layer(Extension(Arc::new(handlers)))
                .layer(from_extractor::<ContentLengthLimit<(), 1024>>())
                .into_inner(),
        )
        .route_layer(middleware::from_fn(move |req, next| {
            ip_restriction(req, next, allow.clone())
        }));

    let handle = Handle::new();
    let h = handle.clone();
    tokio::spawn(async move {
        shutdown_future().await;
        h.shutdown();
    });

    let addr = config
        .listen
        .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 12525)));

    tracing::info!(%addr, tls=config.tls.is_some(), "listen");

    if let Some(tls) = config.tls {
        let tls_config = RustlsConfig::from_pem_file(&tls.cert, &tls.key)
            .await
            .with_context(|| {
                format!(
                    "creating TLS configuration, cert={} key={}",
                    tls.cert, tls.key
                )
            })?;

        if tls.periodic_reload.is_some() {
            tokio::spawn(tls_reload(tls_config.clone(), tls));
        }

        axum_server::bind_rustls(addr, tls_config)
            .handle(handle)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await?;
    } else {
        axum_server::bind(addr)
            .handle(handle)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await?;
    }

    for task in tasks {
        task.await?;
    }

    Ok(())
}

async fn tls_reload(config: RustlsConfig, tls: crate::config::TlsConfig) {
    let period = tls
        .periodic_reload
        .expect("Periodic reload should be specified")
        .into_std();
    let mut delay = period;
    let mut fails = 0;
    loop {
        tokio::time::sleep(delay).await;
        let res = config.reload_from_pem_file(&tls.cert, &tls.key).await;
        match res {
            Ok(_) => {
                fails = 0;
                delay = period;
                tracing::info!(reload=%"success", next=?delay, "tls");
            }
            Err(e) => {
                fails += 1;
                delay = Duration::from_secs(60 * std::cmp::min(15, fails));
                tracing::error!(reload=%"error", retry=?delay, error=%e, "tls");
            }
        }
    }
}

#[tracing::instrument(skip_all)]
async fn notify(
    Extension(handlers): Extension<Arc<Vec<(ImseEvent, String, HandlerSender)>>>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    Json(mut message): Json<ImseMessage>,
) -> impl IntoResponse {
    tracing::info!(%remote_addr, event=?message.event, user=%message.user);
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
        _ = ctrl_c => tracing::info!(kind=%"interrupt", "signal"),
        _ = terminate => tracing::info!(kind=%"terminate", "signal"),
    }
}
