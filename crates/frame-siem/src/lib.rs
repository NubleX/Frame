//! Shield SIEM, ported from the browser extensions in
//! `idarti-com/extensions/`. Monitoring stays opt-in and local.
//!
//! Differences from the extensions, all hardening:
//! - Headers can only enter an [`Event`] through [`SanitizedHeaders`].
//! - Credentials embedded in a URL are removed before storage.
//! - The table of in-flight requests is bounded, so a page cannot grow it.
//! - Every CSV field is quoted, and spreadsheet formulas are neutralised.

use std::collections::{BTreeMap, VecDeque};
use url::Url;

/// Most finished events kept.
pub const MAX_EVENTS: usize = 1000;
/// Most requests tracked in flight at once.
pub const MAX_PENDING: usize = 4096;
/// A completed request slower than this is labelled slow.
pub const SLOW_THRESHOLD_MS: u64 = 3000;

const REDACTED: &str = "[redacted]";
const SENSITIVE_HEADERS: [&str; 6] = [
    "cookie",
    "authorization",
    "proxy-authorization",
    "set-cookie",
    "x-api-key",
    "x-auth-token",
];
const CRYPTOMINERS: [&str; 8] = [
    "coinhive",
    "coin-hive",
    "jsecoin",
    "cryptoloot",
    "webminepool",
    "monerominer",
    "coinimp",
    "minero.cc",
];
const MALWARE: [&str; 6] = [
    "malware",
    "phishing",
    "trojan",
    "ransomware",
    "c2server",
    "botnet",
];
const TRACKERS: [&str; 10] = [
    "tracker",
    "analytics",
    "doubleclick",
    "google-analytics",
    "scorecardresearch",
    "quantserve",
    "hotjar",
    "crazyegg",
    "mouseflow",
    "fullstory",
];
const ADS: [&str; 8] = [
    "doubleclick",
    "googlesyndication",
    "adservice",
    "adsserver",
    "adnxs",
    "criteo",
    "taboola",
    "outbrain",
];

/// How concerning an event is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// Nothing noteworthy.
    Normal,
    /// Tracking or advertising.
    Suspicious,
    /// Known-bad pattern.
    Threat,
    /// Completed, but slowly.
    Slow,
    /// The request failed.
    Failed,
}

impl Severity {
    fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Suspicious => "suspicious",
            Self::Threat => "threat",
            Self::Slow => "slow",
            Self::Failed => "failed",
        }
    }
}

/// Why a request was labelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreatCategory {
    /// Browser cryptomining.
    Cryptominer,
    /// Malware or phishing pattern.
    Malware,
    /// Tracking.
    Tracker,
    /// Advertising.
    Advertising,
}

/// Labels a URL. Substring heuristics, checked most serious first, exactly as
/// the extension does. They produce false positives and are labels, not proof.
#[must_use]
pub fn detect_threat(url: &str) -> (Severity, Option<ThreatCategory>) {
    let lower = url.to_lowercase();
    let hit = |patterns: &[&str]| patterns.iter().any(|pattern| lower.contains(pattern));
    if hit(&CRYPTOMINERS) {
        (Severity::Threat, Some(ThreatCategory::Cryptominer))
    } else if hit(&MALWARE) {
        (Severity::Threat, Some(ThreatCategory::Malware))
    } else if hit(&TRACKERS) {
        (Severity::Suspicious, Some(ThreatCategory::Tracker))
    } else if hit(&ADS) {
        (Severity::Suspicious, Some(ThreatCategory::Advertising))
    } else {
        (Severity::Normal, None)
    }
}

/// Headers with every sensitive value replaced. The only header type an
/// [`Event`] holds, so unsanitised values cannot reach the store.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SanitizedHeaders(Vec<(String, String)>);

impl SanitizedHeaders {
    /// Redacts sensitive values, matching names without regard to case.
    #[must_use]
    pub fn new(headers: &[(String, String)]) -> Self {
        let clean = headers.iter().map(|(name, value)| {
            let sensitive = SENSITIVE_HEADERS
                .iter()
                .any(|known| name.eq_ignore_ascii_case(known));
            let kept = if sensitive { REDACTED } else { value.as_str() };
            (name.clone(), kept.to_owned())
        });
        Self(clean.collect())
    }

    /// The sanitised name and value pairs.
    #[must_use]
    pub fn entries(&self) -> &[(String, String)] {
        &self.0
    }
}

/// How far a request got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventPhase {
    /// In flight.
    Start,
    /// Finished normally.
    Complete,
    /// Failed.
    Error,
}

