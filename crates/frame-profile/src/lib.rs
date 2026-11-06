//! Frame Profiles: one skeleton, many machines.
//!
//! A Profile picks a role and what runs inside it. Unknown keys are refused,
//! and no key exists for anything the web can see, so a Profile cannot change
//! the outward print. Signature checking arrives with the update system.

use serde::Deserialize;
use std::collections::BTreeSet;
use std::fmt;

/// The only accepted manifest schema.
pub const SCHEMA: u32 = 1;

/// What this Frame is for.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    /// Everyday browser.
    Browser,
    /// Marketing tool.
    Marketing,
    /// Dev environment.
    Dev,
    /// Honeypot.
    Honeypot,
    /// Headless server.
    Headless,
    /// Car or kiosk.
    Kiosk,
}

/// Which inner interface runs.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Shell {
    /// The Aurelia shell.
    Aurelia,
    /// No interface; control API only.
    None,
}

/// How traffic leaves.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Route {
    /// Plain TCP and UDP through the host NIC.
    Direct,
    /// Through a Frame node you own.
    OwnExit,
    /// Through Tor.
    Tor,
}

/// One compartment a Profile asks for.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CompartmentPlan {
    /// Name, unique within the Profile.
    pub name: String,
    /// Thrown away when closed.
    #[serde(default = "disposable_by_default")]
    pub disposable: bool,
    /// Accepts inbound connections. Honeypot only.
    #[serde(default)]
    pub inbound: bool,
}

fn disposable_by_default() -> bool {
    true
}

/// A validated Profile.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// Manifest schema version.
    pub schema: u32,
    /// Profile name.
    pub name: String,
    /// What this Frame is for.
    pub role: Role,
    /// Which inner interface runs.
    pub shell: Shell,
    /// How traffic leaves.
    pub route: Route,
    /// Add-ons to load, by id.
    #[serde(default)]
    pub add_ons: Vec<String>,
    /// Compartments to prepare.
    pub compartments: Vec<CompartmentPlan>,
}

/// Why a Profile was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileError {
    /// Not valid TOML, a missing key, or an unknown key.
    Parse(String),
    /// The schema version is not supported.
    UnsupportedSchema(u32),
    /// A name is not lowercase letters, digits and hyphens.
    BadName(String),
    /// The Profile lists no compartments.
    NoCompartments,
    /// Two compartments share a name.
    DuplicateCompartment(String),
    /// An add-on is listed twice.
    DuplicateAddOn(String),
    /// Inbound was requested outside the honeypot role.
    InboundNotAllowed(String),
    /// A honeypot's inbound compartment must be disposable.
    InboundMustBeDisposable(String),
    /// The shell does not suit the role.
    ShellMismatch,
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(reason) => write!(f, "profile is not valid: {reason}"),
            Self::UnsupportedSchema(version) => write!(f, "unsupported schema {version}"),
            Self::BadName(name) => write!(f, "bad name: {name}"),
            Self::NoCompartments => f.write_str("no compartments listed"),
            Self::DuplicateCompartment(name) => write!(f, "duplicate compartment: {name}"),
            Self::DuplicateAddOn(name) => write!(f, "duplicate add-on: {name}"),
            Self::InboundNotAllowed(name) => {
                write!(f, "inbound is honeypot only, found on {name}")
            }
            Self::InboundMustBeDisposable(name) => {
                write!(f, "inbound compartment must be disposable: {name}")
            }
            Self::ShellMismatch => f.write_str("shell does not suit this role"),
        }
    }
}

impl std::error::Error for ProfileError {}

fn check_name(name: &str) -> Result<(), ProfileError> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if valid {
        Ok(())
    } else {
        Err(ProfileError::BadName(name.to_owned()))
    }
}

impl Profile {
    /// Parses and validates a manifest.
    ///
    /// # Errors
    /// [`ProfileError`] names the first problem found.
    pub fn parse(manifest: &str) -> Result<Self, ProfileError> {
        let profile: Self =
            toml::from_str(manifest).map_err(|error| ProfileError::Parse(error.to_string()))?;
        profile.validate()?;
        Ok(profile)
    }

