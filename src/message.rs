use serde::Deserialize;
use strum::{Display, EnumString};

#[derive(Deserialize, Clone, Debug)]
pub struct ImseMessage {
    #[serde(skip)]
    pub remote_addr: Option<std::net::SocketAddr>,
    pub event: ImseEvent,
    pub user: String,
    pub unseen: u32,
    pub folder: String,
    pub from: Option<String>,
    pub snippet: Option<String>,
}

#[derive(Copy, Clone, Debug, Display, Deserialize, Hash, PartialEq, Eq, EnumString)]
#[strum(ascii_case_insensitive)]
#[serde(try_from = "&str")]
pub enum ImseEvent {
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
