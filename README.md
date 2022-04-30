# IMSErious

Execute commands in response to [Internet Message Store Events](rfc5423), as
sent by Dovecot's [push notifications](XO) Open-Xchange driver.

This was written specifically to automatically trigger an IMAP IDLE-unaware
[MRA] on new messages, but could also be used for desktop/mobile notifications
or anything else you could do by executing a command in response to a web
request.

Currently IMSErious is hardcoded to load a config from `/usr/local/etc/imserious.toml`
of the form:

```toml
listen = "127.0.0.1:12345"

[[handler]]
event = "MessageNew"
user = "freaky"
min_delay = "30s"
max_delay = "300s" # infinite if not specified
command = "/usr/local/bin/fdm -a eda -l fetch"
```

The `[[handler]]` section may be repeated for multiple users and events. In this
case a `MessageNew` notification for user `freaky` will trigger `fdm` to fetch
email from the named account `eda` at most every 30 seconds, and will trigger every
300 seconds regardless of whether there has been an event or not.

`command` does not support shell metacharacters other than basic word splitting and
quotes.

Event fields will be exposed in `IMSE_*` env vars if available.

[rfc5423]: https://www.rfc-editor.org/rfc/rfc5423.html
[OX]: https://doc.dovecot.org/configuration_manual/push_notification/
[MRA]: https://en.wikipedia.org/wiki/Mail_retrieval_agent