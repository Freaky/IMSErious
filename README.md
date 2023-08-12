# IMSErious

Execute commands in response to Dovecot [Internet Message Store Events][rfc5423].

## Synopsis

```
imserious [-t] [-c file]
imserious [--test] [--config file]
imserious [-hv]
imserious [--help] [--version]
```

```
Optional arguments:
  -h, --help           print help message
  -v, --version        print program version
  -t, --test           test configuration
  -c, --config CONFIG  path to configuration
```

## Summary

IMSErious is a service that listens for Dovecot push notification events,
as sent by its [OX (Open-Xchange) driver][OX], and executes commands in response.
This allows, for example, waking up an [MRA] or issuing desktop notifications
on new messages.

## Configuration

IMSErious is configured from a [TOML] file specified as the first argument,
defaulting to `/usr/local/etc/imserious.toml`:

```toml
listen = "10.0.0.1:12525"  # listen address, default 127.0.0.1:12525
allow = [ "10.0.0.2/32" ]  # allowed notification IP ranges, default all
endpoint = "/notify"       # path to API endpoint, default /notify
max_connections = 8        # connection limit, default 8
timeout = "5s"             # request timeout, default 5s

# optional Basic auth
[auth]
user = "foo"
pass = "bar"

# optional TLS
[tls]
cert = "/etc/ssl/foo.example.com.crt"
key = "/etc/ssl/foo.example.com.key"
periodic_reload = "1d" # optionally reload keys periodically, no default

# optional stdout logging
[log]
max_level = "info"    # One of error, warn, info (default), debug, trace
                      # May be overridden by setting IMSERIOUS_LOG env var
format = "compact"    # One of full (default), compact, pretty, json
ansi = false          # Format "pretty" with ANSI codes, default false
timestamp = false     # Display a timestamp, default false
target = false        # Display the log target, default false
level = false         # Display the log level, default false

[[handler]]
ip = [ "10.0.0.2/32" ] # allowed handler IP ranges, default all
user = "freaky"        # Username, required
event = "MessageNew"   # Event type, optional, default MessageNew
                       # Note this is currently the only type supported by Dovecot's OX driver
delay = "5s"           # Delay execution this long after initial event, optional, default none
limit_period = "30s"   # Rate limit executions over this interval, optional, default 30s
limit_burst = 1        # Allow this many executions per interval, optional, default 1
periodic = "300s"      # Execute unconditionally after this long, optional, default none
command = "/usr/local/bin/fdm -a eda -l fetch"
```

## Handlers

A handler is a command to execute in response to a specific event/user pair.  Multiple
handlers for the same event and user may be specified to trigger different commands
with their own rate limits, periodic configuration, etc.

Commands only support basic shell word splitting and quoting - if shell metacharacters
are required they should be provided by executing via a shell such as with `/bin/sh -c`.

Event fields will be exposed in `IMSE_*` env vars if available - only `IMSE_USER`
and `IMSE_EVENT`are guaranteed to be set if `periodic` execution is specified.

* `IMSE_USER` - user being notified
* `IMSE_EVENT` - event name
* `IMSE_REMOTE_IP` - notifying IP address
* `IMSE_REMOTE_PORT` - notifying TCP port
* `IMSE_UNSEEN` - number of unseen messages
* `IMSE_FOLDER` - IMAP folder name
* `IMSE_FROM` - `From:` address of a new email (if any)
* `IMSE_SNIPPET` - a sample of the body of a new email (if any)

## Security

It should not need to be said that there are potentially serious security implications
from allowing remote clients to trigger commands on your server.  While every effort
has been made to limit the potential for harm, it is your responsibility not to use
this program unsafely.

It is strongly discouraged to run an open instance of IMSErious on a public network,
or as a privileged user.

[rfc5423]: https://www.rfc-editor.org/rfc/rfc5423.html
[OX]: https://doc.dovecot.org/configuration_manual/push_notification/
[MRA]: https://en.wikipedia.org/wiki/Mail_retrieval_agent
[TOML]: https://toml.io
