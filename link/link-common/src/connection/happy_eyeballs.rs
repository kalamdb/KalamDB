//! Happy Eyeballs (RFC 8305) TCP connect for WebSocket dialing.
//!
//! `tokio-tungstenite::connect_async` tries resolved addresses sequentially.
//! On mobile networks a broken IPv6 route commonly stalls ~1–3s before IPv4
//! is attempted. Racing families with a short delay avoids that wait.

use std::{
    io::{Error as IoError, ErrorKind},
    net::SocketAddr,
    time::Duration,
};

use tokio::{
    net::TcpStream,
    task::JoinSet,
    time::{sleep_until, Instant as TokioInstant},
};

/// RFC 8305 connection attempt delay.
pub(crate) const HAPPY_EYEBALLS_ATTEMPT_DELAY: Duration = Duration::from_millis(250);

/// Connect to the first address that succeeds, starting the next family after
/// [HAPPY_EYEBALLS_ATTEMPT_DELAY] or immediately if the previous attempt fails.
pub(crate) async fn connect_tcp_happy_eyeballs(addrs: &[SocketAddr]) -> std::io::Result<TcpStream> {
    let addrs = interleave_address_families(addrs);
    if addrs.is_empty() {
        return Err(IoError::new(
            ErrorKind::AddrNotAvailable,
            "No resolved addresses for Happy Eyeballs connect",
        ));
    }

    if addrs.len() == 1 {
        return connect_one(addrs[0]).await;
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<std::io::Result<TcpStream>>();
    let mut join_set = JoinSet::new();
    let mut remaining = addrs.into_iter();
    let mut in_flight = 0usize;
    let mut last_err: Option<IoError> = None;
    let attempt_delay = happy_eyeballs_delay(remaining.as_slice());

    if attempt_delay.is_zero() {
        for addr in remaining.by_ref() {
            spawn_attempt(&mut join_set, tx.clone(), addr, &mut in_flight);
        }
    } else if let Some(addr) = remaining.next() {
        spawn_attempt(&mut join_set, tx.clone(), addr, &mut in_flight);
    }

    let mut next_stagger = TokioInstant::now() + attempt_delay;
    let mut stagger_armed = !attempt_delay.is_zero() && !remaining.as_slice().is_empty();

    loop {
        tokio::select! {
            biased;

            result = rx.recv() => {
                in_flight = in_flight.saturating_sub(1);
                match result {
                    Some(Ok(stream)) => {
                        join_set.abort_all();
                        return Ok(stream);
                    },
                    Some(Err(err)) => {
                        last_err = Some(err);
                        if let Some(addr) = remaining.next() {
                            spawn_attempt(&mut join_set, tx.clone(), addr, &mut in_flight);
                            next_stagger = TokioInstant::now() + attempt_delay;
                            stagger_armed = !remaining.as_slice().is_empty();
                        } else if in_flight == 0 {
                            break;
                        }
                    },
                    None => break,
                }
            }

            _ = sleep_until(next_stagger), if stagger_armed => {
                if let Some(addr) = remaining.next() {
                    spawn_attempt(&mut join_set, tx.clone(), addr, &mut in_flight);
                    next_stagger = TokioInstant::now() + attempt_delay;
                    stagger_armed = !remaining.as_slice().is_empty();
                } else {
                    stagger_armed = false;
                    if in_flight == 0 {
                        break;
                    }
                }
            }
        }
    }

    Err(last_err.unwrap_or_else(|| {
        IoError::new(ErrorKind::AddrNotAvailable, "Happy Eyeballs failed to connect")
    }))
}

fn spawn_attempt(
    join_set: &mut JoinSet<()>,
    tx: tokio::sync::mpsc::UnboundedSender<std::io::Result<TcpStream>>,
    addr: SocketAddr,
    in_flight: &mut usize,
) {
    join_set.spawn(async move {
        let _ = tx.send(connect_one(addr).await);
    });
    *in_flight += 1;
}

async fn connect_one(addr: SocketAddr) -> std::io::Result<TcpStream> {
    let stream = TcpStream::connect(addr).await?;
    let _ = stream.set_nodelay(true);
    Ok(stream)
}

fn happy_eyeballs_delay(addrs: &[SocketAddr]) -> Duration {
    if !addrs.is_empty() && addrs.iter().all(|addr| addr.ip().is_loopback()) {
        Duration::ZERO
    } else {
        HAPPY_EYEBALLS_ATTEMPT_DELAY
    }
}

/// Interleave IPv6 and IPv4, starting with whichever family DNS returned first.
pub(crate) fn interleave_address_families(addrs: &[SocketAddr]) -> Vec<SocketAddr> {
    let mut v6 = Vec::new();
    let mut v4 = Vec::new();
    for addr in addrs {
        if addr.is_ipv6() {
            v6.push(*addr);
        } else {
            v4.push(*addr);
        }
    }

    let prefer_v6 = addrs.first().is_some_and(SocketAddr::is_ipv6);
    let (mut first, mut second) = if prefer_v6 {
        (v6.into_iter(), v4.into_iter())
    } else {
        (v4.into_iter(), v6.into_iter())
    };

    let mut out = Vec::with_capacity(addrs.len());
    loop {
        match (first.next(), second.next()) {
            (Some(a), Some(b)) => {
                out.push(a);
                out.push(b);
            },
            (Some(a), None) => {
                out.push(a);
                out.extend(first);
                break;
            },
            (None, Some(b)) => {
                out.push(b);
                out.extend(second);
                break;
            },
            (None, None) => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::{
        net::{Ipv4Addr, Ipv6Addr, SocketAddr},
        time::Duration,
    };

    use super::{
        connect_tcp_happy_eyeballs, happy_eyeballs_delay, interleave_address_families,
        HAPPY_EYEBALLS_ATTEMPT_DELAY,
    };

    fn v4(port: u16) -> SocketAddr {
        SocketAddr::from((Ipv4Addr::LOCALHOST, port))
    }

    fn v6(port: u16) -> SocketAddr {
        SocketAddr::from((Ipv6Addr::LOCALHOST, port))
    }

    #[test]
    fn interleave_starts_with_first_dns_family() {
        let addrs = [v4(1), v6(2), v4(3), v6(4)];
        assert_eq!(interleave_address_families(&addrs), vec![v4(1), v6(2), v4(3), v6(4)]);

        let addrs = [v6(1), v4(2), v6(3), v4(4)];
        assert_eq!(interleave_address_families(&addrs), vec![v6(1), v4(2), v6(3), v4(4)]);
    }

    #[test]
    fn interleave_does_not_starve_the_other_family() {
        let addrs = [v6(1), v6(2), v6(3), v4(9)];
        assert_eq!(interleave_address_families(&addrs), vec![v6(1), v4(9), v6(2), v6(3)]);
    }

    #[test]
    fn loopback_addresses_are_not_staggered() {
        assert_eq!(happy_eyeballs_delay(&[v4(1), v6(2)]), Duration::ZERO);
        assert_eq!(
            happy_eyeballs_delay(&[SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 443))]),
            HAPPY_EYEBALLS_ATTEMPT_DELAY
        );
    }

    #[tokio::test]
    async fn happy_eyeballs_uses_working_address_after_refused() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ok_addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        let refused = SocketAddr::from((Ipv4Addr::LOCALHOST, 1));
        let stream = connect_tcp_happy_eyeballs(&[refused, ok_addr])
            .await
            .expect("should fall back to the listening address");
        assert_eq!(stream.peer_addr().unwrap(), ok_addr);
    }
}
