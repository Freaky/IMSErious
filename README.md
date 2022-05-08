# IMSErious

Execute commands in response to [Internet Message Store Events](rfc5423), as
sent by Dovecot's [push notifications](XO) Open-Xchange driver.

This was written specifically to automatically trigger an IMAP IDLE-unaware
[MRA] on new messages, but could also be used for desktop/mobile notifications,
or whatever else you could do by executing commands.

IMSErious is configured from a [TOML] file specified as the first argument,
or defaulting to `/usr/local/etc/imserious.toml`:

```toml
listen = "10.0.0.1:12345" # optional listen address, default to localhost:12525
allow = [ "10.0.0.2/32" ] # optional allowed notification IP ranges

# optional Basic auth
[auth]
user = "foo"
pass = "bar"

# optional TLS
[tls]
cert = "/etc/ssl/foo.example.com.crt"
key = "/etc/ssl/foo.example.com.key"

[[handler]]
event = "MessageNew" # Event type, required
user = "freaky"      # Username, required
min_delay = "30s"    # Minimum duration between command executions, required
max_delay = "300s"   # Maximum duration between command executions, optional
command = "/usr/local/bin/fdm -a eda -l fetch"
```

The `[[handler]]` section may be repeated for multiple users and events, and
even repeated for the same user and event to trigger multiple programs on different
schedules.

In this example a `MessageNew` notification for user `freaky` will trigger `fdm` to
fetch email from the named account `eda` at most every 30 seconds, and will trigger
every 300 seconds regardless of whether there has been an event or not.

`command` does not support shell metacharacters other than basic word splitting and
quotes, though you could execute a command via a shell such as `/bin/sh -c` to
gain this.  Please do so with care.

Event fields will be exposed in `IMSE_*` env vars if available - only `IMSE_USER`
and `IMSE_EVENT`are guaranteed to be set if max_delay is specified.

* `IMSE_USER` - user being notified
* `IMSE_EVENT` - event name
* `IMSE_REMOTE_IP` - notifying IP address
* `IMSE_REMOTE_PORT` - notifying TCP port
* `IMSE_UNSEEN` - number of unseen messages
* `IMSE_FOLDER` - IMAP folder name
* `IMSE_FROM` - `From:` address of a new email (if any)
* `IMSE_SNIPPET` - a sample of the body of a new email (if any)

[rfc5423]: https://www.rfc-editor.org/rfc/rfc5423.html
[OX]: https://doc.dovecot.org/configuration_manual/push_notification/
[MRA]: https://en.wikipedia.org/wiki/Mail_retrieval_agent
[TOML]: https://toml.io