//! The one guard every inner-side action passes through.
//!
//! Rules are evaluated in order and the first match decides. Nothing matching
//! means deny. Two floors sit above the rules and no rule can lift them:
//! the outer side never gets a capability, and an agent never performs an
//! outward action without approval.
//!
//! The guard answers "may you do this?". The service that owns the resource
//! still clamps what the caller may do.

/// Something a caller can ask to do. There is deliberately no capability that
/// touches a page: add-ons attach to the membrane, never to web content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Capability {
    /// Receive traffic observations.
    ObserveTraffic,
    /// Read stored security events.
    ReadEvents,
    /// Export stored security events.
    ExportEvents,
    /// Create or destroy compartments.
    ManageCompartments,
    /// Send a compartment to a URL.
    Navigate,
    /// Send keyboard or pointer input to a compartment.
    SendInput,
    /// Change which route a compartment uses.
    SetRoute,
}

impl Capability {
    /// Resolves a capability name from a manifest. Unknown names return `None`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "observe-traffic" => Some(Self::ObserveTraffic),
            "read-events" => Some(Self::ReadEvents),
            "export-events" => Some(Self::ExportEvents),
            "manage-compartments" => Some(Self::ManageCompartments),
            "navigate" => Some(Self::Navigate),
            "send-input" => Some(Self::SendInput),
            "set-route" => Some(Self::SetRoute),
            _ => None,
        }
    }

    /// True when the action can cause traffic or behaviour the web can see.
    #[must_use]
    pub fn is_outward(self) -> bool {
        matches!(self, Self::Navigate | Self::SendInput | Self::SetRoute)
    }
}

/// Who is asking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Subject {
    /// The Aurelia shell.
    Shell,
    /// A client of the broker control API.
    ControlApi,
    /// An installed add-on, by id.
    AddOn(String),
    /// An automated agent, by id.
    Agent(String),
    /// Anything on the outer side of the membrane.
    Outer,
}

/// Which subjects a rule applies to. There is no way to name the outer side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubjectMatcher {
    /// The Aurelia shell.
    Shell,
    /// Control API clients.
    ControlApi,
    /// One add-on.
    AddOn(String),
    /// Every add-on.
    AnyAddOn,
    /// One agent.
    Agent(String),
    /// Every agent.
    AnyAgent,
}

impl SubjectMatcher {
    fn matches(&self, subject: &Subject) -> bool {
        match (self, subject) {
            (Self::Shell, Subject::Shell)
            | (Self::ControlApi, Subject::ControlApi)
            | (Self::AnyAddOn, Subject::AddOn(_))
            | (Self::AnyAgent, Subject::Agent(_)) => true,
            (Self::AddOn(want), Subject::AddOn(got)) | (Self::Agent(want), Subject::Agent(got)) => {
                want == got
            }
            _ => false,
        }
    }
}

/// What a matching rule does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Permit the action.
    Allow,
    /// Refuse the action.
    Deny,
}

/// One line of policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// Who the rule covers.
    pub subject: SubjectMatcher,
    /// What the rule covers.
    pub capability: Capability,
    /// What happens on a match.
    pub effect: Effect,
}

/// Why an action was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DenyReason {
    /// The request came from the outer side.
    OuterSide,
    /// The capability name is not known.
    UnknownCapability,
    /// No rule matched.
    NoMatchingRule,
    /// A rule refused it.
    ExplicitRule,
}

/// The guard's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Go ahead.
    Allow,
    /// Go ahead only after the user approves.
    NeedsApproval,
    /// Refused.
    Deny(DenyReason),
}

/// An ordered, default-deny rule set.
#[derive(Clone, Debug, Default)]
pub struct Guard {
    rules: Vec<Rule>,
}

impl Guard {
    /// Builds a guard. Rule order is significant.
    #[must_use]
    pub fn new(rules: Vec<Rule>) -> Self {
        Self { rules }
    }

    /// Decides one request.
    #[must_use]
    pub fn check(&self, subject: &Subject, capability: Capability) -> Decision {
        if *subject == Subject::Outer {
            return Decision::Deny(DenyReason::OuterSide);
        }
        let matched = self
            .rules
            .iter()
            .find(|rule| rule.capability == capability && rule.subject.matches(subject));
        match matched.map(|rule| rule.effect) {
            None => Decision::Deny(DenyReason::NoMatchingRule),
            Some(Effect::Deny) => Decision::Deny(DenyReason::ExplicitRule),
            Some(Effect::Allow) => {
                if matches!(subject, Subject::Agent(_)) && capability.is_outward() {
                    Decision::NeedsApproval
                } else {
                    Decision::Allow
                }
            }
        }
    }

