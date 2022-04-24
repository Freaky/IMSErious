
use axum::{
    routing::{get, post, put},
    http::StatusCode,
    response::IntoResponse,
    Json, Router,
    extract::Extension
};
use tracing::{event, Level};
use serde::Deserialize;
use tokio::{sync::Mutex, process::Command};

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let mut accounts = HashMap::new();
    accounts.insert("freaky".to_string(), Instant::now());
    accounts.insert("veron".to_string(), Instant::now());
    let accounts = Arc::new(Mutex::new(accounts));

    let app = Router::new()
        .route("/ox_notify", put(ox_notify))
        .layer(Extension(accounts));

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
    Extension(state): Extension<State>,
    Json(payload): Json<OxMessage>
) -> impl IntoResponse {
    event!(Level::INFO, "{:?}", payload);
    if payload.event != "messageNew" {
        return StatusCode::OK;
    }

    let mut state = state.lock().await;
    match state.get_mut(&payload.user) {
        Some(last_run) => {
            if last_run.elapsed() < Duration::from_secs(15) {
                event!(Level::DEBUG, "ratelimit");
                return StatusCode::OK;
            }
            *last_run = Instant::now();
            let mut command = Command::new("/usr/local/bin/sudo");
            command
                .arg("-u")
                .arg(payload.user)
                .arg("/usr/local/bin/fdm")
                .args(&["-a", "eda"]) // account eda
                .args(&["-l"]) // log to syslog
                .arg("fetch");
            event!(Level::DEBUG, "Executing {:?}", command);
            let result = command.status().await;
            event!(Level::DEBUG, "Result {:?}", result);
            match result {
                Ok(_) => StatusCode::OK,
                Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
            }
        }
        None => {
            event!(Level::ERROR, "Unknown user: {}", payload.user);
            StatusCode::NOT_FOUND
        }
        _ => StatusCode::OK
    }
}

type State = Arc<Mutex<HashMap<String, Instant>>>;

#[derive(Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
struct OxMessage {
    event: String,
    folder: String,
    from: Option<String>,
    imap_uid: Option<u32>,
    imap_uidvalidity: u32,
    snippet: Option<String>,
    unseen: u32,
    user: String,
}