/// One monitored request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    /// Compartment the broker says this came from.
    pub compartment: u32,
    /// Engine request id.
    pub request_id: u64,
    /// URL with embedded credentials removed.
    pub url: String,
    /// HTTP method.
    pub method: String,
    /// Resource type.
    pub resource_type: String,
    /// Host name, or `unknown`.
    pub domain: String,
    /// Start time in milliseconds since the Unix epoch.
    pub started_ms: u64,
    /// Current label.
    pub severity: Severity,
    /// Why it was labelled.
    pub threat_category: Option<ThreatCategory>,
    /// How far it got.
    pub phase: EventPhase,
    /// HTTP status.
    pub status: Option<u16>,
    /// Size from `Content-Length`.
    pub size: u64,
    /// Total time in milliseconds.
    pub duration_ms: u64,
    /// Served from cache.
    pub from_cache: bool,
    /// Remote address, when known.
    pub ip: Option<String>,
    /// Failure text, when it failed.
    pub error: Option<String>,
    /// Sanitised response headers.
    pub response_headers: SanitizedHeaders,
}

/// Running totals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Requests finished, either way.
    pub total_requests: u64,
    /// Bytes across finished requests.
    pub total_bytes: u64,
    /// Failed requests.
    pub failed_requests: u64,
    /// Slow requests.
    pub slow_requests: u64,
    /// Suspicious requests.
    pub suspicious_requests: u64,
    /// Threat requests.
    pub threat_requests: u64,
    /// Requests not tracked because too many were in flight.
    pub dropped_requests: u64,
}

fn strip_credentials(url: &str) -> (String, String) {
    match Url::parse(url) {
        Ok(mut parsed) => {
            let domain = parsed.host_str().unwrap_or("unknown").to_owned();
            // These only fail for URLs that cannot hold credentials at all.
            parsed.set_username("").ok();
            parsed.set_password(None).ok();
            (parsed.to_string(), domain)
        }
        Err(_) => (url.to_owned(), "unknown".to_owned()),
    }
}

/// The start of a request.
#[derive(Clone, Copy, Debug)]
pub struct RequestStart<'a> {
    /// Compartment, as stamped by the broker.
    pub compartment: u32,
    /// Engine request id.
    pub request_id: u64,
    /// Full URL.
    pub url: &'a str,
    /// HTTP method.
    pub method: &'a str,
    /// Resource type.
    pub resource_type: &'a str,
    /// Start time in milliseconds since the Unix epoch.
    pub started_ms: u64,
}

/// The event store. Owns all monitored data. Off until the user starts it.
#[derive(Debug, Default)]
pub struct Monitor {
    active: bool,
    events: VecDeque<Event>,
    pending: BTreeMap<(u32, u64), Event>,
    stats: Stats,
}

impl Monitor {
    /// Starts monitoring. Only ever a direct user action.
    pub fn start(&mut self) {
        self.active = true;
    }

    /// Stops monitoring and forgets anything in flight.
    pub fn stop(&mut self) {
        self.active = false;
        self.pending.clear();
    }

    /// Deletes everything collected.
    pub fn clear(&mut self) {
        self.events.clear();
        self.pending.clear();
        self.stats = Stats::default();
    }

    /// Finished events, newest first.
    pub fn events(&self) -> impl Iterator<Item = &Event> {
        self.events.iter()
    }

    /// Running totals.
    #[must_use]
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Records the start of a request.
    pub fn on_request_started(&mut self, request: &RequestStart<'_>) {
        if !self.active {
            return;
        }
        if self.pending.len() >= MAX_PENDING {
            self.stats.dropped_requests += 1;
            return;
        }
        let (severity, threat_category) = detect_threat(request.url);
        let (url, domain) = strip_credentials(request.url);
        let event = Event {
            compartment: request.compartment,
            request_id: request.request_id,
            url,
            method: request.method.to_owned(),
            resource_type: request.resource_type.to_owned(),
            domain,
            started_ms: request.started_ms,
            severity,
            threat_category,
            phase: EventPhase::Start,
            status: None,
            size: 0,
            duration_ms: 0,
            from_cache: false,
            ip: None,
            error: None,
            response_headers: SanitizedHeaders::default(),
        };
        self.pending
            .insert((request.compartment, request.request_id), event);
    }

