use axum::{
    extract::Extension, http::StatusCode, response::IntoResponse, routing::put, Json, Router,
};
use serde::Deserialize;
use tokio::{
    process::Command,
    signal,
    sync::watch,
    time::{self, Duration, Instant},
};
use tracing::{event, Level};

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    tracing_subscriber::fmt::init();

    let mut accounts = HashMap::new();
    let mut tasks = vec![];
    for user in &["freaky", "veron"] {
        let (tx, task) = user_handler(user.to_string());
        tasks.push(task);
        accounts.insert(user.to_string(), tx);
    }

    let app = Router::new()
        .route("/ox_notify", put(ox_notify))
        .layer(Extension(Arc::new(accounts)));

    let addr = SocketAddr::from(([10, 0, 0, 1], 12525));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .with_graceful_shutdown(shutdown_future())
        .await
        .unwrap();

    for task in tasks {
        event!(Level::TRACE, "handler shutdown {:?}", task.await);
    }
}

async fn ox_notify(
    Extension(handler): Extension<Arc<HashMap<String, watch::Sender<()>>>>,
    Json(payload): Json<OxMessage>,
) -> impl IntoResponse {
    event!(Level::DEBUG, "{:?}", payload);

    if payload.event != "messageNew" {
        StatusCode::OK
    } else if handler
        .get(&payload.user)
        .and_then(|entry| entry.send(()).ok())
        .is_some()
    {
        StatusCode::OK
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
struct OxMessage {
    event: String,
    // fields we don't currently care about
    // folder: String,
    // from: Option<String>,
    // imap_uid: Option<u32>,
    // imap_uidvalidity: u32,
    // snippet: Option<String>,
    // unseen: u32,
    user: String,
}

fn user_handler(user: String) -> (watch::Sender<()>, tokio::task::JoinHandle<()>) {
    let (tx, mut rx) = watch::channel::<()>(());

    let task = tokio::spawn(async move {
        let min_delay = Duration::from_secs(30);
        let max_delay = Duration::from_secs(300);
        let mut next_delay = max_delay;
        let mut last_execution = Instant::now();

        loop {
            let event = time::timeout_at(last_execution + next_delay, rx.changed()).await;

            match event {
                Ok(Ok(())) => {
                    // Notification wakeup
                    event!(Level::INFO, "notification for {}", &user);
                    if last_execution.elapsed() < min_delay {
                        event!(Level::INFO, "scheduling next wakeup for {}", &user);
                        // schedule the next wakeup at our earliest allowable moment
                        next_delay = min_delay;
                        continue;
                    }
                }
                Err(_) => {
                    // Timeout wakeup
                    event!(Level::INFO, "scheduled wakeup for {}", &user);
                }
                Ok(Err(_)) => {
                    event!(Level::DEBUG, "shutdown handler for {}", &user);
                    break;
                }
            }

            event!(Level::INFO, "fetching for {}", &user);
            let mut command = Command::new("/usr/local/bin/sudo");
            command
                .args(&["-n", "-H"]) // non-interactive, set HOME
                .arg("-u")
                .arg(&user)
                .arg("/usr/local/bin/fdm")
                .args(&["-a", "eda"]) // account eda
                .args(&["-l"]) // log to syslog
                .arg("fetch");

            let result = command.status().await;
            last_execution = Instant::now();
            event!(Level::INFO, "fetch complete for {}: {:?}", &user, result);
            next_delay = max_delay;
        }
    });

    (tx, task)
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
        _ = ctrl_c => event!(Level::DEBUG, "SIGINT"),
        _ = terminate => event!(Level::DEBUG, "SIGTERM"),
    }
}
