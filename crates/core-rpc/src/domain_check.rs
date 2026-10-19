//! Read-only DNS diagnostics for an outgoing mail domain. These are publication
//! checks, not an SPF evaluation or a DKIM signature/DMARC alignment verdict.
use std::{collections::BTreeMap, time::Duration};

use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct Report {
    pub domain: String,
    pub checks: Vec<Check>,
}

#[derive(Debug, Serialize)]
pub struct Check {
    pub protocol: &'static str,
    pub name: String,
    // "present" deliberately does not mean authenticated or correctly sending.
    pub status: &'static str,
    pub notes: Vec<&'static str>,
    pub records: Vec<String>,
}

impl Check {
    fn new(protocol: &'static str, name: String, records: Vec<String>) -> Self {
        Self {
            protocol,
            name,
            status: "present",
            notes: vec![],
            records,
        }
    }

    fn note(&mut self, status: &'static str, note: &'static str) {
        if self.status != "error" {
            self.status = status;
        }
        self.notes.push(note);
    }
}

/// Accept a domain, never a URL/address/IP. IDNs are converted before DNS use.
fn domain_name(input: &str) -> Result<String, String> {
    let input = input.trim().trim_end_matches('.');
    let url::Host::Domain(domain) = url::Host::parse(input).map_err(|_| "Invalid mail domain")?
    else {
        return Err("Enter a mail domain, not an IP address".into());
    };
    if !domain.contains('.') || !valid_labels(&domain) {
        return Err("Invalid mail domain".into());
    }
    Ok(domain)
}

fn valid_labels(name: &str) -> bool {
    name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

#[derive(Deserialize)]
struct DnsResponse {
    #[serde(rename = "Status")]
    status: u16,
    #[serde(rename = "TC", default)]
    truncated: bool,
    #[serde(rename = "Answer", default)]
    answers: Vec<DnsAnswer>,
}

#[derive(Deserialize)]
struct DnsAnswer {
    #[serde(rename = "type")]
    kind: u16,
    data: String,
}

fn dns_records(reply: DnsResponse) -> Result<Vec<String>, &'static str> {
    if reply.truncated || !matches!(reply.status, 0 | 3) {
        return Err("DNS lookup failed. Retry later; this does not mean the record is missing.");
    }
    if reply.status == 3 {
        return Ok(vec![]);
    }
    reply
        .answers
        .into_iter()
        .filter(|a| a.kind == 16)
        .map(|a| txt_value(&a.data))
        .collect()
}

/// DNS presentation format can split one TXT RR into multiple quoted strings.
/// Joining different RRs would incorrectly hide duplicate SPF/DMARC records.
fn txt_value(input: &str) -> Result<String, &'static str> {
    let invalid = "The DNS TXT response could not be read.";
    if input.len() > 16_384 {
        return Err(invalid);
    }
    // Google's JSON endpoint can return already-decoded TXT content. Older
    // responses and other DNS presentation encoders quote individual chunks.
    // Do not require presentation quotes or unescape already-decoded content.
    if !input.trim_start().starts_with('"') {
        return Ok(input.to_owned());
    }
    let bytes = input.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    let mut chunks = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i == bytes.len() {
            break;
        }
        if bytes[i] != b'"' {
            return Err(invalid);
        }
        chunks += 1;
        i += 1;
        while i < bytes.len() && bytes[i] != b'"' {
            if bytes[i] == b'\\' {
                i += 1;
                if i == bytes.len() {
                    return Err(invalid);
                }
                if bytes[i].is_ascii_digit() {
                    let digits = bytes.get(i..i + 3).ok_or(invalid)?;
                    if !digits.iter().all(u8::is_ascii_digit) {
                        return Err(invalid);
                    }
                    let value = (digits[0] - b'0') as u16 * 100
                        + (digits[1] - b'0') as u16 * 10
                        + (digits[2] - b'0') as u16;
                    out.push(u8::try_from(value).map_err(|_| invalid)?);
                    i += 3;
                    continue;
                }
            }
            out.push(bytes[i]);
            i += 1;
        }
        if i == bytes.len() {
            return Err(invalid);
        }
        i += 1;
    }
    if chunks == 0 {
        return Err(invalid);
    }
    String::from_utf8(out).map_err(|_| invalid)
}

