//! One disconnect signal per TCP connection.
//!
//! Actix keeps the handler future alive after the peer closes the socket.
//! `client_disconnect_timeout` only bounds how long the dispatcher waits; it
//! does not stop the statement. [`watch_http_client_disconnect`] installs one
//! [`HttpClientCancel`] on the connection. SQL execution selects on that token
//! around every statement — query, insert, update, transaction, and `CALL` —
//! and drops the statement future. A procedure runs inside that future, so the
//! same drop cancels the function worker. The disconnect check is a slow timer
//! on a duplicated socket, not a readability watch, so request bytes do not
//! wake it.

use std::any::Any;

use actix_web::dev::Extensions;
use tokio_util::sync::CancellationToken;

/// Cancellation signal for the TCP connection that carried an HTTP request.
#[derive(Clone)]
pub struct HttpClientCancel {
    token: CancellationToken,
}

impl HttpClientCancel {
    #[must_use]
    pub fn new(token: CancellationToken) -> Self {
        Self { token }
    }

    #[must_use]
    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }
}

/// `HttpServer::on_connect` callback. Parks until the peer closes the socket.
pub fn watch_http_client_disconnect(connection: &dyn Any, extensions: &mut Extensions) {
    let cancel = CancellationToken::new();
    extensions.insert(HttpClientCancel::new(cancel.clone()));
    #[cfg(unix)]
    watch_unix_disconnect(connection, cancel);
}

#[cfg(unix)]
fn watch_unix_disconnect(connection: &dyn Any, cancel: CancellationToken) {
    use std::{io::ErrorKind, time::Duration};

    let Some(stream) = connection.downcast_ref::<actix_web::rt::net::TcpStream>() else {
        return;
    };
    let Some(watcher) = duplicate_tcp_stream(stream) else {
        return;
    };
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    // Do not register the socket with the reactor. A readability watch wakes
    // this task on every request byte and peeks while Actix is reading and
    // writing the same connection. Hangup is checked on a slow timer instead.
    handle.spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_millis(20));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if peer_hangup(&watcher) {
                cancel.cancel();
                break;
            }
        }
    });

    /// `true` when the peer has closed the read half.
    ///
    /// The socket is nonblocking. `WouldBlock` means Actix already took the
    /// bytes, which is a live connection.
    fn peer_hangup(stream: &tokio::net::TcpStream) -> bool {
        use std::os::unix::io::AsRawFd;

        let fd = stream.as_raw_fd();
        let mut buf = [0u8; 1];
        loop {
            // SAFETY: `fd` is the duplicate socket owned by `stream`. `MSG_PEEK`
            // does not remove bytes from the buffer Actix is reading.
            let n = unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), 1, libc::MSG_PEEK) };
            if n == 0 {
                return true;
            }
            if n > 0 {
                return false;
            }
            let err = std::io::Error::last_os_error();
            if err.kind() == ErrorKind::Interrupted {
                continue;
            }
            return err.kind() != ErrorKind::WouldBlock;
        }
    }
}

#[cfg(unix)]
fn duplicate_tcp_stream(stream: &actix_web::rt::net::TcpStream) -> Option<tokio::net::TcpStream> {
    use std::os::unix::io::{AsRawFd, FromRawFd};

    let fd = stream.as_raw_fd();
    // SAFETY: `fd` is the accepted socket Actix still owns. `dup` creates a new
    // descriptor for that same connection and does not close `fd`.
    let duped = unsafe { libc::dup(fd) };
    if duped < 0 {
        return None;
    }
    // SAFETY: `duped` is a fresh descriptor owned only here. Dropping the
    // resulting stream closes that duplicate, not Actix's original socket.
    let std_stream = unsafe { std::net::TcpStream::from_raw_fd(duped) };
    if std_stream.set_nonblocking(true).is_err() {
        return None;
    }
    tokio::net::TcpStream::from_std(std_stream).ok()
}
