// use futures::future::TryFutureExt;
use futures::FutureExt;
use governor::{Quota, RateLimiter};
use nonzero_ext::nonzero;
use serde::Deserialize;
use tokio::{
    sync::watch,
    time::{timeout_at, Duration, Instant},
};

use std::{num::NonZeroU32, sync::Arc};

use crate::{
    config::SplitCommand,
    message::{ImseEvent, ImseMessage},
};

#[derive(Deserialize, Debug, Clone)]
pub struct Handler {
    pub event: ImseEvent,
    pub user: String,
    #[serde(default, with = "humantime_serde")]
    pub delay: Option<Duration>,
    #[serde(default, with = "humantime_serde")]
    pub limit_period: Option<Duration>,
    #[serde(default)]
    pub limit_burst: Option<NonZeroU32>,
    #[serde(default, with = "humantime_serde")]
    pub periodic: Option<Duration>,
    pub command: SplitCommand,
}

pub type HandlerPayload = Option<Arc<ImseMessage>>;
pub type HandlerSender = watch::Sender<HandlerPayload>;

impl Handler {
    async fn task(self, mut rx: watch::Receiver<HandlerPayload>) {
        let mut last_execution = Instant::now();
        let mut latest: HandlerPayload = None;
        let period = self.periodic.unwrap_or(Duration::from_secs(3600));

        let quota = Quota::with_period(
            self.limit_period
                .filter(Duration::is_zero)
                .unwrap_or(Duration::from_secs(30)),
        )
        .expect("Non-zero Duration")
        .allow_burst(self.limit_burst.unwrap_or(nonzero!(1u32)));
        let limiter = RateLimiter::direct(quota);

        while let Ok(event) = timeout_at(
            last_execution + period,
            limiter.until_ready().then(|_| rx.changed()),
        )
        .await
        .ok()
        .transpose()
        {
            match event {
                Some(_) => {
                    latest = rx.borrow_and_update().clone();
                }
                None if latest.is_none() && self.periodic.is_none() => {
                    continue;
                }
                None => (),
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
        }
    }

    pub fn into_sender_handle(self) -> (HandlerSender, tokio::task::JoinHandle<()>) {
        let (tx, rx) = watch::channel::<HandlerPayload>(None);

        let task = tokio::spawn(self.task(rx));

        (tx, task)
    }
}
