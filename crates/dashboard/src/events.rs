//! Event handling for the dashboard.

use std::time::Duration;

use crossterm::event::{Event as CrosstermEvent, EventStream, KeyEvent};
use futures::StreamExt;
use tokio::sync::mpsc;

/// Dashboard events.
#[derive(Debug, Clone)]
pub enum Event {
    /// Keyboard input.
    Key(KeyEvent),
    /// Terminal resize.
    Resize(u16, u16),
    /// Tick for periodic updates.
    Tick,
}

/// Capacity for the event channel.
/// 256 entries is enough for burst input without unbounded growth.
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// Async event handler with bounded backpressure.
///
/// The reader task owns the only sender, so when it stops (a terminal read
/// error) the channel closes and `next` returns `None` instead of waiting.
pub struct EventHandler {
    rx: mpsc::Receiver<Event>,
    reader: tokio::task::JoinHandle<()>,
}

impl EventHandler {
    /// Create new event handler with tick rate.
    pub fn new(tick_rate: Duration) -> Self {
        Self::from_stream(EventStream::new(), tick_rate)
    }

    /// Feeds key and resize events from `reader`, plus a tick every
    /// `tick_rate`, into the channel `next` reads.
    fn from_stream<S>(mut reader: S, tick_rate: Duration) -> Self
    where
        S: futures::Stream<Item = std::io::Result<CrosstermEvent>> + Unpin + Send + 'static,
    {
        let (tx_clone, rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);

        let reader = tokio::spawn(async move {
            let mut interval = tokio::time::interval(tick_rate);

            loop {
                tokio::select! {
                    maybe_event = reader.next() => {
                        let event = match maybe_event {
                            Some(Ok(CrosstermEvent::Key(key))) => Event::Key(key),
                            Some(Ok(CrosstermEvent::Resize(w, h))) => Event::Resize(w, h),
                            Some(Ok(_)) => continue,
                            Some(Err(e)) => {
                                tracing::warn!(error = %e, "crossterm event stream error, stopping event loop");
                                break;
                            }
                            None => break,
                        };
                        if tx_clone.send(event).await.is_err() {
                            break;
                        }
                    }
                    _ = interval.tick() => {
                        if tx_clone.send(Event::Tick).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Self { rx, reader }
    }

    /// Get next event, or `None` if the event channel has closed.
    pub async fn next(&mut self) -> Option<Event> {
        self.rx.recv().await
    }
}

impl Drop for EventHandler {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// After a terminal read error the reader task stops. The handler used to
    /// keep a sender of its own, so `next()` waited forever and, with raw mode
    /// swallowing Ctrl+C, the dashboard hung.
    #[tokio::test]
    async fn next_returns_none_after_the_reader_stops_on_an_error() {
        let failing = futures::stream::iter([Err(std::io::Error::other("tty gone"))]);
        let mut handler = EventHandler::from_stream(failing, Duration::from_secs(3600));
        let drained = tokio::time::timeout(Duration::from_secs(5), async {
            while handler.next().await.is_some() {}
        })
        .await;
        assert!(drained.is_ok(), "next() never returned None");
    }

    #[tokio::test]
    async fn keys_resizes_and_ticks_all_reach_next() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let terminal = futures::stream::iter([
            Ok(CrosstermEvent::Key(KeyEvent::new(
                KeyCode::Char('x'),
                KeyModifiers::NONE,
            ))),
            Ok(CrosstermEvent::FocusGained),
            Ok(CrosstermEvent::Resize(80, 24)),
        ])
        .chain(futures::stream::pending());
        let mut handler = EventHandler::from_stream(terminal, Duration::from_millis(10));
        let (mut key, mut resize, mut tick) = (false, false, false);
        let all_seen = tokio::time::timeout(Duration::from_secs(5), async {
            while !(key && resize && tick) {
                match handler.next().await {
                    Some(Event::Key(k)) => key |= k.code == KeyCode::Char('x'),
                    Some(Event::Resize(80, 24)) => resize = true,
                    Some(Event::Tick) => tick = true,
                    other => panic!("unexpected event {other:?}"),
                }
            }
        })
        .await;
        assert!(all_seen.is_ok(), "key={key} resize={resize} tick={tick}");
    }
}
