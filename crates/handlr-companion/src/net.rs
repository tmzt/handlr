//! The `net` thread: a Tokio current-thread runtime that keeps a connection
//! to the daemon's socket, reconnects with backoff, and drives
//! [`State`](crate::client::State) (plan 20261022_04, decision 2, and the
//! 18:10 EDT amendment's step 1).

use std::sync::mpsc;
use std::time::Duration;

use libhandlr_proto::Incoming;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::UnixStream;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::client::{Event, Input, Output, State};
use crate::keys::KeyStore;
use crate::CompanionConfig;

/// The longest line read from the daemon (the daemon's own limit).
const MAX_LINE: usize = 1 << 20;

/// Delays between reconnects: `initial`, doubling up to `max` (1, 2, 4 …
/// 30 s by default), back to `initial` once a connection says hello.
#[derive(Debug, Clone)]
pub struct Backoff {
    initial: Duration,
    max: Duration,
    next: Duration,
}

impl Backoff {
    pub fn new(initial: Duration, max: Duration) -> Self {
        Backoff {
            initial,
            max,
            next: initial,
        }
    }

    /// The delay before the next attempt.
    pub fn next_delay(&mut self) -> Duration {
        let delay = self.next;
        self.next = (self.next * 2).min(self.max);
        delay
    }

    pub fn reset(&mut self) {
        self.next = self.initial;
    }
}

/// Runs the thread until the app drops its [`Companion`](crate::Companion).
pub(crate) fn run(
    config: CompanionConfig,
    keys: Box<dyn KeyStore>,
    inputs: UnboundedReceiver<Input>,
    events: mpsc::Sender<Event>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            let _ = events.send(Event::Error(format!("starting the net runtime: {e}")));
            return;
        }
    };
    let net = Net {
        state: State::new(config.client.clone()),
        backoff: Backoff::new(config.backoff_initial, config.backoff_max),
        config,
        _keys: keys,
        inputs,
        events,
    };
    runtime.block_on(net.run());
}

struct Net {
    config: CompanionConfig,
    /// Used by the link (plan 20261022_04, step 2).
    _keys: Box<dyn KeyStore>,
    state: State,
    backoff: Backoff,
    inputs: UnboundedReceiver<Input>,
    events: mpsc::Sender<Event>,
}

/// How a session ended.
enum End {
    /// The connection closed; reconnect.
    Closed,
    /// The app is gone; stop.
    AppGone,
}

impl Net {
    async fn run(mut self) {
        loop {
            match UnixStream::connect(&self.config.socket).await {
                Ok(stream) => {
                    log::info!("connected to {}", self.config.socket.display());
                    if let End::AppGone = self.session(stream).await {
                        return;
                    }
                }
                Err(e) => log::debug!("connecting to {}: {e}", self.config.socket.display()),
            }
            if self.state.online() {
                self.backoff.reset();
            }
            let delay = self.backoff.next_delay();
            let retry_in_ms = u32::try_from(delay.as_millis()).unwrap_or(u32::MAX);
            let outputs = self.state.disconnected(retry_in_ms);
            self.emit_offline(outputs);
            let sleep = tokio::time::sleep(delay);
            tokio::pin!(sleep);
            loop {
                tokio::select! {
                    _ = &mut sleep => break,
                    input = self.inputs.recv() => match input {
                        Some(input) => {
                            let outputs = self.state.input(input);
                            self.emit_offline(outputs);
                        }
                        None => return,
                    },
                }
            }
        }
    }

    /// Gives events while there is no connection. The state sends nothing
    /// while offline, so anything else is only logged.
    fn emit_offline(&self, outputs: Vec<Output>) {
        for output in outputs {
            match output {
                Output::Event(e) => {
                    let _ = self.events.send(e);
                }
                other => log::warn!("nothing to send it on: {other:?}"),
            }
        }
    }

    async fn session(&mut self, stream: UnixStream) -> End {
        let (read, mut write) = stream.into_split();
        let (lines_tx, mut lines) = tokio::sync::mpsc::unbounded_channel();
        let reader = tokio::spawn(async move {
            let mut read = BufReader::new(read);
            loop {
                let mut line = Vec::new();
                let limit = (MAX_LINE + 1) as u64;
                match (&mut read).take(limit).read_until(b'\n', &mut line).await {
                    Ok(0) => break,
                    Ok(_) if line.len() > MAX_LINE => {
                        log::warn!("a line from the daemon is over {MAX_LINE} bytes");
                        break;
                    }
                    Ok(_) => {
                        if lines_tx.send(line).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        log::debug!("reading from the daemon: {e}");
                        break;
                    }
                }
            }
        });
        let outputs = self.state.connected();
        let mut end = self.carry_out(outputs, &mut write).await;
        while end.is_none() {
            tokio::select! {
                line = lines.recv() => match line {
                    Some(line) => match serde_json::from_slice::<Incoming>(&line) {
                        Ok(message) => {
                            let outputs = self.state.incoming(message);
                            end = self.carry_out(outputs, &mut write).await;
                        }
                        Err(e) => log::warn!("a line from the daemon is not JSON-RPC: {e}"),
                    },
                    None => end = Some(End::Closed),
                },
                input = self.inputs.recv() => match input {
                    Some(input) => {
                        let outputs = self.state.input(input);
                        end = self.carry_out(outputs, &mut write).await;
                    }
                    None => end = Some(End::AppGone),
                },
            }
        }
        reader.abort();
        end.unwrap_or(End::Closed)
    }

    /// Carries out the state's outputs; `Some` when the session must end.
    async fn carry_out(&self, outputs: Vec<Output>, write: &mut OwnedWriteHalf) -> Option<End> {
        for output in outputs {
            match output {
                Output::Event(e) => {
                    let _ = self.events.send(e);
                }
                Output::Send(request) => {
                    let mut line = match serde_json::to_vec(&request) {
                        Ok(line) => line,
                        Err(e) => {
                            log::error!("serializing {}: {e}", request.method);
                            continue;
                        }
                    };
                    line.push(b'\n');
                    if let Err(e) = write.write_all(&line).await {
                        log::debug!("writing to the daemon: {e}");
                        return Some(End::Closed);
                    }
                }
                Output::Close(reason) => {
                    log::warn!("closing the connection: {reason}");
                    return Some(End::Closed);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_to_30_s_and_resets() {
        let mut b = Backoff::new(Duration::from_secs(1), Duration::from_secs(30));
        let secs: Vec<u64> = (0..7).map(|_| b.next_delay().as_secs()).collect();
        assert_eq!(secs, vec![1, 2, 4, 8, 16, 30, 30]);
        b.reset();
        assert_eq!(b.next_delay(), Duration::from_secs(1));
    }
}