async fn lookup(client: &reqwest::Client, name: &str) -> Result<Vec<String>, &'static str> {
    let unavailable = "DNS lookup failed. Retry later; this does not mean the record is missing.";
    let mut url = url::Url::parse("https://dns.google/resolve").expect("fixed resolver URL");
    url.query_pairs_mut().extend_pairs([
        ("name", name),
        ("type", "TXT"),
        ("edns_client_subnet", "0.0.0.0/0"),
    ]);
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|_| unavailable)?
        .error_for_status()
        .map_err(|_| unavailable)?;
    let mut data = vec![];
    while let Some(chunk) = response.chunk().await.map_err(|_| unavailable)? {
        if data.len() + chunk.len() > 65_536 {
            return Err(unavailable);
        }
        data.extend_from_slice(&chunk);
    }
    dns_records(serde_json::from_slice(&data).map_err(|_| unavailable)?)
}

fn tags(record: &str) -> Result<BTreeMap<&str, &str>, &'static str> {
    let mut tags = BTreeMap::new();
    for part in record.trim_end_matches([';', ' ', '\t']).split(';') {
        let (name, value) = part
            .trim()
            .split_once('=')
            .ok_or("Malformed or duplicate tags in the DNS record.")?;
        let name = name.trim();
        if name.is_empty()
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || tags.insert(name, value.trim()).is_some()
        {
            return Err("Malformed or duplicate tags in the DNS record.");
        }
    }
    Ok(tags)
}

fn spf(name: String, records: Vec<String>) -> Check {
    let records: Vec<_> = records
        .into_iter()
        .filter(|r| {
            r.split_ascii_whitespace()
                .next()
                .is_some_and(|v| v.eq_ignore_ascii_case("v=spf1"))
        })
        .collect();
    let mut result = Check::new("SPF", name, records);
    if result.records.is_empty() {
        result.note("missing", "No SPF record was found at this domain.");
    } else if result.records.len() > 1 {
        result.note(
            "error",
            "Multiple SPF records are published. Publish one SPF policy only.",
        );
    } else {
        let record = result.records[0].to_ascii_lowercase();
        let terms: Vec<_> = record.split_ascii_whitespace().skip(1).collect();
        let all = terms
            .iter()
            .find(|t| t.trim_start_matches(['+', '-', '~', '?']) == "all");
        if all.is_some_and(|t| matches!(*t, "all" | "+all")) {
            result.note(
                "error",
                "SPF +all permits every sender and offers no protection.",
            );
        }
        if all == Some(&"?all") {
            result.note("warning", "SPF ends with a neutral policy (?all).");
        }
        if !terms.iter().any(|t| {
            t.trim_start_matches(['+', '-', '~', '?']) == "all" || t.starts_with("redirect=")
        }) {
            result.note(
                "warning",
                "SPF has no final all mechanism or redirect. Unmatched senders remain neutral.",
            );
        }
        result.notes.push("SPF is published. Sending IPs, include/redirect chains and their lookup limits still need a real sending check.");
    }
    result
}

fn dmarc(name: String, records: Vec<String>) -> Check {
    let records: Vec<_> = records
        .into_iter()
        .filter(|r| {
            r.split(';')
                .next()
                .and_then(|v| v.trim().split_once('='))
                .is_some_and(|(k, v)| k.trim() == "v" && v.trim() == "DMARC1")
        })
        .collect();
    let mut result = Check::new("DMARC", name, records);
    if result.records.is_empty() {
        result.note("missing", "No DMARC record at this exact domain. A parent-domain policy may apply; inheritance is not checked here.");
    } else if result.records.len() > 1 {
        result.note(
            "error",
            "Multiple DMARC records are published at this name.",
        );
    } else {
        let record = result.records[0].clone();
        match tags(&record) {
            Err(note) => result.note("error", note),
            Ok(values) => {
                if !values
                    .get("p")
                    .is_some_and(|p| matches!(*p, "none" | "quarantine" | "reject"))
                    || ["sp", "np"].iter().any(|k| {
                        values
                            .get(k)
                            .is_some_and(|p| !matches!(*p, "none" | "quarantine" | "reject"))
                    })
                {
                    result.note(
                        "error",
                        "DMARC needs a valid p policy; sp and np must also be valid when present.",
                    );
                }
                if ["adkim", "aspf"]
                    .iter()
                    .any(|k| values.get(k).is_some_and(|v| !matches!(*v, "r" | "s")))
                {
                    result.note("error", "DMARC alignment mode must be r or s.");
                }
                if values.get("p") == Some(&"none") {
                    result.note(
                        "warning",
                        "DMARC p=none monitors mail but does not request quarantine or rejection.",
                    );
                }
                if !values.contains_key("rua") {
                    result.notes.push(
                        "No aggregate report address (rua) is configured; reporting is optional.",
                    );
                }
                result.notes.push("A published DMARC policy does not prove that outgoing messages pass SPF/DKIM alignment.");
            }
        }
    }
    result
}