    /// Records response headers.
    pub fn on_response(
        &mut self,
        compartment: u32,
        request_id: u64,
        status: u16,
        headers: &[(String, String)],
    ) {
        let Some(event) = self.pending.get_mut(&(compartment, request_id)) else {
            return;
        };
        event.status = Some(status);
        event.response_headers = SanitizedHeaders::new(headers);
        let length = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"));
        if let Some((_, value)) = length {
            event.size = value.trim().parse().unwrap_or(0);
        }
    }

    /// Records a normal finish.
    pub fn on_completed(
        &mut self,
        compartment: u32,
        request_id: u64,
        duration_ms: u64,
        from_cache: bool,
        ip: Option<&str>,
    ) {
        let Some(mut event) = self.pending.remove(&(compartment, request_id)) else {
            return;
        };
        event.phase = EventPhase::Complete;
        event.duration_ms = duration_ms;
        event.from_cache = from_cache;
        event.ip = ip.map(str::to_owned);
        if duration_ms > SLOW_THRESHOLD_MS && event.severity == Severity::Normal {
            event.severity = Severity::Slow;
        }
        self.finalize(event);
    }

    /// Records a failure.
    pub fn on_failed(&mut self, compartment: u32, request_id: u64, duration_ms: u64, error: &str) {
        let Some(mut event) = self.pending.remove(&(compartment, request_id)) else {
            return;
        };
        event.phase = EventPhase::Error;
        event.severity = Severity::Failed;
        event.error = Some(error.to_owned());
        event.duration_ms = duration_ms;
        self.stats.failed_requests += 1;
        self.finalize(event);
    }

    fn finalize(&mut self, event: Event) {
        self.stats.total_requests += 1;
        self.stats.total_bytes = self.stats.total_bytes.saturating_add(event.size);
        match event.severity {
            Severity::Slow => self.stats.slow_requests += 1,
            Severity::Suspicious => self.stats.suspicious_requests += 1,
            Severity::Threat => self.stats.threat_requests += 1,
            Severity::Normal | Severity::Failed => {}
        }
        self.events.push_front(event);
        self.events.truncate(MAX_EVENTS);
    }

    /// Exports finished events as CSV, newest first.
    #[must_use]
    pub fn export_csv(&self) -> String {
        let mut lines = vec![
            "Timestamp,Method,URL,Status,Type,Size,Duration,Severity,Domain,IP,Compartment"
                .to_owned(),
        ];
        lines.extend(self.events.iter().map(csv_row));
        lines.join("\n")
    }
}

fn csv_field(text: &str) -> String {
    let formula = text.starts_with(['=', '+', '-', '@', '\t', '\r']);
    let guard = if formula { "'" } else { "" };
    format!("\"{guard}{}\"", text.replace('"', "\"\""))
}

fn csv_row(event: &Event) -> String {
    let status = event.status.map_or(String::new(), |code| code.to_string());
    [
        csv_field(&iso_timestamp(event.started_ms)),
        csv_field(&event.method),
        csv_field(&event.url),
        csv_field(&status),
        csv_field(&event.resource_type),
        csv_field(&event.size.to_string()),
        csv_field(&event.duration_ms.to_string()),
        csv_field(event.severity.label()),
        csv_field(&event.domain),
        csv_field(event.ip.as_deref().unwrap_or("")),
        csv_field(&event.compartment.to_string()),
    ]
    .join(",")
}

/// Formats Unix milliseconds as RFC 3339 UTC.
#[must_use]
pub fn iso_timestamp(unix_ms: u64) -> String {
    let seconds = unix_ms / 1000;
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let second_of_day = seconds % 86_400;
    // Civil-from-days, after Howard Hinnant's published algorithm.
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        second_of_day / 3600,
        second_of_day % 3600 / 60,
        second_of_day % 60,
        unix_ms % 1000
    )
}

#[cfg(test)]
mod tests {
    use super::{
        EventPhase, MAX_EVENTS, MAX_PENDING, Monitor, RequestStart, SLOW_THRESHOLD_MS,
        SanitizedHeaders, Severity, ThreatCategory, detect_threat, iso_timestamp,
    };

