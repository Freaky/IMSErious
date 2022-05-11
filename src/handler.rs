use serde::Deserialize;
use tokio::{
    sync::watch,
    time::{timeout_at, Duration, Instant},
};

use std::sync::Arc;

use crate::{
    config::SplitCommand,
    message::{ImseEvent, ImseMessage},
};

#[derive(Deserialize, Debug, Clone)]
pub struct Handler {
    pub event: ImseEvent,
    pub user: String,
    #[serde(with = "humantime_serde")]
    pub min_delay: Duration,
    #[serde(default, with = "humantime_serde")]
    pub max_delay: Option<Duration>,
    pub command: SplitCommand,
}

pub type HandlerPayload = Option<Arc<ImseMessage>>;
pub type HandlerSender = watch::Sender<HandlerPayload>;

impl Handler {
    async fn task(self, mut rx: watch::Receiver<HandlerPayload>) {
        let min_delay = self.min_delay;
        let max_delay = self.max_delay.unwrap_or(Duration::from_secs(3600));
        let mut next_delay = max_delay;
        let mut last_execution = Instant::now();
        let mut latest: HandlerPayload = None;

        while let Ok(event) = timeout_at(last_execution + next_delay, rx.changed())
            .await
            .ok()
            .transpose()
        {
            match event {
                Some(_) => {
                    latest = rx.borrow_and_update().clone();
                    if last_execution.elapsed() < min_delay {
                        tracing::trace!(
                            "scheduling next wakeup for {}::{}",
                            self.event,
                            self.user
                        );
                        next_delay = min_delay;
                        continue;
                    }
                }
                None => {
                    if latest.is_none() && self.max_delay.is_none() {
                        continue;
                    }
                }
            }

            let mut command = self.command.as_tokio_command();
            command
                .env("IMSE_USER", &self.user)
                .env("IMSE_EVENT", self.event.to_string());

            if let Some(message) = latest.take() {
                if let Some(remote) = message.remote_addr {
                    command
                        .env("IMSE_REMOTE_IP", remote.ip().to_string())
                        .env("IMSE_REMOTE_PORT", remote.port().to_string());
                }
                command
                    .env("IMSE_UNSEEN", message.unseen.to_string())
                    .env("IMSE_FOLDER", &message.folder)
                    .env("IMSE_FROM", message.from.as_deref().unwrap_or(""))
                    .env("IMSE_SNIPPET", message.snippet.as_deref().unwrap_or(""));
            }

            tracing::trace!("execute for {}::{}: {:?}", self.event, self.user, command);
            let result = command.status().await;
            last_execution = Instant::now();
            if let Ok(result) = result {
                tracing::info!(
                    "execution complete for {}::{}: rc={}",
                    self.event,
                    &self.user,
                    result.code().unwrap_or(-1)
                );
            } else {
                tracing::warn!(
                    "execution failed for {}::{}: status={:?}",
                    self.event,
                    &self.user,
                    result
                );
            }

            next_delay = max_delay;
        }
    }

    pub fn into_sender_handle(self) -> (HandlerSender, tokio::task::JoinHandle<()>) {
        let (tx, rx) = watch::channel::<HandlerPayload>(None);

        let task = tokio::spawn(self.task(rx));

        (tx, task)
    }
}
