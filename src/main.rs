
use axum::{
    routing::{get, post, put},
    http::StatusCode,
    response::IntoResponse,
    Json, Router,
    extract::Extension
};
use tracing::{event, Level};
use serde::Deserialize;
use tokio::{process::Command, sync::mpsc, time};
use futures::{stream::futures_unordered::FuturesUnordered, StreamExt, future::FutureExt};

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let handler = spawn_handler();

    let app = Router::new()
        .route("/ox_notify", put(ox_notify))
        .layer(Extension(handler));

    // run our app with hyper
    // `axum::Server` is a re-export of `hyper::Server`
    let addr = SocketAddr::from(([10, 0, 0, 1], 12525));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn ox_notify(
    Extension(handler): Extension<mpsc::Sender<OxMessage>>,
    Json(payload): Json<OxMessage>
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
    pending_messages: u32,
    consecutive_fails: u32,
    executing: bool
}

fn spawn_handler() -> mpsc::Sender<OxMessage> {
    let (tx, mut rx) = mpsc::channel::<OxMessage>(16);

    // hardcode this for now
    let mut accounts = HashMap::new();
    accounts.insert("freaky".to_string(), Account::default());
    accounts.insert("veron".to_string(), Account::default());

    // Could set a more lenient MissedTickBehaviour
    let mut tock = time::interval(Duration::from_secs(10));
    let mut tasks = FuturesUnordered::new();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                message = rx.recv() => {
                    if let Some(message) = message {
                        if let Some(account) = accounts.get_mut(&message.user) {
                            // in case fdm leaves messages on the server,
                            // just count notifications instead of unseen
                            event!(Level::INFO, "New mail for {}", message.user);
                            account.pending_messages += 1;
                        }
                    }
                }
                Some(result) = tasks.next() => {
                    let (user, status) = result;
                    event!(Level::INFO, "Complete for {}: {:?}", user, status);
                    if let Some(account) = accounts.get_mut(&user) {
                        account.last_update = Some(Instant::now());
                        account.executing = false;
                        account.pending_messages = 0;
                    }
                }
                _ = tock.tick() => {
                    for (user, account) in accounts.iter_mut() {
                        event!(Level::DEBUG, "Tick for {}: {:?}", user, account);

                        // Nothing to do
                        if account.pending_messages == 0 || account.executing {
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
    });

    tx
}
