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
pub struct EventHandler {
    rx: mpsc::Receiver<Event>,
    _tx: mpsc::Sender<Event>,
}

impl EventHandler {
    /// Create new event handler with tick rate.
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
        let tx_clone = tx.clone();

        // Spawn async event polling task using EventStream (non-blocking)
        tokio::spawn(async move {
            let mut reader = EventStream::new();
            let mut interval = tokio::time::interval(tick_rate);

            loop {
                tokio::select! {
                    maybe_event = reader.next() => {
                        match maybe_event {
                            Some(Ok(CrosstermEvent::Key(key))) => {
                                if tx_clone.send(Event::Key(key)).await.is_err() {
                                    break;
                                }
                            }
                            Some(Ok(CrosstermEvent::Resize(w, h))) => {
                                if tx_clone.send(Event::Resize(w, h)).await.is_err() {
                                    break;
                                }
                            }
                            Some(Err(e)) => {
                                tracing::warn!(error = %e, "crossterm event stream error, stopping event loop");
                                break;
                            }
                    None => break,
                            _ => {}
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

        Self { rx, _tx: tx }
    }

    /// Get next event, or `None` if the event channel has closed.
    pub async fn next(&mut self) -> Option<Event> {
        self.rx.recv().await
    }
}
