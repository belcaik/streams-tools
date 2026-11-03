use std::{
    collections::{HashSet, VecDeque},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::json;
use streams_tools_core::{Event, EventData, Platform, PluginData, User};

pub(crate) struct RecentIds {
    ids: HashSet<String>,
    order: VecDeque<String>,
}

impl RecentIds {
    pub(crate) fn new() -> Self {
        Self {
            ids: HashSet::new(),
            order: VecDeque::new(),
        }
    }

    pub(crate) fn insert(&mut self, id: String) -> bool {
        if !self.ids.insert(id.clone()) {
            return false;
        }
        self.order.push_back(id);
        if self.order.len() > 512
            && let Some(oldest) = self.order.pop_front()
        {
            self.ids.remove(&oldest);
        }
        true
    }
}

pub(crate) fn next_backoff(current: Duration) -> Duration {
    Duration::from_secs(match current.as_secs() {
        0 => 1,
        1 => 2,
        2 => 4,
        4 => 8,
        8 => 16,
        16 => 32,
        _ => 60,
    })
}

pub(crate) fn system(
    platform: Platform,
    prefix: &str,
    display_name: &str,
    status: &str,
    message: String,
) -> Event {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    Event {
        id: format!(
            "{prefix}-system-{}",
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ),
        platform,
        event: EventData::Plugin(PluginData {
            name: "system".into(),
            data: json!({"status": status, "message": message}),
        }),
        user: User {
            id: prefix.into(),
            name: display_name.into(),
            roles: Vec::new(),
        },
        ts: now_utc(),
    }
}

pub(crate) fn now_utc() -> String {
    format_utc(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO),
    )
}

pub(crate) fn format_utc(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    let days = (seconds / 86_400) as i64;
    let within_day = seconds % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    format!(
        "{year:04}-{:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        month,
        within_day / 3_600,
        (within_day / 60) % 60,
        within_day % 60,
        elapsed.subsec_millis()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_saturates_at_sixty_seconds() {
        let mut value = Duration::ZERO;
        let observed: Vec<_> = (0..8)
            .map(|_| {
                value = next_backoff(value);
                value.as_secs()
            })
            .collect();
        assert_eq!(observed, [1, 2, 4, 8, 16, 32, 60, 60]);
    }

    #[test]
    fn recent_ids_retains_the_latest_512() {
        let mut ids = RecentIds::new();
        assert!(ids.insert("same".into()));
        assert!(!ids.insert("same".into()));
        for index in 0..512 {
            assert!(ids.insert(format!("id-{index}")));
        }
        assert!(ids.insert("same".into()));
    }

    #[test]
    fn utc_conversion_handles_epoch_and_leap_date() {
        assert_eq!(format_utc(Duration::ZERO), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            format_utc(Duration::from_millis(1_582_934_423_456)),
            "2020-02-29T00:00:23.456Z"
        );
    }
}
