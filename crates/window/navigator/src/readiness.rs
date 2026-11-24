//! When a server's answers count as sound, by the rules of a row's `ready`. A [`Readiness`] is fed
//! what the server reports, in order, and knows no server: every rule is row data.

use serde_json::Value;

use crate::registry::Ready;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readying {
    /// Started, and the rule not yet met: answers are found by text match meanwhile.
    Starting,
    /// A server once ready stays ready until it ends.
    Ready,
    /// Ended, or failed to answer `initialize`.
    Gone,
}

#[derive(Debug, Clone)]
pub struct Readiness {
    rule: Ready,
    now: Readying,
    open: Vec<Value>,
    began: bool,
}

impl Readiness {
    pub fn new(rule: &Ready) -> Readiness {
        Readiness {
            rule: rule.clone(),
            now: Readying::Starting,
            open: Vec::new(),
            began: false,
        }
    }

    pub fn answered_initialize(&mut self) -> Readying {
        if self.now == Readying::Starting && self.rule == Ready::Answer {
            self.now = Readying::Ready;
        }
        self.now
    }

    /// Takes a notification in: `$/progress` with a `begin` or an `end` value counts for
    /// [`Ready::Progress`]; one of the rule's method whose params hold its `match`, for
    /// [`Ready::Notification`].
    pub fn notification(&mut self, method: &str, params: &Value) -> Readying {
        // Ready stays ready, and an ended server is not brought back by what it said last.
        if self.now != Readying::Starting {
            return self.now;
        }
        let met = match &self.rule {
            Ready::Answer => false,
            Ready::Progress => method == "$/progress" && self.progress(params),
            Ready::Notification {
                method: wanted,
                matches,
            } => {
                method == wanted
                    && matches
                        .iter()
                        .all(|(key, value)| params.get(key) == Some(value))
            }
        };
        if met {
            self.now = Readying::Ready;
        }
        self.now
    }

    pub fn ended(&mut self) -> Readying {
        self.now = Readying::Gone;
        self.now
    }

    pub fn now(&self) -> Readying {
        self.now
    }

