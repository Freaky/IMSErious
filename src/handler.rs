use governor::{Quota, RateLimiter};
use nonzero_ext::nonzero;
use tokio::{
    sync::watch,
    time::{timeout_at, Duration, Instant},
};

use std::sync::Arc;

use crate::{config::Handler, message::ImseMessage};

pub type HandlerPayload = Option<Arc<ImseMessage>>;
pub type HandlerSender = watch::Sender<HandlerPayload>;

impl Handler {
    async fn task(self, mut rx: watch::Receiver<HandlerPayload>) {
        let period = self
            .periodic
            .map_or(Duration::from_secs(3600), Duration::from);
        let mut latest: HandlerPayload = None;
        let mut now = Instant::now();
        let mut last_burst = now;
        let mut deadline = now + period;

        let quota = Quota::with_period(
            self.limit_period
                .map_or(Duration::from_secs(30), Duration::from)
        )
        .expect("Non-zero Duration")
        .allow_burst(self.limit_burst.unwrap_or(nonzero!(1u32)));
        let clock = governor::clock::MonotonicClock::default();
        let limiter = RateLimiter::direct_with_clock(quota, &clock);

        while let Ok(event) = timeout_at(deadline, rx.changed()).await.ok().transpose() {
            now = Instant::now();
            if event.is_some() {
                if latest.is_none() {
                    last_burst = now;
                }
                latest = rx.borrow_and_update().clone();

                if let Some(delay) = self.delay {
                    if let Some(delay) = delay.into_std().checked_sub(last_burst.elapsed()) {
                        deadline = now + delay;
                        continue;
                    }
                }
            } else if latest.is_none() && self.periodic.is_none() {
                // Ignore periodic wakeups if not configured for them
                continue;
            }

            // Let periodic execution ignore rate limits
            if latest.is_some() {
                if let Err(not_until) = limiter.check() {
                    deadline = not_until.earliest_possible().into();
                    continue;
                }
            }

            self.execute(latest.take()).await;
            deadline = Instant::now() + period;
        }
    }

    #[tracing::instrument(skip_all, fields(event=%self.event, user=%self.user, prog=%self.command.get_prog()))]
    async fn execute(&self, message: HandlerPayload) {
        let mut command = self.command.as_tokio_command();
        command
            .env("IMSE_USER", &self.user)
            .env("IMSE_EVENT", self.event.to_string());

        if let Some(message) = message {
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

        let start = Instant::now();
        tracing::info!("spawn");
        let result = command.status().await;
        if let Ok(result) = result {
            tracing::info!(elapsed_ms=%start.elapsed().as_millis(), rc=result.code().unwrap_or(-1), "complete");
        } else {
            tracing::error!(status=?result, "failure");
        }
    }

    pub fn into_sender_handle(self) -> (HandlerSender, tokio::task::JoinHandle<()>) {
        let (tx, rx) = watch::channel::<HandlerPayload>(None);

        let task = tokio::spawn(self.task(rx));

        (tx, task)
    }
}
