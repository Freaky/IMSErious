use axum::{
    extract::Extension, http::StatusCode, response::IntoResponse, routing::put, Json, Router,
};
use futures::{future::FutureExt, stream::futures_unordered::FuturesUnordered, StreamExt};
use serde::Deserialize;
use tokio::{process::Command, signal, sync::mpsc, time};
use tracing::{event, Level};

use std::collections::HashMap;
use std::net::SocketAddr;
use std::process::ExitStatus;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let (tx, task) = spawn_handler();
    let app = Router::new()
        .route("/ox_notify", put(ox_notify))
        .layer(Extension(tx));

    let addr = SocketAddr::from(([10, 0, 0, 1], 12525));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .with_graceful_shutdown(async move {
            task.await.unwrap();
        })
        .await
        .unwrap();
}

async fn ox_notify(
    Extension(handler): Extension<mpsc::Sender<OxMessage>>,
    Json(payload): Json<OxMessage>,
) -> impl IntoResponse {
    event!(Level::DEBUG, "{:?}", payload);

    if payload.event == "messageNew" {
        if handler.send(payload).await.is_err() {
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    }

    StatusCode::OK
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

#[derive(Debug, Default)]
struct Account {
    last_update: Option<Instant>,
    pending_messages: bool,
    consecutive_fails: u32,
    executing: bool,
}

fn spawn_handler() -> (mpsc::Sender<OxMessage>, tokio::task::JoinHandle<()>) {
    let (tx, mut rx) = mpsc::channel::<OxMessage>(16);

    // hardcode this for now
    let mut accounts = HashMap::new();
    accounts.insert("freaky".to_string(), Account::default());
    accounts.insert("veron".to_string(), Account::default());

    // Could set a more lenient MissedTickBehaviour
    let mut tock = time::interval(Duration::from_secs(10));
    let mut tasks = FuturesUnordered::new();
    let mut shutdown = false;
    let mut terminate = tokio::spawn(shutdown_future());

    let task = tokio::spawn(async move {
        while !shutdown || !tasks.is_empty() {
            tokio::select! {
                _ = &mut terminate, if !shutdown => {
                    shutdown = true;
                }
                Some(message) = rx.recv() => {
                    if let Some(account) = accounts.get_mut(&message.user) {
                        // in case fdm leaves messages on the server,
                        // just count notifications instead of unseen
                        event!(Level::INFO, "New mail for {}", message.user);
                        account.pending_messages = true;
                    }
                }
                Some((user, status)) = tasks.next() => {
                    event!(Level::INFO, "Complete for {}: {:?}", user, status);
                    if let Some(account) = accounts.get_mut(&user) {
                        account.last_update = Some(Instant::now());
                        account.executing = false;
                        account.pending_messages = false;

                        // Type inference seems to get a bit wonky here
                        match status {
                            Ok(status) => {
                                if ExitStatus::success(&status) {
                                    account.consecutive_fails = 0;
                                } else {
                                    account.consecutive_fails += 1;
                                }
                            }
                            _ => {
                                account.consecutive_fails += 1;
                            }
                        }
                    }
                }
                _ = tock.tick(), if !shutdown => {
                    for (user, account) in accounts.iter_mut() {
                        event!(Level::TRACE, "Tick for {}: {:?}", user, account);

                        // Nothing to do
                        if !account.pending_messages || account.executing {
                            continue;
                        }

                        if let Some(last_update) = account.last_update {
                            if last_update.elapsed() < Duration::from_secs(10) {
                                continue;
                            }
                        }

                        account.executing = true;
                        let mut command = Command::new("/usr/local/bin/sudo");
                        command
                            .args(&["-n", "-H"]) // non-interactive, set HOME
                            .arg("-u")
                            .arg(user)
                            .arg("/usr/local/bin/fdm")
                            .args(&["-a", "eda"]) // account eda
                            .args(&["-l"]) // log to syslog
                            .arg("fetch");
                        event!(Level::DEBUG, "Executing {:?}", command);
                        let user = user.clone();
                        tasks.push(command.status().map(|status| (user, status)));
                    }
                }
            }
        }

        event!(Level::TRACE, "handler loop exit");
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