    fn validate(&self) -> Result<(), ProfileError> {
        if self.schema != SCHEMA {
            return Err(ProfileError::UnsupportedSchema(self.schema));
        }
        check_name(&self.name)?;
        match (self.role, self.shell) {
            (Role::Headless, Shell::Aurelia)
            | (Role::Browser | Role::Kiosk | Role::Marketing | Role::Dev, Shell::None) => {
                return Err(ProfileError::ShellMismatch);
            }
            _ => {}
        }
        let mut add_ons = BTreeSet::new();
        for add_on in &self.add_ons {
            check_name(add_on)?;
            if !add_ons.insert(add_on) {
                return Err(ProfileError::DuplicateAddOn(add_on.clone()));
            }
        }
        if self.compartments.is_empty() {
            return Err(ProfileError::NoCompartments);
        }
        let mut names = BTreeSet::new();
        for compartment in &self.compartments {
            check_name(&compartment.name)?;
            if !names.insert(&compartment.name) {
                return Err(ProfileError::DuplicateCompartment(compartment.name.clone()));
            }
            if compartment.inbound {
                if self.role != Role::Honeypot {
                    return Err(ProfileError::InboundNotAllowed(compartment.name.clone()));
                }
                if !compartment.disposable {
                    return Err(ProfileError::InboundMustBeDisposable(
                        compartment.name.clone(),
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Profile, ProfileError, Role, Route, Shell};

    const BROWSER: &str = r#"
        schema = 1
        name = "daily"
        role = "browser"
        shell = "aurelia"
        route = "direct"
        add_ons = ["shield-siem"]

        [[compartments]]
        name = "work"
        disposable = false

        [[compartments]]
        name = "links"
    "#;

    const HONEYPOT: &str = r#"
        schema = 1
        name = "decoy"
        role = "honeypot"
        shell = "none"
        route = "own-exit"

        [[compartments]]
        name = "trap"
        inbound = true
    "#;

    #[test]
    fn valid_browser_profile_parses() -> Result<(), ProfileError> {
        let profile = Profile::parse(BROWSER)?;
        assert_eq!(profile.role, Role::Browser);
        assert_eq!(profile.shell, Shell::Aurelia);
        assert_eq!(profile.route, Route::Direct);
        assert_eq!(profile.compartments.len(), 2);
        Ok(())
    }

    #[test]
    fn compartments_are_disposable_unless_stated() -> Result<(), ProfileError> {
        let profile = Profile::parse(BROWSER)?;
        assert!(!profile.compartments[0].disposable);
        assert!(profile.compartments[1].disposable);
        Ok(())
    }

    #[test]
    fn honeypot_may_accept_inbound() -> Result<(), ProfileError> {
        assert!(Profile::parse(HONEYPOT)?.compartments[0].inbound);
        Ok(())
    }

    #[test]
    fn inbound_outside_honeypot_is_refused() {
        let manifest = HONEYPOT.replace("\"honeypot\"", "\"headless\"");
        assert_eq!(
            Profile::parse(&manifest),
            Err(ProfileError::InboundNotAllowed("trap".into()))
        );
    }

    #[test]
    fn persistent_inbound_compartment_is_refused() {
        let manifest = HONEYPOT.replace("inbound = true", "inbound = true\ndisposable = false");
        assert_eq!(
            Profile::parse(&manifest),
            Err(ProfileError::InboundMustBeDisposable("trap".into()))
        );
    }

    #[test]
    fn a_profile_cannot_change_the_outward_print() {
        for key in ["user_agent", "fingerprint", "screen", "tls", "locale"] {
            let manifest = BROWSER.replace("schema = 1", &format!("schema = 1\n{key} = \"x\""));
            assert!(
                matches!(Profile::parse(&manifest), Err(ProfileError::Parse(_))),
                "{key} was accepted"
            );
        }
    }

    #[test]
    fn a_compartment_cannot_ask_for_host_access() {
        let manifest = BROWSER.replace("disposable = false", "host_path = \"/home\"");
        assert!(matches!(
            Profile::parse(&manifest),
            Err(ProfileError::Parse(_))
        ));
    }

    #[test]
    fn unknown_schema_is_refused() {
        let manifest = BROWSER.replace("schema = 1", "schema = 2");
        assert_eq!(
            Profile::parse(&manifest),
            Err(ProfileError::UnsupportedSchema(2))
        );
    }

    #[test]
    fn unknown_role_is_refused() {
        let manifest = BROWSER.replace("\"browser\"", "\"root\"");
        assert!(matches!(
            Profile::parse(&manifest),
            Err(ProfileError::Parse(_))
        ));
    }

    #[test]
    fn missing_required_key_is_refused() {
        let manifest = BROWSER.replace("route = \"direct\"", "");
        assert!(matches!(
            Profile::parse(&manifest),
            Err(ProfileError::Parse(_))
        ));
    }

    #[test]
    fn headless_cannot_have_a_shell() {
        let manifest = HONEYPOT
            .replace("\"honeypot\"", "\"headless\"")
            .replace("\"none\"", "\"aurelia\"")
            .replace("inbound = true", "");
        assert_eq!(Profile::parse(&manifest), Err(ProfileError::ShellMismatch));
    }

    #[test]
    fn browser_needs_a_shell() {
        let manifest = BROWSER.replace("\"aurelia\"", "\"none\"");
        assert_eq!(Profile::parse(&manifest), Err(ProfileError::ShellMismatch));
    }

    #[test]
    fn duplicate_compartments_are_refused() {
        let manifest = BROWSER.replace("name = \"links\"", "name = \"work\"");
        assert_eq!(
            Profile::parse(&manifest),
            Err(ProfileError::DuplicateCompartment("work".into()))
        );
    }

    #[test]
    fn duplicate_add_ons_are_refused() {
        let manifest = BROWSER.replace("[\"shield-siem\"]", "[\"shield-siem\", \"shield-siem\"]");
        assert_eq!(
            Profile::parse(&manifest),
            Err(ProfileError::DuplicateAddOn("shield-siem".into()))
        );
    }

    #[test]
    fn profile_with_no_compartments_is_refused() {
        let manifest = "schema = 1\nname = \"a\"\nrole = \"browser\"\nshell = \"aurelia\"\nroute = \"direct\"\ncompartments = []\n";
        assert_eq!(Profile::parse(manifest), Err(ProfileError::NoCompartments));
    }

    #[test]
    fn names_with_path_characters_are_refused() {
        let manifest = BROWSER.replace("name = \"links\"", "name = \"../etc\"");
        assert_eq!(
            Profile::parse(&manifest),
            Err(ProfileError::BadName("../etc".into()))
        );
    }
}
