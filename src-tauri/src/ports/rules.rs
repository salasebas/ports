use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_store::StoreExt;

use super::sampler::{Kind, Server, Snapshot};
use super::{Monitor, blocking, lock};

const STORE: &str = "rules.json";
const KEY: &str = "rules";
const VERSION: u32 = 1;
const RULES_EVENT: &str = "ports://rules";
const PATTERN_LIMIT: usize = 200;
const AUTO_IDLE_MINUTES: std::ops::RangeInclusive<u64> = 15..=24 * 60;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Rules {
    version: u32,
    pub port_range: Option<PortRange>,
    pub include: Vec<Rule>,
    pub exclude: Vec<Rule>,
    pub hidden: Vec<String>,
    pub protection: BTreeMap<String, bool>,
    pub auto_cleanup: AutoCleanup,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            version: VERSION,
            port_range: None,
            include: Vec::new(),
            exclude: Vec::new(),
            hidden: Vec::new(),
            protection: BTreeMap::new(),
            auto_cleanup: AutoCleanup::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PortRange {
    pub start: u16,
    pub end: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Field {
    Process,
    Command,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub field: Field,
    pub pattern: String,
}

impl Rule {
    fn matches(&self, server: &Server) -> bool {
        let text = match self.field {
            Field::Process => &server.process,
            Field::Command => &server.command,
        };
        text.to_lowercase().contains(&self.pattern.to_lowercase())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AutoCleanup {
    pub policy: Option<Policy>,
    pub paused: bool,
}

impl AutoCleanup {
    pub fn active(&self) -> Option<Policy> {
        self.policy.filter(|_| !self.paused)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    pub scope: Scope,
    pub idle_minutes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Deleted,
    Idle,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Change {
    ResetLists,
    Hide {
        entry: String,
    },
    Unhide {
        entry: String,
    },
    Protect {
        key: String,
        value: bool,
    },
    PortRange {
        range: Option<PortRange>,
    },
    Filters {
        include: Vec<Rule>,
        exclude: Vec<Rule>,
    },
    AutoCleanup {
        policy: Option<Policy>,
        paused: bool,
    },
    Import {
        hidden: Vec<String>,
        protection: BTreeMap<String, bool>,
    },
}

pub fn key(server: &Server) -> String {
    match &server.folder {
        Some(folder) if server.kind != Kind::Container => format!("project:{folder}"),
        _ => format!("port:{}", server.port),
    }
}

fn check_entry(entry: &str) -> Result<(), String> {
    let valid = match entry.split_once(':') {
        Some(("port", port)) => port.parse::<u16>().is_ok_and(|port| port > 0),
        Some(("process", name)) => !name.trim().is_empty(),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(format!("{entry} is not a port or a process to hide"))
    }
}

fn check_range(range: PortRange) -> Result<(), String> {
    if range.start == 0 {
        Err("Ports start at 1".into())
    } else if range.start > range.end {
        Err(format!(
            "The range starts at {} but ends at {}",
            range.start, range.end
        ))
    } else {
        Ok(())
    }
}

fn check_rule(rule: &Rule) -> Result<Rule, String> {
    let pattern = rule.pattern.trim();
    if pattern.is_empty() {
        return Err("A rule needs some text to match".into());
    }
    if pattern.len() > PATTERN_LIMIT {
        return Err(format!("A rule matches at most {PATTERN_LIMIT} characters"));
    }
    Ok(Rule {
        field: rule.field,
        pattern: pattern.to_string(),
    })
}

impl Rules {
    pub fn hides(&self, server: &Server) -> bool {
        let entry = |entry: &String| match entry.split_once(':') {
            Some(("port", port)) => port.parse() == Ok(server.port),
            Some(("process", name)) => name == server.process,
            _ => false,
        };
        self.hidden.iter().any(entry)
            || self
                .port_range
                .is_some_and(|range| !(range.start..=range.end).contains(&server.port))
            || self.exclude.iter().any(|rule| rule.matches(server))
            || (!self.include.is_empty() && !self.include.iter().any(|rule| rule.matches(server)))
    }

    pub fn apply(&self, servers: &mut [Server]) {
        for server in servers {
            server.hidden = self.hides(server);
            if let Some(&protected) = self.protection.get(&key(server)) {
                server.protected = protected;
            }
        }
    }

    pub fn change(&mut self, change: Change) -> Result<(), String> {
        match change {
            Change::ResetLists => {
                self.port_range = None;
                self.include.clear();
                self.exclude.clear();
                self.hidden.clear();
            }
            Change::Hide { entry } => {
                check_entry(&entry)?;
                if !self.hidden.contains(&entry) {
                    self.hidden.push(entry);
                }
            }
            Change::Unhide { entry } => self.hidden.retain(|hidden| *hidden != entry),
            Change::Protect { key, value } => {
                self.protection.insert(key, value);
            }
            Change::PortRange { range } => {
                if let Some(range) = range {
                    check_range(range)?;
                }
                self.port_range = range;
            }
            Change::Filters { include, exclude } => {
                let include = include.iter().map(check_rule).collect::<Result<_, _>>()?;
                let exclude = exclude.iter().map(check_rule).collect::<Result<_, _>>()?;
                self.include = include;
                self.exclude = exclude;
            }
            Change::AutoCleanup { policy, paused } => {
                if let Some(policy) = policy
                    && !AUTO_IDLE_MINUTES.contains(&policy.idle_minutes)
                {
                    return Err(format!(
                        "Automatic clean up waits between {} minutes and {} hours",
                        AUTO_IDLE_MINUTES.start(),
                        AUTO_IDLE_MINUTES.end() / 60
                    ));
                }
                self.auto_cleanup = AutoCleanup { policy, paused };
            }
            Change::Import { hidden, protection } => {
                for entry in hidden {
                    if check_entry(&entry).is_ok() && !self.hidden.contains(&entry) {
                        self.hidden.push(entry);
                    }
                }
                for (key, value) in protection {
                    self.protection.entry(key).or_insert(value);
                }
            }
        }
        Ok(())
    }
}

pub fn load(app: &AppHandle) -> Rules {
    app.store(STORE)
        .ok()
        .and_then(|store| store.get(KEY))
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn save(app: &AppHandle, rules: &Rules) -> Result<(), String> {
    let store = app.store(STORE).map_err(|error| error.to_string())?;
    let previous = store.get(KEY);
    store.set(
        KEY,
        serde_json::to_value(rules).map_err(|error| error.to_string())?,
    );
    if let Err(error) = store.save() {
        match previous {
            Some(value) => store.set(KEY, value),
            None => {
                store.delete(KEY);
            }
        }
        return Err(format!("Could not save the rules: {error}"));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct RulesView {
    revision: u64,
    rules: Rules,
}

#[derive(Debug, Serialize)]
pub struct RulesUpdate {
    rules: RulesView,
    snapshot: Snapshot,
}

#[tauri::command]
pub fn get_rules(monitor: tauri::State<'_, Monitor>) -> RulesView {
    let (revision, rules) = monitor.rules();
    RulesView { revision, rules }
}

#[tauri::command]
pub async fn update_rules(app: AppHandle, change: Change) -> Result<RulesUpdate, String> {
    blocking(move || {
        let monitor = app.state::<Monitor>();
        let view = {
            let mut state = lock(&monitor.rules);
            let mut next = state.rules.clone();
            next.change(change)?;
            if next != state.rules {
                save(&app, &next)?;
                state.rules = next;
                state.revision += 1;
            }
            RulesView {
                revision: state.revision,
                rules: state.rules.clone(),
            }
        };
        if let Err(error) = app.emit(RULES_EVENT, &view) {
            eprintln!("could not send the rules: {error}");
        }
        let snapshot = monitor.publish(&app)?;
        Ok(RulesUpdate {
            rules: view,
            snapshot,
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::alerts::tests::server;

    fn rule(field: Field, pattern: &str) -> Rule {
        Rule {
            field,
            pattern: pattern.into(),
        }
    }

    #[test]
    fn resetting_lists_clears_filters_and_hidden_servers_but_keeps_protection_and_cleanup() {
        let mut rules = Rules {
            port_range: Some(PortRange {
                start: 3000,
                end: 4000,
            }),
            include: vec![rule(Field::Process, "node")],
            exclude: vec![rule(Field::Process, "vite")],
            hidden: vec!["port:3000".into()],
            protection: BTreeMap::from([("port:3000".into(), true)]),
            auto_cleanup: AutoCleanup {
                policy: Some(Policy {
                    scope: Scope::Deleted,
                    idle_minutes: 60,
                }),
                paused: true,
            },
            ..Rules::default()
        };
        let original = rules.clone();
        rules.change(Change::ResetLists).unwrap();
        assert!(rules.port_range.is_none());
        assert!(rules.include.is_empty());
        assert!(rules.exclude.is_empty());
        assert!(rules.hidden.is_empty());
        assert_eq!(rules.protection, original.protection);
        assert_eq!(rules.auto_cleanup, original.auto_cleanup);
    }

    #[test]
    fn hides_ports_outside_the_range() {
        let mut rules = Rules::default();
        rules
            .change(Change::PortRange {
                range: Some(PortRange {
                    start: 3000,
                    end: 9999,
                }),
            })
            .unwrap();
        assert!(!rules.hides(&server(3000, 0)));
        assert!(!rules.hides(&server(9999, 0)));
        assert!(rules.hides(&server(2999, 0)));
        assert!(rules.hides(&server(10000, 0)));
    }

    #[test]
    fn hides_a_port_or_every_port_of_a_process() {
        let mut rules = Rules::default();
        let mut airplay = server(5000, 0);
        airplay.process = "ControlCenter".into();
        let mut airplay2 = airplay.clone();
        airplay2.port = 7000;
        rules
            .change(Change::Hide {
                entry: "process:ControlCenter".into(),
            })
            .unwrap();
        assert!(rules.hides(&airplay) && rules.hides(&airplay2));
        assert!(!rules.hides(&server(5173, 0)));
        rules
            .change(Change::Hide {
                entry: "port:5173".into(),
            })
            .unwrap();
        assert!(rules.hides(&server(5173, 0)));
        rules
            .change(Change::Unhide {
                entry: "process:ControlCenter".into(),
            })
            .unwrap();
        assert!(!rules.hides(&airplay));
    }

    #[test]
    fn exclude_rules_win_over_include_rules() {
        let mut rules = Rules::default();
        let mut vite = server(5173, 0);
        vite.command = "npm run dev".into();
        let mut storybook = server(6006, 0);
        storybook.command = "npm run storybook".into();
        let mut postgres = server(5432, 0);
        postgres.process = "postgres".into();
        postgres.command = "postgres -D data".into();
        rules
            .change(Change::Filters {
                include: vec![rule(Field::Command, "NPM RUN")],
                exclude: vec![rule(Field::Command, "storybook")],
            })
            .unwrap();
        assert!(!rules.hides(&vite));
        assert!(rules.hides(&storybook));
        assert!(
            rules.hides(&postgres),
            "include rules list only what they match"
        );
        rules
            .change(Change::Filters {
                include: Vec::new(),
                exclude: vec![rule(Field::Process, "postgres")],
            })
            .unwrap();
        assert!(rules.hides(&postgres) && !rules.hides(&storybook));
    }

    #[test]
    fn rejects_an_invalid_change_and_keeps_the_saved_rules() {
        let mut rules = Rules::default();
        let range = Some(PortRange {
            start: 3000,
            end: 9999,
        });
        rules.change(Change::PortRange { range }).unwrap();
        rules
            .change(Change::Filters {
                include: Vec::new(),
                exclude: vec![rule(Field::Process, "ControlCenter")],
            })
            .unwrap();
        let saved = rules.clone();
        for invalid in [
            Change::PortRange {
                range: Some(PortRange {
                    start: 9000,
                    end: 3000,
                }),
            },
            Change::PortRange {
                range: Some(PortRange { start: 0, end: 80 }),
            },
            Change::Filters {
                include: Vec::new(),
                exclude: vec![
                    rule(Field::Process, "ControlCenter"),
                    rule(Field::Command, "  "),
                ],
            },
            Change::Hide {
                entry: "port:99999".into(),
            },
            Change::AutoCleanup {
                policy: Some(Policy {
                    scope: Scope::Idle,
                    idle_minutes: 1,
                }),
                paused: false,
            },
        ] {
            assert!(rules.change(invalid).is_err());
            assert_eq!(rules, saved);
        }
    }

    #[test]
    fn applies_the_users_protection_over_the_default() {
        let mut rules = Rules::default();
        let mut db = server(5432, 0);
        db.protected = true;
        let mut web = server(5173, 0);
        web.folder = Some("~/web".into());
        let mut web2 = web.clone();
        web2.port = 5174;
        let mut container = web.clone();
        container.port = 5433;
        container.kind = Kind::Container;
        rules
            .change(Change::Protect {
                key: key(&db),
                value: false,
            })
            .unwrap();
        rules
            .change(Change::Protect {
                key: key(&web),
                value: true,
            })
            .unwrap();
        let mut servers = [db, web, web2, container];
        rules.apply(&mut servers);
        assert_eq!(
            servers.map(|server| server.protected),
            [false, true, true, false],
            "protection covers every port of a project, but a container stays on its own port"
        );
    }

    #[test]
    fn imports_old_preferences_without_overriding_newer_choices() {
        let mut rules = Rules::default();
        rules
            .change(Change::Protect {
                key: "port:3000".into(),
                value: true,
            })
            .unwrap();
        rules
            .change(Change::Import {
                hidden: vec!["port:5000".into(), "nonsense".into(), "port:5000".into()],
                protection: BTreeMap::from([
                    ("port:3000".into(), false),
                    ("port:8080".into(), true),
                ]),
            })
            .unwrap();
        assert_eq!(rules.hidden, ["port:5000"]);
        assert!(rules.protection["port:3000"]);
        assert!(rules.protection["port:8080"]);
    }

    #[test]
    fn automatic_clean_up_is_off_until_a_policy_is_chosen_and_pauses() {
        let mut rules: Rules = serde_json::from_str("{}").unwrap();
        assert_eq!(rules.auto_cleanup.active(), None);
        let policy = Policy {
            scope: Scope::Deleted,
            idle_minutes: 60,
        };
        rules
            .change(Change::AutoCleanup {
                policy: Some(policy),
                paused: false,
            })
            .unwrap();
        assert_eq!(rules.auto_cleanup.active(), Some(policy));
        rules
            .change(Change::AutoCleanup {
                policy: Some(policy),
                paused: true,
            })
            .unwrap();
        assert_eq!(rules.auto_cleanup.active(), None);
    }
}
