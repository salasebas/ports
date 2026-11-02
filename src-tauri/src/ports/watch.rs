use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::health::HealthState;
use super::rules::{self, WatchEvent};
use super::sampler::Server;

pub const SETTLE_MS: u64 = 5_000;

#[derive(Debug, PartialEq)]
pub struct Notice {
    pub key: String,
    pub server_id: String,
    pub title: String,
    pub body: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
struct Owner {
    title: String,
}

#[derive(Debug, Clone, PartialEq)]
struct Observed {
    owners: BTreeMap<String, Owner>,
    answering: Option<bool>,
}

impl Observed {
    fn of(key: &str, servers: &[Server]) -> Self {
        let watched = || {
            servers
                .iter()
                .filter(|server| rules::watched_by(key, server))
        };
        let any = |state| watched().any(|server| server.health.state == state);
        Self {
            owners: watched()
                .map(|server| {
                    let title = format!("{} on :{}", server.name, server.port);
                    (server.id.clone(), Owner { title })
                })
                .collect(),
            answering: if any(HealthState::TimedOut) {
                Some(false)
            } else if any(HealthState::Responsive) {
                Some(true)
            } else {
                None
            },
        }
    }

    fn ids(&self) -> BTreeSet<&String> {
        self.owners.keys().collect()
    }

    fn same_as(&self, other: &Self) -> bool {
        self.ids() == other.ids() && self.answering == other.answering
    }

    fn keeping_last_known_health(mut self, reported: &Self) -> Self {
        if !self.owners.is_empty() && !self.ids().is_disjoint(&reported.ids()) {
            self.answering = self.answering.or(reported.answering);
        }
        self
    }

    fn first(&self) -> Option<(&String, &Owner)> {
        self.owners.iter().next()
    }
}

struct Seen {
    reported: Observed,
    pending: Option<(Observed, u64)>,
}

#[derive(Default)]
pub struct Watches {
    seen: HashMap<String, Seen>,
}

impl Watches {
    pub fn due(
        &mut self,
        watches: &BTreeMap<String, Vec<WatchEvent>>,
        servers: &[Server],
        now: u64,
    ) -> Vec<Notice> {
        self.seen.retain(|key, _| watches.contains_key(key));
        let mut notices = Vec::new();
        for (key, events) in watches {
            let current = Observed::of(key, servers);
            let Some(seen) = self.seen.get_mut(key) else {
                let seen = Seen {
                    reported: current,
                    pending: None,
                };
                self.seen.insert(key.clone(), seen);
                continue;
            };
            let current = current.keeping_last_known_health(&seen.reported);
            if current.same_as(&seen.reported) {
                seen.pending = None;
                continue;
            }
            match &seen.pending {
                Some((pending, since)) if pending.same_as(&current) => {
                    if now.saturating_sub(*since) >= SETTLE_MS {
                        notices.extend(changes(key, events, &seen.reported, &current));
                        seen.reported = current;
                        seen.pending = None;
                    }
                }
                _ => seen.pending = Some((current, now)),
            }
        }
        notices
    }

    pub fn settling(&self) -> bool {
        self.seen.values().any(|seen| seen.pending.is_some())
    }
}

fn changes(key: &str, events: &[WatchEvent], before: &Observed, after: &Observed) -> Vec<Notice> {
    let notice = |(id, owner): (&String, &Owner), body| Notice {
        key: key.to_string(),
        server_id: id.clone(),
        title: owner.title.clone(),
        body,
    };
    let mut notices = Vec::new();
    match (before.first(), after.first()) {
        (None, Some(owner)) if events.contains(&WatchEvent::Started) => {
            notices.push(notice(owner, "Started."));
        }
        (Some(owner), None) if events.contains(&WatchEvent::Stopped) => {
            notices.push(notice(owner, "Stopped. It's no longer running."));
        }
        (Some(_), Some(owner))
            if events.contains(&WatchEvent::Started) && before.ids().is_disjoint(&after.ids()) =>
        {
            notices.push(notice(owner, "A new process replaced the previous one."));
        }
        _ => {}
    }
    if let (Some(owner), true) = (after.first(), events.contains(&WatchEvent::Health)) {
        match (before.answering, after.answering) {
            (Some(true), Some(false)) => notices.push(notice(owner, "Stopped responding.")),
            (Some(false), Some(true)) => notices.push(notice(owner, "Responding again.")),
            _ => {}
        }
    }
    notices
}

#[cfg(test)]
mod tests {
    use super::super::alerts::tests::server;
    use super::super::health::Health;
    use super::*;

    const ALL: [WatchEvent; 3] = [WatchEvent::Started, WatchEvent::Stopped, WatchEvent::Health];
    const KEY: &str = "port:3000";

    fn rules(events: &[WatchEvent]) -> BTreeMap<String, Vec<WatchEvent>> {
        BTreeMap::from([(KEY.to_string(), events.to_vec())])
    }

    fn web(id: &str, state: HealthState) -> Server {
        let mut server = server(3000, 0);
        server.id = id.into();
        server.health = Health {
            state,
            checked_at: None,
        };
        server
    }

    fn settled(
        watches: &mut Watches,
        rules: &[WatchEvent],
        servers: &[Server],
        at: u64,
    ) -> Vec<&'static str> {
        let rules = self::rules(rules);
        assert_eq!(watches.due(&rules, servers, at), Vec::new());
        watches
            .due(&rules, servers, at + SETTLE_MS)
            .into_iter()
            .map(|notice| notice.body)
            .collect()
    }