fn dkim(name: String, records: Vec<String>) -> Check {
    let mut result = Check::new("DKIM", name, records);
    if result.records.is_empty() {
        result.note("missing", "No DKIM TXT key was found for this selector. Check the selector with your mail provider.");
    } else if result.records.len() > 1 {
        result.note(
            "error",
            "Multiple DKIM key records were found for this selector.",
        );
    } else {
        let record = result.records[0].clone();
        match tags(&record) {
            Err(note) => result.note("error", note),
            Ok(values) => {
                if values.get("v").is_some_and(|v| *v != "DKIM1")
                    || (values.contains_key("v")
                        && !record
                            .split(';')
                            .next()
                            .and_then(|v| v.trim().split_once('='))
                            .is_some_and(|(k, _)| k.trim() == "v"))
                {
                    result.note("error", "Invalid DKIM version tag or tag order.");
                }
                match values.get("p") {
                    None => result.note("error", "The DKIM public key (p) is missing."),
                    Some(key) if key.is_empty() => {
                        result.note("error", "This DKIM key has been revoked (empty p).")
                    }
                    Some(key) => {
                        let compact: String =
                            key.chars().filter(|c| !c.is_ascii_whitespace()).collect();
                        if STANDARD.decode(compact).is_err() {
                            result.note("error", "The DKIM public key is not valid Base64.");
                        }
                    }
                }
                if values
                    .get("k")
                    .is_some_and(|k| !matches!(*k, "rsa" | "ed25519"))
                {
                    result.note(
                        "warning",
                        "The DKIM key type is not recognised by this check.",
                    );
                }
                if values
                    .get("t")
                    .is_some_and(|v| v.split(':').any(|f| f.trim() == "y"))
                {
                    result.note("warning", "DKIM is published in testing mode (t=y).");
                }
                result.notes.push("A key is published. Key strength and actual signing must be verified on a sent message.");
            }
        }
    }
    result
}

