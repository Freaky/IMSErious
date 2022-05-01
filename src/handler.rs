use serde::Deserialize;
use tokio::{
    sync::watch,
    time::{timeout_at, Duration, Instant},
};

use std::sync::Arc;

use crate::config::SplitCommand;
use crate::message::{ImseEvent, ImseMessage};

#[derive(Deserialize, Debug, Clone)]
pub struct Handler {
    pub event: ImseEvent,
    pub user: String,
    #[serde(with = "humantime_serde")]
    pub min_delay: Duration,
    #[serde(with = "humantime_serde")]
    pub max_delay: Option<Duration>,
    pub command: SplitCommand,
}

pub type HandlerPayload = Option<Arc<ImseMessage>>;
pub type HandlerSender = watch::Sender<HandlerPayload>;

impl Handler {
    pub fn into_sender_handle(self) -> (HandlerSender, tokio::task::JoinHandle<()>) {
        let (tx, mut rx) = watch::channel::<HandlerPayload>(None);

        let task = tokio::spawn(async move {
            let min_delay = self.min_delay;
            let max_delay = self.max_delay.unwrap_or(Duration::from_secs(3600));
            let mut next_delay = max_delay;
            let mut last_execution = Instant::now();

            while let Ok(event) = timeout_at(last_execution + next_delay, rx.changed())
                .await
                .ok()
                .transpose()
            {
                match event {
                    Some(_) if last_execution.elapsed() < min_delay => {
                        tracing::trace!(
                            "scheduling next wakeup for {}::{}",
                            self.event,
                            &self.user
                        );
                        next_delay = min_delay;
                        continue;
                    }
                    None if self.max_delay.is_none() => continue,
                    _ => (),
                }

                let mut command = self.command.as_tokio_command();
                command.env("IMSE_USER", &self.user);
                if let Some(message) = &*rx.borrow() {
                    command
                        .env("IMSE_EVENT", message.event.to_string())
                        .env("IMSE_UNSEEN", message.unseen.to_string())
                        .env("IMSE_FOLDER", &message.folder)
                        .env("IMSE_FROM", message.from.as_deref().unwrap_or(""))
                        .env("IMSE_SNIPPET", message.snippet.as_deref().unwrap_or(""));
                }

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
                    )
                }

                next_delay = max_delay;
            }
        });

        (tx, task)
    }
}
