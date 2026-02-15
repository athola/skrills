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

/// Async event handler.
pub struct EventHandler {
    rx: mpsc::UnboundedReceiver<Event>,
    _tx: mpsc::UnboundedSender<Event>,
}

impl EventHandler {
    /// Create new event handler with tick rate.
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
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
                                if tx_clone.send(Event::Key(key)).is_err() {
                                    break;
                                }
                            }
                            Some(Ok(CrosstermEvent::Resize(w, h))) => {
                                if tx_clone.send(Event::Resize(w, h)).is_err() {
                                    break;
                                }
                            }
                            Some(Err(_)) | None => break,
                            _ => {}
                        }
                    }
                    _ = interval.tick() => {
                        if tx_clone.send(Event::Tick).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Self { rx, _tx: tx }
    }

    /// Get next event.
    pub async fn next(&mut self) -> Event {
        self.rx.recv().await.unwrap_or(Event::Tick)
    }
}
