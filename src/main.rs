use anyhow::Result;
use axum::{
    extract::Extension, http::StatusCode, response::IntoResponse, routing::put, Json, Router,
};
use derive_more::Display;
use serde::Deserialize;
use tokio::{
    process::Command,
    signal,
    sync::watch,
    time::{timeout_at, Duration, Instant},
};
use tracing::{event, Level};

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let config_path = "/usr/local/etc/imserious.toml";

    tracing::debug!("loading config from {}", config_path);
    let config: Config =
        toml::from_str(&std::fs::read_to_string(&config_path)?)?;

    let mut handlers = HashMap::new();
    let mut tasks = vec![];
    for handler in config.handler {
        tracing::debug!("register handler: {:?}", handler);
        let (tx, task) = command_handler(handler.clone());
        tasks.push(task);
        handlers.insert((handler.event, handler.user.clone()), tx);
    }

    let app = Router::new()
        .route("/notify", put(notify))
        .layer(Extension(Arc::new(handlers)));

    let addr = config
        .listen
        .unwrap_or(SocketAddr::from(([127, 0, 0, 1], 12525)));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .with_graceful_shutdown(shutdown_future())
        .await?;

    for task in tasks {
        event!(Level::TRACE, "handler shutdown {:?}", task.await);
    }

    Ok(())
}

#[tracing::instrument(skip(handler))]
async fn notify(
    Extension(handler): Extension<
        Arc<HashMap<(ImseEvent, String), watch::Sender<Option<ImseMessage>>>>,
    >,
    Json(message): Json<ImseMessage>,
) -> impl IntoResponse {
    if let Some(tx) = handler.get(&(message.event, message.user.clone())) {
        let _ = tx.send(Some(message));
    }
    StatusCode::OK
}

#[derive(Copy, Clone, Deserialize, Debug, Display, Hash, PartialEq, Eq)]
#[serde(try_from = "&str")]
enum ImseEvent {
    FlagsClear,
    FlagsSet,
    MailboxCreate,
    MailboxDelete,
    MailboxRename,
    MailboxSubscribe,
    MailboxUnsubscribe,
    MessageAppend,
    MessageExpunge,
    MessageNew,
    MessageRead,
    MessageTrash,
}

impl TryFrom<&str> for ImseEvent {
    type Error = &'static str;

    fn try_from(string: &str) -> Result<Self, Self::Error> {
        if string.eq_ignore_ascii_case("FlagsClear") {
            Ok(Self::FlagsClear)
        } else if string.eq_ignore_ascii_case("FlagsSet") {
            Ok(Self::FlagsSet)
        } else if string.eq_ignore_ascii_case("MailboxCreate") {
            Ok(Self::MailboxCreate)
        } else if string.eq_ignore_ascii_case("MailboxDelete") {
            Ok(Self::MailboxDelete)
        } else if string.eq_ignore_ascii_case("MailboxRename") {
            Ok(Self::MailboxRename)
        } else if string.eq_ignore_ascii_case("MailboxSubscribe") {
            Ok(Self::MailboxSubscribe)
        } else if string.eq_ignore_ascii_case("MailboxUnsubscribe") {
            Ok(Self::MailboxUnsubscribe)
        } else if string.eq_ignore_ascii_case("MessageAppend") {
            Ok(Self::MessageAppend)
        } else if string.eq_ignore_ascii_case("MessageExpunge") {
            Ok(Self::MessageExpunge)
        } else if string.eq_ignore_ascii_case("MessageNew") {
            Ok(Self::MessageNew)
        } else if string.eq_ignore_ascii_case("MessageRead") {
            Ok(Self::MessageRead)
        } else if string.eq_ignore_ascii_case("MessageTrash") {
            Ok(Self::MessageTrash)
        } else {
            Err("unknown message type")
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "&str")]
struct SplitCommand(Vec<String>);

impl TryFrom<&str> for SplitCommand {
    type Error = &'static str;

    fn try_from(string: &str) -> Result<Self, Self::Error> {
        let command = shell_words::split(string).map_err(|_| "missing closing quote")?;
        if command.is_empty() {
            return Err("command is empty");
        }
        Ok(Self(command))
    }
}

impl SplitCommand {
    fn as_tokio_command(&self) -> Command {
        let mut command = Command::new(&self.0[0]);
        if self.0.len() > 1 {
            command.args(&self.0[1..]);
        }
        command
    }
}

#[derive(Deserialize, Debug)]
struct ImseMessage {
    event: ImseEvent,
    user: String,
    unseen: u32,
    folder: String,
    from: Option<String>,
    snippet: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
struct Config {
    listen: Option<SocketAddr>,
    handler: Vec<Handler>,
}

#[derive(Deserialize, Debug, Clone)]
struct Handler {
    event: ImseEvent,
    user: String,
    #[serde(with = "humantime_serde")]
    min_delay: Duration,
    #[serde(with = "humantime_serde")]
    max_delay: Option<Duration>,
    command: SplitCommand,
}

fn command_handler(
    handler: Handler,
) -> (
    watch::Sender<Option<ImseMessage>>,
    tokio::task::JoinHandle<()>,
) {
    let (tx, mut rx) = watch::channel::<Option<ImseMessage>>(None);

    let task = tokio::spawn(async move {
        let min_delay = handler.min_delay;
        let max_delay = handler.max_delay.unwrap_or(Duration::from_secs(3600));
        let mut next_delay = max_delay;
        let mut last_execution = Instant::now();

        while let Ok(event) = timeout_at(last_execution + next_delay, rx.changed())
            .await
            .ok()
            .transpose()
        {
            match event {
                Some(_) if last_execution.elapsed() < min_delay => {
                    event!(
                        Level::TRACE,
                        "scheduling next wakeup for {}::{}",
                        handler.event,
                        &handler.user
                    );
                    next_delay = min_delay;
                    continue;
                }
                None if handler.max_delay.is_none() => continue,
                _ => (),
            }

            let mut command = handler.command.as_tokio_command();
            if let Some(message) = &*rx.borrow() {
                command
                    .env("IMSE_EVENT", message.event.to_string())
                    .env("IMSE_USER", &handler.user)
                    .env("IMSE_UNSEEN", message.unseen.to_string())
                    .env("IMSE_FOLDER", &message.folder)
                    .env("IMSE_FROM", message.from.as_deref().unwrap_or(""))
                    .env("IMSE_SNIPPET", message.snippet.as_deref().unwrap_or(""));
            }

            let result = command.status().await;
            last_execution = Instant::now();
            event!(
                Level::INFO,
                "execution complete for {}::{}: {:?}",
                handler.event,
                &handler.user,
                result
            );
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