pub async fn check(domain: &str, selector: Option<&str>) -> Result<Report, String> {
    let domain = domain_name(domain)?;
    let selector = selector.map(str::trim).filter(|s| !s.is_empty());
    if selector.is_some_and(|s| !valid_labels(s) || s.len() + domain.len() + 12 > 253) {
        return Err("Invalid DKIM selector".into());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "DNS client unavailable")?;
    let mut checks = vec![];
    for (protocol, name) in [
        ("SPF", domain.clone()),
        ("DMARC", format!("_dmarc.{domain}")),
    ] {
        let result = match lookup(&client, &name).await {
            Ok(records) if protocol == "SPF" => spf(name, records),
            Ok(records) => dmarc(name, records),
            Err(note) => {
                let mut r = Check::new(protocol, name, vec![]);
                r.note("unavailable", note);
                r
            }
        };
        checks.push(result);
    }
    let mut key = Check::new("DKIM", String::new(), vec![]);
    if let Some(selector) = selector {
        let name = format!("{selector}._domainkey.{domain}");
        key = match lookup(&client, &name).await {
            Ok(records) => dkim(name, records),
            Err(note) => {
                let mut r = Check::new("DKIM", name, vec![]);
                r.note("unavailable", note);
                r
            }
        };
    } else {
        key.note("not_checked", "Enter the DKIM selector supplied by your mail provider. Without it, DKIM cannot be checked.");
    }
    checks.push(key);
    Ok(Report { domain, checks })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn dns_names_and_selectors_are_not_urls_or_addresses() {
        assert_eq!(domain_name("BÜCHER.de.").unwrap(), "xn--bcher-kva.de");
        for input in [
            "a@b.de",
            "https://example.com",
            "127.0.0.1",
            "a..de",
            "-a.de",
            "a/b.de",
            "localhost",
        ] {
            assert!(domain_name(input).is_err(), "{input}");
        }
        assert!(valid_labels("selector.2026"));
        assert!(!valid_labels("selector/_domainkey"));
    }

    #[test]
    fn txt_chunks_join_but_records_remain_separate() {
        assert_eq!(
            txt_value(r#""v=spf1 " "include:example.org -all""#).unwrap(),
            "v=spf1 include:example.org -all"
        );
        assert_eq!(txt_value(r#""a\032b\"c""#).unwrap(), "a b\"c");
        assert!(txt_value(r#""unterminated"#).is_err());
        assert!(txt_value(r#""\999""#).is_err());
        let response: DnsResponse = serde_json::from_str(r#"{"Status":0,"Answer":[{"type":5,"data":"alias."},{"type":16,"data":"\"v=spf1 -all\""},{"type":16,"data":"\"v=spf1 ~all\""}]}"#).unwrap();
        assert_eq!(dns_records(response).unwrap().len(), 2);
    }

    #[test]
    fn google_plain_txt_responses_reach_the_publication_checks() {
        let reply: DnsResponse = serde_json::from_str(
            r#"{"Status":0,"Answer":[{"type":16,"data":"v=spf1 ip4:192.0.2.1 -all"}]}"#,
        )
        .unwrap();
        let result = spf("example.com".into(), dns_records(reply).unwrap());
        assert_eq!(result.status, "present");
        assert_eq!(result.records, vec!["v=spf1 ip4:192.0.2.1 -all"]);
        let reply: DnsResponse = serde_json::from_str(r#"{"Status":0,"Answer":[{"type":16,"data":"v=DMARC1;p=none;rua=mailto:reports@example.com;"}]}"#).unwrap();
        assert_eq!(
            dmarc("_dmarc.example.com".into(), dns_records(reply).unwrap()).status,
            "warning"
        );
        assert_eq!(
            txt_value(r"a\032b").unwrap(),
            r"a\032b",
            "decoded TXT is not unescaped twice"
        );
    }

    #[test]
    fn dns_failure_is_not_reported_as_missing() {
        for status in [2, 5] {
            assert!(dns_records(DnsResponse {
                status,
                truncated: false,
                answers: vec![]
            })
            .is_err());
        }
        assert!(dns_records(DnsResponse {
            status: 0,
            truncated: true,
            answers: vec![]
        })
        .is_err());
        assert!(dns_records(DnsResponse {
            status: 3,
            truncated: false,
            answers: vec![]
        })
        .unwrap()
        .is_empty());
    }

    #[test]
    fn spf_diagnostics_distinguish_presence_from_errors() {
        assert_eq!(
            spf(
                "x.de".into(),
                records(&["unrelated", "v=spf1 include:x.de -all"])
            )
            .status,
            "present"
        );
        assert_eq!(
            spf("x.de".into(), records(&["v=spf1 +all"])).status,
            "error"
        );
        assert_eq!(
            spf("x.de".into(), records(&["v=spf1 -all", "v=spf1 ~all"])).status,
            "error"
        );
        assert_eq!(
            spf("x.de".into(), records(&["v=spf10 -all"])).status,
            "missing"
        );
    }

    #[test]
    fn dmarc_checks_policy_duplicates_and_monitoring() {
        for record in [
            "v=DMARC1; p=wrong",
            "v=DMARC1; p=reject; p=none",
            "v=DMARC1; p=reject; adkim=x",
        ] {
            assert_eq!(
                dmarc("_dmarc.x.de".into(), records(&[record])).status,
                "error"
            );
        }
        assert_eq!(
            dmarc("x".into(), records(&["v=DMARC1; p=none"])).status,
            "warning"
        );
        assert_eq!(
            dmarc("x".into(), records(&["v=DMARC1; p=reject; adkim=s"])).status,
            "present"
        );
        assert_eq!(dmarc("x".into(), vec![]).notes[0], "No DMARC record at this exact domain. A parent-domain policy may apply; inheritance is not checked here.");
    }

    #[test]
    fn dkim_requires_a_non_revoked_key_but_version_is_optional() {
        assert_eq!(
            dkim("x".into(), records(&["k=rsa; p=YWJj"])).status,
            "present"
        );
        for record in [
            "v=DKIM1; p=",
            "v=DKIM1; k=rsa",
            "v=DKIM1; p=?!",
            "v=DKIM1; p=YWJj; p=YWJj",
        ] {
            assert_eq!(dkim("x".into(), records(&[record])).status, "error");
        }
        assert_eq!(
            dkim("x".into(), records(&["v=DKIM1; p=YWJj; t=y"])).status,
            "warning"
        );
    }
}