    #[test]
    fn what_is_already_running_when_a_watch_starts_is_not_news() {
        let mut watches = Watches::default();
        let running = [web("a", HealthState::Responsive)];
        assert_eq!(settled(&mut watches, &ALL, &running, 0), Vec::<&str>::new());
    }

    #[test]
    fn reports_a_watched_start_and_stop_once() {
        let mut watches = Watches::default();
        settled(&mut watches, &ALL, &[], 0);
        let running = [web("a", HealthState::Unknown)];
        assert_eq!(settled(&mut watches, &ALL, &running, 10_000), ["Started."]);
        assert_eq!(
            watches.due(&self::rules(&ALL), &running, 40_000),
            Vec::new(),
            "polling the same state is silent"
        );
        assert_eq!(
            settled(&mut watches, &ALL, &[], 50_000),
            ["Stopped. It's no longer running."]
        );
    }

    #[test]
    fn a_restart_inside_the_quiet_period_is_one_replacement() {
        let mut watches = Watches::default();
        let before = [web("old", HealthState::Unknown)];
        let after = [web("new", HealthState::Unknown)];
        settled(&mut watches, &ALL, &before, 0);
        let rules = rules(&ALL);
        assert_eq!(watches.due(&rules, &[], 10_000), Vec::new());
        assert!(watches.settling());
        assert_eq!(watches.due(&rules, &after, 12_000), Vec::new());
        let notices = watches.due(&rules, &after, 12_000 + SETTLE_MS);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].body, "A new process replaced the previous one.");
        assert_eq!(notices[0].server_id, "new");
        assert!(!watches.settling());
    }

    #[test]
    fn a_change_waits_out_the_quiet_period_before_it_is_reported() {
        let mut watches = Watches::default();
        let running = [web("a", HealthState::Unknown)];
        settled(&mut watches, &ALL, &running, 0);
        let rules = rules(&ALL);
        assert_eq!(watches.due(&rules, &[], 10_000), Vec::new());
        assert_eq!(watches.due(&rules, &[], 10_000 + SETTLE_MS - 1), Vec::new());
        assert_eq!(watches.due(&rules, &[], 10_000 + SETTLE_MS).len(), 1);
    }

    #[test]
    fn a_blip_shorter_than_the_quiet_period_sends_nothing() {
        let mut watches = Watches::default();
        let running = [web("a", HealthState::Unknown)];
        settled(&mut watches, &ALL, &running, 0);
        let rules = rules(&ALL);
        assert_eq!(watches.due(&rules, &[], 10_000), Vec::new());
        assert_eq!(watches.due(&rules, &running, 12_000), Vec::new());
        assert_eq!(watches.due(&rules, &running, 60_000), Vec::new());
        assert!(!watches.settling());
    }

    #[test]
    fn only_the_chosen_events_notify() {
        let mut watches = Watches::default();
        settled(&mut watches, &[WatchEvent::Stopped], &[], 0);
        let running = [web("a", HealthState::Unknown)];
        assert_eq!(
            settled(&mut watches, &[WatchEvent::Stopped], &running, 10_000),
            Vec::<&str>::new()
        );
        assert_eq!(
            settled(&mut watches, &[WatchEvent::Stopped], &[], 20_000),
            ["Stopped. It's no longer running."]
        );
    }

    #[test]
    fn reports_health_changes_but_not_warming_or_lost_checks() {
        let mut watches = Watches::default();
        let responsive = [web("a", HealthState::Responsive)];
        let silent = [web("a", HealthState::TimedOut)];
        let unknown = [web("a", HealthState::Unknown)];
        settled(&mut watches, &ALL, &responsive, 0);
        assert_eq!(
            settled(&mut watches, &ALL, &unknown, 10_000),
            Vec::<&str>::new()
        );
        assert_eq!(
            settled(&mut watches, &ALL, &silent, 20_000),
            ["Stopped responding."]
        );
        assert_eq!(
            settled(&mut watches, &ALL, &responsive, 30_000),
            ["Responding again."]
        );
    }

    #[test]
    fn another_owner_joining_a_watched_port_is_not_a_start() {
        let mut watches = Watches::default();
        let one = [web("a", HealthState::Unknown)];
        let two = [
            web("a", HealthState::Unknown),
            web("b", HealthState::Unknown),
        ];
        settled(&mut watches, &ALL, &one, 0);
        assert_eq!(
            settled(&mut watches, &ALL, &two, 10_000),
            Vec::<&str>::new()
        );
        assert_eq!(
            settled(&mut watches, &ALL, &one, 20_000),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn removing_a_watch_forgets_it() {
        let mut watches = Watches::default();
        let running = [web("a", HealthState::Unknown)];
        settled(&mut watches, &ALL, &running, 0);
        let none = BTreeMap::new();
        assert_eq!(watches.due(&none, &[], 10_000), Vec::new());
        assert_eq!(watches.due(&none, &[], 20_000), Vec::new());
        assert!(watches.seen.is_empty());
    }

    #[test]
    fn a_watch_on_another_port_ignores_this_one() {
        let mut watches = Watches::default();
        let rules = BTreeMap::from([("port:9000".to_string(), ALL.to_vec())]);
        watches.due(&rules, &[], 0);
        let running = [web("a", HealthState::Unknown)];
        watches.due(&rules, &running, 10_000);
        assert_eq!(watches.due(&rules, &running, 20_000), Vec::new());
    }
}