    /// Decides a request that names its capability as text.
    #[must_use]
    pub fn check_named(&self, subject: &Subject, capability: &str) -> Decision {
        match Capability::parse(capability) {
            Some(known) => self.check(subject, known),
            None => Decision::Deny(DenyReason::UnknownCapability),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Capability, Decision, DenyReason, Effect, Guard, Rule, Subject, SubjectMatcher};

    fn rule(subject: SubjectMatcher, capability: Capability, effect: Effect) -> Rule {
        Rule {
            subject,
            capability,
            effect,
        }
    }

    #[test]
    fn empty_guard_denies_everything() {
        let guard = Guard::default();
        assert_eq!(
            guard.check(&Subject::Shell, Capability::Navigate),
            Decision::Deny(DenyReason::NoMatchingRule)
        );
    }

    #[test]
    fn unknown_capability_is_denied() {
        let guard = Guard::new(vec![rule(
            SubjectMatcher::Shell,
            Capability::Navigate,
            Effect::Allow,
        )]);
        assert_eq!(
            guard.check_named(&Subject::Shell, "read-host-files"),
            Decision::Deny(DenyReason::UnknownCapability)
        );
    }

    #[test]
    fn known_capability_name_resolves() {
        let guard = Guard::new(vec![rule(
            SubjectMatcher::Shell,
            Capability::Navigate,
            Effect::Allow,
        )]);
        assert_eq!(
            guard.check_named(&Subject::Shell, "navigate"),
            Decision::Allow
        );
    }

    #[test]
    fn first_matching_rule_wins() {
        let guard = Guard::new(vec![
            rule(
                SubjectMatcher::AddOn("shield".into()),
                Capability::ReadEvents,
                Effect::Deny,
            ),
            rule(
                SubjectMatcher::AnyAddOn,
                Capability::ReadEvents,
                Effect::Allow,
            ),
        ]);
        assert_eq!(
            guard.check(&Subject::AddOn("shield".into()), Capability::ReadEvents),
            Decision::Deny(DenyReason::ExplicitRule)
        );
        assert_eq!(
            guard.check(&Subject::AddOn("other".into()), Capability::ReadEvents),
            Decision::Allow
        );
    }

    #[test]
    fn rule_for_one_add_on_does_not_cover_another() {
        let guard = Guard::new(vec![rule(
            SubjectMatcher::AddOn("shield".into()),
            Capability::ObserveTraffic,
            Effect::Allow,
        )]);
        assert_eq!(
            guard.check(&Subject::AddOn("other".into()), Capability::ObserveTraffic),
            Decision::Deny(DenyReason::NoMatchingRule)
        );
    }

    #[test]
    fn outer_side_is_denied_whatever_the_rules_say() {
        let every_inner_subject_allowed = vec![
            rule(SubjectMatcher::Shell, Capability::SetRoute, Effect::Allow),
            rule(
                SubjectMatcher::ControlApi,
                Capability::SetRoute,
                Effect::Allow,
            ),
            rule(
                SubjectMatcher::AnyAddOn,
                Capability::SetRoute,
                Effect::Allow,
            ),
            rule(
                SubjectMatcher::AnyAgent,
                Capability::SetRoute,
                Effect::Allow,
            ),
        ];
        let guard = Guard::new(every_inner_subject_allowed);
        assert_eq!(
            guard.check(&Subject::Outer, Capability::SetRoute),
            Decision::Deny(DenyReason::OuterSide)
        );
    }

    #[test]
    fn agent_outward_action_needs_approval_even_when_allowed() {
        let guard = Guard::new(vec![rule(
            SubjectMatcher::AnyAgent,
            Capability::Navigate,
            Effect::Allow,
        )]);
        assert_eq!(
            guard.check(&Subject::Agent("stagehand".into()), Capability::Navigate),
            Decision::NeedsApproval
        );
    }

    #[test]
    fn agent_inward_action_needs_no_approval() {
        let guard = Guard::new(vec![rule(
            SubjectMatcher::AnyAgent,
            Capability::ReadEvents,
            Effect::Allow,
        )]);
        assert_eq!(
            guard.check(&Subject::Agent("stagehand".into()), Capability::ReadEvents),
            Decision::Allow
        );
    }
}