    /// Whether every report begun has now ended. A report with no token or no kind is none.
    fn progress(&mut self, params: &Value) -> bool {
        let Some(token) = params.get("token") else {
            return false;
        };
        match params.pointer("/value/kind").and_then(Value::as_str) {
            Some("begin") => {
                self.began = true;
                if !self.open.contains(token) {
                    self.open.push(token.clone());
                }
                false
            }
            Some("end") => {
                self.open.retain(|open| open != token);
                self.began && self.open.is_empty()
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::registry::{BUILTIN, Registry};

    fn rule_of(row: &str) -> Ready {
        let (registry, errors) = Registry::read(BUILTIN, None);
        assert_eq!(errors, vec![]);
        registry.row(row).unwrap().servers[0].ready.clone()
    }

    fn quiescent(flag: bool) -> Value {
        json!({ "health": "ok", "quiescent": flag })
    }

    fn progress(token: Value, kind: &str) -> Value {
        json!({ "token": token, "value": { "kind": kind, "title": "Indexing" } })
    }

    #[test]
    fn each_built_in_row_has_the_rule_its_server_needs() {
        assert_eq!(rule_of("typescript"), Ready::Answer);
        assert_eq!(rule_of("python"), Ready::Answer);
        assert_eq!(
            rule_of("rust"),
            Ready::Notification {
                method: "experimental/serverStatus".into(),
                matches: json!({ "quiescent": true }).as_object().unwrap().clone(),
            }
        );
    }

    #[test]
    fn a_server_just_started_is_starting_whatever_its_rule() {
        for row in ["typescript", "python", "rust"] {
            let readiness = Readiness::new(&rule_of(row));
            assert_eq!(readiness.now(), Readying::Starting, "{row}");
        }
        assert_eq!(Readiness::new(&Ready::Progress).now(), Readying::Starting);
    }

    #[test]
    fn a_server_by_the_answer_rule_is_ready_once_it_answered_initialize() {
        for row in ["typescript", "python"] {
            let mut readiness = Readiness::new(&rule_of(row));
            assert_eq!(
                readiness.notification("experimental/serverStatus", &quiescent(true)),
                Readying::Starting
            );
            assert_eq!(
                readiness.notification("$/progress", &progress(json!("t"), "end")),
                Readying::Starting
            );
            assert_eq!(readiness.answered_initialize(), Readying::Ready, "{row}");
            assert_eq!(readiness.now(), Readying::Ready);
        }
    }

    #[test]
    fn rust_analyzer_is_ready_when_it_reports_quiescent_and_not_when_it_answers_initialize() {
        let mut readiness = Readiness::new(&rule_of("rust"));
        assert_eq!(readiness.answered_initialize(), Readying::Starting);
        assert_eq!(
            readiness.notification("experimental/serverStatus", &quiescent(false)),
            Readying::Starting
        );
        assert_eq!(
            readiness.notification("$/progress", &progress(json!("roots"), "begin")),
            Readying::Starting
        );
        assert_eq!(
            readiness.notification("$/progress", &progress(json!("roots"), "end")),
            Readying::Starting
        );
        assert_eq!(
            readiness.notification("window/status", &quiescent(true)),
            Readying::Starting
        );
        assert_eq!(
            readiness.notification("experimental/serverStatus", &quiescent(true)),
            Readying::Ready
        );
        assert_eq!(readiness.now(), Readying::Ready);
    }

    #[test]
    fn a_notification_with_no_parameters_or_other_ones_does_not_meet_the_rule() {
        let mut readiness = Readiness::new(&rule_of("rust"));
        for params in [
            json!(null),
            json!([true]),
            json!({}),
            json!({ "quiescent": "true" }),
            json!({ "quiescent": 1 }),
        ] {
            assert_eq!(
                readiness.notification("experimental/serverStatus", &params),
                Readying::Starting,
                "{params}"
            );
        }
    }

    #[test]
    fn ready_stays_ready_after_a_later_report_that_says_otherwise() {
        let mut readiness = Readiness::new(&rule_of("rust"));
        readiness.notification("experimental/serverStatus", &quiescent(true));
        assert_eq!(
            readiness.notification("experimental/serverStatus", &quiescent(false)),
            Readying::Ready
        );
        assert_eq!(readiness.now(), Readying::Ready);

        let mut by_progress = Readiness::new(&Ready::Progress);
        by_progress.notification("$/progress", &progress(json!("a"), "begin"));
        by_progress.notification("$/progress", &progress(json!("a"), "end"));
        assert_eq!(by_progress.now(), Readying::Ready);
        assert_eq!(
            by_progress.notification("$/progress", &progress(json!("b"), "begin")),
            Readying::Ready
        );
    }

    #[test]
    fn a_server_that_ended_is_gone_from_either_state_and_stays_gone() {
        for row in ["typescript", "rust"] {
            let mut starting = Readiness::new(&rule_of(row));
            assert_eq!(starting.ended(), Readying::Gone, "{row} starting");
            assert_eq!(starting.now(), Readying::Gone);

            let mut ready = Readiness::new(&rule_of(row));
            ready.answered_initialize();
            ready.notification("experimental/serverStatus", &quiescent(true));
            assert_eq!(ready.ended(), Readying::Gone, "{row} ready");
            assert_eq!(ready.now(), Readying::Gone);
            assert_eq!(ready.answered_initialize(), Readying::Gone);
            assert_eq!(
                ready.notification("experimental/serverStatus", &quiescent(true)),
                Readying::Gone
            );
        }
    }

    fn notify(readiness: &mut Readiness, token: Value, kind: &str) -> Readying {
        readiness.notification("$/progress", &progress(token, kind))
    }

    #[test]
    fn by_the_progress_rule_a_server_is_ready_when_every_report_begun_has_ended() {
        let mut readiness = Readiness::new(&Ready::Progress);
        assert_eq!(readiness.answered_initialize(), Readying::Starting);
        assert_eq!(
            notify(&mut readiness, json!("a"), "begin"),
            Readying::Starting
        );
        assert_eq!(
            notify(&mut readiness, json!("b"), "begin"),
            Readying::Starting
        );
        assert_eq!(
            notify(&mut readiness, json!("a"), "end"),
            Readying::Starting
        );
        assert_eq!(notify(&mut readiness, json!("b"), "end"), Readying::Ready);
        assert_eq!(readiness.now(), Readying::Ready);
    }

    #[test]
    fn a_token_that_is_a_number_counts_as_one_that_is_text_does() {
        let mut readiness = Readiness::new(&Ready::Progress);
        assert_eq!(
            notify(&mut readiness, json!(1), "begin"),
            Readying::Starting
        );
        assert_eq!(
            notify(&mut readiness, json!("1"), "begin"),
            Readying::Starting
        );
        assert_eq!(notify(&mut readiness, json!(1), "end"), Readying::Starting);
        assert_eq!(notify(&mut readiness, json!("1"), "end"), Readying::Ready);
    }

    #[test]
    fn a_report_under_a_token_that_is_a_number_holds_readiness_back_until_it_ends() {
        // Alone, then beside a token that is text: its begin holds the server back and only
        // its own end lets it go.
        let mut alone = Readiness::new(&Ready::Progress);
        assert_eq!(notify(&mut alone, json!(7), "begin"), Readying::Starting);
        assert_eq!(notify(&mut alone, json!(7), "end"), Readying::Ready);

        let mut beside = Readiness::new(&Ready::Progress);
        assert_eq!(notify(&mut beside, json!("a"), "begin"), Readying::Starting);
        assert_eq!(notify(&mut beside, json!(7), "begin"), Readying::Starting);
        assert_eq!(notify(&mut beside, json!("a"), "end"), Readying::Starting);
        assert_eq!(notify(&mut beside, json!(7), "end"), Readying::Ready);
    }

    #[test]
    fn by_the_progress_rule_an_end_with_no_begin_and_a_report_in_between_do_not_make_it_ready() {
        let mut readiness = Readiness::new(&Ready::Progress);
        assert_eq!(
            notify(&mut readiness, json!("x"), "end"),
            Readying::Starting
        );
        assert_eq!(
            notify(&mut readiness, json!("a"), "begin"),
            Readying::Starting
        );
        assert_eq!(
            notify(&mut readiness, json!("a"), "report"),
            Readying::Starting
        );
        assert_eq!(
            readiness.notification(
                "window/logMessage",
                &json!({ "token": "a", "value": { "kind": "end" } })
            ),
            Readying::Starting
        );
        assert_eq!(
            readiness.notification("$/progress", &json!({ "value": { "kind": "end" } })),
            Readying::Starting
        );
        assert_eq!(
            readiness.notification("$/progress", &json!({ "token": "a" })),
            Readying::Starting
        );
        assert_eq!(
            readiness.notification("$/progress", &json!(null)),
            Readying::Starting
        );
        assert_eq!(notify(&mut readiness, json!("a"), "end"), Readying::Ready);
    }

    #[test]
    fn a_report_begun_twice_under_one_token_ends_with_one_end() {
        let mut readiness = Readiness::new(&Ready::Progress);
        notify(&mut readiness, json!("a"), "begin");
        notify(&mut readiness, json!("a"), "begin");
        assert_eq!(notify(&mut readiness, json!("a"), "end"), Readying::Ready);
    }

    #[test]
    fn a_notification_rule_with_no_match_is_met_by_any_notification_of_its_method() {
        let rule = Ready::Notification {
            method: "standin/status".into(),
            matches: Default::default(),
        };
        let mut readiness = Readiness::new(&rule);
        assert_eq!(
            readiness.notification("standin/other", &json!({})),
            Readying::Starting
        );
        assert_eq!(
            readiness.notification("standin/status", &json!(null)),
            Readying::Ready
        );
    }

    #[test]
    fn a_notification_rule_needs_every_key_of_its_match_and_ignores_the_others() {
        let rule = Ready::Notification {
            method: "standin/status".into(),
            matches: json!({ "ready": true, "n": 2 })
                .as_object()
                .unwrap()
                .clone(),
        };
        let mut readiness = Readiness::new(&rule);
        assert_eq!(
            readiness.notification("standin/status", &json!({ "ready": true })),
            Readying::Starting
        );
        assert_eq!(
            readiness.notification("standin/status", &json!({ "ready": true, "n": 3 })),
            Readying::Starting
        );
        assert_eq!(
            readiness.notification(
                "standin/status",
                &json!({ "ready": true, "n": 2, "more": [] })
            ),
            Readying::Ready
        );
    }
}