    fn start(request_id: u64, url: &str) -> RequestStart<'_> {
        RequestStart {
            compartment: 1,
            request_id,
            url,
            method: "GET",
            resource_type: "document",
            started_ms: 0,
        }
    }

    fn active() -> Monitor {
        let mut monitor = Monitor::default();
        monitor.start();
        monitor
    }

    fn header(name: &str, value: &str) -> (String, String) {
        (name.to_owned(), value.to_owned())
    }

    #[test]
    fn labels_match_the_extension() {
        let cases = [
            (
                "https://coinhive.com/x",
                Severity::Threat,
                Some(ThreatCategory::Cryptominer),
            ),
            (
                "https://a.test/phishing",
                Severity::Threat,
                Some(ThreatCategory::Malware),
            ),
            (
                "https://www.google-analytics.com/c",
                Severity::Suspicious,
                Some(ThreatCategory::Tracker),
            ),
            (
                "https://cdn.taboola.com/l",
                Severity::Suspicious,
                Some(ThreatCategory::Advertising),
            ),
            ("https://example.com/", Severity::Normal, None),
        ];
        for (url, severity, category) in cases {
            assert_eq!(detect_threat(url), (severity, category), "{url}");
        }
    }

    #[test]
    fn matching_ignores_case() {
        assert_eq!(detect_threat("https://COINHIVE.com/").0, Severity::Threat);
    }

    #[test]
    fn doubleclick_is_a_tracker_because_trackers_are_checked_first() {
        assert_eq!(
            detect_threat("https://doubleclick.net/").1,
            Some(ThreatCategory::Tracker)
        );
    }

    #[test]
    fn sensitive_header_values_never_survive_in_any_case() {
        let raw = [
            header("Cookie", "secret"),
            header("AUTHORIZATION", "secret"),
            header("proxy-authorization", "secret"),
            header("Set-Cookie", "secret"),
            header("X-Api-Key", "secret"),
            header("x-auth-token", "secret"),
            header("Content-Type", "text/html"),
        ];
        let clean = SanitizedHeaders::new(&raw);
        assert!(!format!("{clean:?}").contains("secret"));
        assert_eq!(clean.entries()[6], header("Content-Type", "text/html"));
        assert_eq!(clean.entries()[0], header("Cookie", "[redacted]"));
    }

    #[test]
    fn sensitive_headers_never_reach_the_event_store() {
        let mut monitor = active();
        monitor.on_request_started(&start(1, "https://example.com/"));
        monitor.on_response(1, 1, 200, &[header("Set-Cookie", "session=topsecret")]);
        monitor.on_completed(1, 1, 5, false, None);
        let stored = format!("{:?}", monitor.events().collect::<Vec<_>>());
        assert!(!stored.contains("topsecret"));
        assert!(!monitor.export_csv().contains("topsecret"));
    }

    #[test]
    fn credentials_in_a_url_are_not_stored() {
        let mut monitor = active();
        monitor.on_request_started(&start(1, "https://user:hunter2@example.com/path"));
        monitor.on_completed(1, 1, 5, false, None);
        let event = monitor.events().next();
        assert_eq!(
            event.map(|e| e.url.as_str()),
            Some("https://example.com/path")
        );
        assert_eq!(event.map(|e| e.domain.as_str()), Some("example.com"));
    }

    #[test]
    fn unparsable_url_has_unknown_domain() {
        let mut monitor = active();
        monitor.on_request_started(&start(1, "not a url"));
        monitor.on_completed(1, 1, 5, false, None);
        assert_eq!(
            monitor.events().next().map(|e| e.domain.as_str()),
            Some("unknown")
        );
    }

    #[test]
    fn nothing_is_recorded_until_the_user_starts_it() {
        let mut monitor = Monitor::default();
        monitor.on_request_started(&start(1, "https://example.com/"));
        monitor.on_completed(1, 1, 5, false, None);
        assert_eq!(monitor.events().count(), 0);
        assert_eq!(monitor.stats().total_requests, 0);
    }

    #[test]
    fn stopping_ends_recording() {
        let mut monitor = active();
        monitor.on_request_started(&start(1, "https://example.com/"));
        monitor.stop();
        monitor.on_completed(1, 1, 5, false, None);
        assert_eq!(monitor.events().count(), 0);
    }

    #[test]
    fn slow_only_relabels_normal_requests() {
        let mut monitor = active();
        monitor.on_request_started(&start(1, "https://example.com/"));
        monitor.on_completed(1, 1, SLOW_THRESHOLD_MS + 1, false, None);
        monitor.on_request_started(&start(2, "https://example.com/"));
        monitor.on_completed(1, 2, SLOW_THRESHOLD_MS, false, None);
        monitor.on_request_started(&start(3, "https://coinhive.com/"));
        monitor.on_completed(1, 3, SLOW_THRESHOLD_MS + 1, false, None);
        let labels: Vec<Severity> = monitor.events().map(|e| e.severity).collect();
        assert_eq!(labels, [Severity::Threat, Severity::Normal, Severity::Slow]);
        assert_eq!(monitor.stats().slow_requests, 1);
    }

    #[test]
    fn failure_is_recorded_and_counted() {
        let mut monitor = active();
        monitor.on_request_started(&start(1, "https://example.com/"));
        monitor.on_failed(1, 1, 9, "net::ERR_FAILED");
        let event = monitor.events().next();
        assert_eq!(event.map(|e| e.phase), Some(EventPhase::Error));
        assert_eq!(event.map(|e| e.severity), Some(Severity::Failed));
        assert_eq!(monitor.stats().failed_requests, 1);
        assert_eq!(monitor.stats().total_requests, 1);
    }

    #[test]
    fn content_length_sets_size_and_totals() {
        let mut monitor = active();
        monitor.on_request_started(&start(1, "https://example.com/"));
        monitor.on_response(1, 1, 200, &[header("Content-Length", "1234")]);
        monitor.on_completed(1, 1, 5, false, None);
        assert_eq!(monitor.stats().total_bytes, 1234);
    }

    #[test]
    fn hostile_content_length_counts_as_zero() {
        let mut monitor = active();
        monitor.on_request_started(&start(1, "https://example.com/"));
        monitor.on_response(1, 1, 200, &[header("Content-Length", "-1e99")]);
        monitor.on_completed(1, 1, 5, false, None);
        assert_eq!(monitor.stats().total_bytes, 0);
    }

    #[test]
    fn same_request_id_in_two_compartments_stays_separate() {
        let mut monitor = active();
        monitor.on_request_started(&start(1, "https://first.example/"));
        monitor.on_request_started(&RequestStart {
            compartment: 2,
            ..start(1, "https://second.example/")
        });
        monitor.on_completed(2, 1, 5, false, None);
        assert_eq!(
            monitor.events().next().map(|e| e.domain.as_str()),
            Some("second.example")
        );
    }

    #[test]
    fn event_history_is_capped_newest_first() {
        let mut monitor = active();
        let total = u64::try_from(MAX_EVENTS).unwrap_or(0) + 5;
        for id in 0..total {
            monitor.on_request_started(&start(id, "https://example.com/"));
            monitor.on_completed(1, id, 1, false, None);
        }
        assert_eq!(monitor.events().count(), MAX_EVENTS);
        assert_eq!(
            monitor.events().next().map(|e| e.request_id),
            Some(total - 1)
        );
        assert_eq!(monitor.stats().total_requests, total);
    }

    #[test]
    fn a_page_cannot_grow_the_pending_table_without_limit() {
        let mut monitor = active();
        let total = u64::try_from(MAX_PENDING).unwrap_or(0) + 10;
        for id in 0..total {
            monitor.on_request_started(&start(id, "https://example.com/"));
        }
        assert_eq!(monitor.stats().dropped_requests, 10);
    }

    #[test]
    fn clear_deletes_everything() {
        let mut monitor = active();
        monitor.on_request_started(&start(1, "https://example.com/"));
        monitor.on_completed(1, 1, 5, false, None);
        monitor.clear();
        assert_eq!(monitor.events().count(), 0);
        assert_eq!(monitor.stats().total_requests, 0);
    }

    #[test]
    fn csv_quotes_fields_and_doubles_quotes() {
        let mut monitor = active();
        monitor.on_request_started(&start(1, "https://example.com/a\"b,c"));
        monitor.on_completed(1, 1, 5, false, Some("192.0.2.1"));
        let csv = monitor.export_csv();
        let mut lines = csv.lines();
        assert_eq!(
            lines.next(),
            Some("Timestamp,Method,URL,Status,Type,Size,Duration,Severity,Domain,IP,Compartment")
        );
        let row = lines.next().unwrap_or_default();
        assert!(row.contains("\"192.0.2.1\""), "{row}");
        assert!(!row.contains("a\"b"), "{row}");
    }

    #[test]
    fn csv_neutralises_spreadsheet_formulas() {
        let mut monitor = active();
        monitor.on_request_started(&RequestStart {
            method: "=HYPERLINK(\"x\")",
            ..start(1, "https://example.com/")
        });
        monitor.on_completed(1, 1, 5, false, None);
        assert!(monitor.export_csv().contains("\"'=HYPERLINK"));
    }

    #[test]
    fn timestamps_format_as_rfc3339() {
        assert_eq!(iso_timestamp(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_timestamp(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(iso_timestamp(1_793_998_020_123), "2026-11-06T20:47:00.123Z");
    }
}
