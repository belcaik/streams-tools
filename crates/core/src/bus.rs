use std::{error::Error, fmt};

use tokio::sync::broadcast;

use crate::Event;

#[derive(Debug, Clone)]
pub struct Bus {
    sender: broadcast::Sender<Event>,
}

#[derive(Debug)]
pub struct Subscriber {
    receiver: broadcast::Receiver<Event>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidCapacity;

impl fmt::Display for InvalidCapacity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("event bus capacity must be greater than zero")
    }
}

impl Error for InvalidCapacity {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveError {
    Lagged(u64),
    Closed,
}

impl fmt::Display for ReceiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lagged(skipped) => write!(formatter, "subscriber lagged by {skipped} events"),
            Self::Closed => formatter.write_str("event bus is closed"),
        }
    }
}

impl Error for ReceiveError {}

impl Bus {
    pub fn new(capacity: usize) -> Result<Self, InvalidCapacity> {
        if capacity == 0 {
            return Err(InvalidCapacity);
        }
        let (sender, _) = broadcast::channel(capacity);
        Ok(Self { sender })
    }

    pub fn subscribe(&self) -> Subscriber {
        Subscriber {
            receiver: self.sender.subscribe(),
        }
    }

    // Preserve Tokio's send error so callers can recover the unpublished event.
    #[expect(clippy::result_large_err)]
    pub fn publish(&self, event: Event) -> Result<usize, broadcast::error::SendError<Event>> {
        self.sender.send(event)
    }
}

impl Subscriber {
    pub async fn recv(&mut self) -> Result<Event, ReceiveError> {
        self.receiver.recv().await.map_err(|error| match error {
            broadcast::error::RecvError::Closed => ReceiveError::Closed,
            broadcast::error::RecvError::Lagged(skipped) => ReceiveError::Lagged(skipped),
        })
    }
}
