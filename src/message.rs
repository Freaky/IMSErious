use derive_more::Display;
use serde::Deserialize;

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

#[derive(Copy, Clone, Deserialize, Debug, Display, Hash, PartialEq, Eq)]
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
