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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatData, EventData, Platform, User};

    fn event(id: &str) -> Event {
        Event {
            id: id.into(),
            platform: Platform::Twitch,
            event: EventData::Chat(ChatData {
                message: String::new(),
            }),
            user: User {
                id: String::new(),
                name: String::new(),
                roles: Vec::new(),
            },
            ts: String::new(),
        }
    }

    #[test]
    fn rejects_zero_capacity() {
        assert!(matches!(Bus::new(0), Err(InvalidCapacity)));
    }

    #[tokio::test]
    async fn publishes_to_multiple_subscribers() {
        let bus = Bus::new(4).expect("capacity should be valid");
        let mut first = bus.subscribe();
        let mut second = bus.subscribe();
        let event = event("1");

        assert_eq!(bus.publish(event.clone()).expect("subscribers exist"), 2);
        assert_eq!(first.recv().await.expect("event should arrive"), event);
        assert_eq!(second.recv().await.expect("event should arrive"), event);
    }

    #[tokio::test]
    async fn reports_lag_and_continues() {
        let bus = Bus::new(2).expect("capacity should be valid");
        let mut subscriber = bus.subscribe();
        for id in ["1", "2", "3"] {
            bus.publish(event(id)).expect("subscriber exists");
        }

        assert_eq!(subscriber.recv().await, Err(ReceiveError::Lagged(1)));
        assert_eq!(
            subscriber.recv().await.expect("event should arrive"),
            event("2")
        );
        assert_eq!(
            subscriber.recv().await.expect("event should arrive"),
            event("3")
        );
    }
}
