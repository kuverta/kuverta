//! The `text/calendar` part of a message, read.
//!
//! An invitation arrives as an iTIP object (RFC 5546): a `VCALENDAR` with a
//! `METHOD`, carrying one `VEVENT`. The method is what makes it a question
//! rather than a notice — `REQUEST` asks, `CANCEL` tells, `REPLY` is
//! somebody else answering a question of yours.
//!
//! This is a parser over bytes a stranger wrote, which is the same footing
//! as the HTML reader: it may refuse, and it may not panic. So nothing here
//! unwraps, every field is optional because every field can be missing in
//! the wild, and the entry loop is bounded rather than trusting the input
//! to end.
//!
//! One thing it will not do is guess a time. A `DTSTART` that names a zone
//! — `TZID=Europe/Berlin:20261104T140000` — is a wall-clock time whose
//! instant depends on a zone database, and an invite card showing the wrong
//! hour is worse than one showing none. So [`When::at`] is filled only when
//! the sender gave an instant that needs no lookup, and [`When::written`]
//! keeps what was actually on the line for everything else.

use serde::{Deserialize, Serialize};

use calcard::{
    common::PartialDateTime,
    icalendar::{
        utils::strip_mailto_scheme, ICalendar, ICalendarComponent, ICalendarComponentType,
        ICalendarParameterName, ICalendarParameterValue, ICalendarParticipationStatus,
        ICalendarProperty,
    },
    Entry, Parser,
};

/// What the sender's program is doing by sending this.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    /// `METHOD:REQUEST` — somebody is asking.
    Request,
    /// `METHOD:CANCEL` — it is off.
    Cancel,
    /// `METHOD:REPLY` — an answer to something this mailbox sent.
    Reply,
    /// `METHOD:COUNTER` — a different time is proposed.
    Counter,
    /// `METHOD:PUBLISH`, anything else, or no method at all: an event sent
    /// as news. A `VCALENDAR` without a `METHOD` is not an iTIP message,
    /// and treating it as one would put a question where none was asked.
    #[default]
    Published,
}

impl Intent {
    fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_uppercase().as_str() {
            "REQUEST" => Self::Request,
            "CANCEL" => Self::Cancel,
            "REPLY" => Self::Reply,
            "COUNTER" => Self::Counter,
            _ => Self::Published,
        }
    }

    /// Whether answering it would mean anything.
    pub fn asks_an_answer(self) -> bool {
        matches!(self, Self::Request | Self::Counter)
    }
}

/// When something happens, as the sender wrote it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct When {
    /// Epoch seconds — only when the sender gave an instant that needs no
    /// zone database: a UTC time, or one carrying its own offset. `None`
    /// for a wall-clock time named by zone, and for a date.
    pub at: Option<i64>,
    /// What was on the line, as digits: `2026-11-04 14:00`, or `2026-11-04`
    /// for a whole day. Always set, which is why the card can show
    /// something even when `at` is empty.
    pub written: String,
    /// The `TZID` the sender named, when they named one.
    pub zone: Option<String>,
    /// A whole day, with no time of day on the line.
    pub all_day: bool,
}

/// How someone answered, or has yet to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Answer {
    Yes,
    No,
    Maybe,
    /// `NEEDS-ACTION`: asked, not yet answered.
    Waiting,
    /// Passed to somebody else.
    Delegated,
}

/// Somebody on the invitation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    /// The address, with `mailto:` taken off.
    pub address: Option<String>,
    /// The `CN` parameter: what the sender's program calls them.
    pub name: Option<String>,
    /// Their `PARTSTAT`, where the sender filled one in.
    pub answer: Option<Answer>,
}

/// An invitation, as much of one as the sender sent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Invite {
    pub intent: Intent,
    /// `UID`: the event's own identity, which is what makes the same
    /// invitation arriving twice recognisable as one thing.
    pub uid: Option<String>,
    /// `SEQUENCE`: which revision of the event this is. An answer has to
    /// quote it back, so it is kept even though nothing answers yet.
    pub sequence: Option<i64>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub begins: Option<When>,
    pub ends: Option<When>,
    pub organizer: Option<Person>,
    pub attendees: Vec<Person>,
    /// Whether the event repeats. Kept as a flag and not expanded: what the
    /// repetition comes to is a calendar's question, not a mail client's.
    pub repeats: bool,
}

/// How many entries are read from one part before giving up.
///
/// An invitation carries one `VCALENDAR`. The bound is here so that input
/// which never reaches its end cannot hold the parser forever.
const MOST_ENTRIES: usize = 64;

/// The invitation in a `text/calendar` part, or `None` if there is none in
/// it that can be read.
pub fn read(text: &str) -> Option<Invite> {
    let mut parser = Parser::new(text);
    for _ in 0..MOST_ENTRIES {
        match parser.entry() {
            Entry::ICalendar(calendar) => {
                if let Some(invite) = from_calendar(&calendar) {
                    return Some(invite);
                }
            }
            Entry::Eof => return None,
            // A line the sender's program got wrong is skipped rather than
            // taking the invitation down with it.
            _ => {}
        }
    }
    None
}

fn from_calendar(calendar: &ICalendar) -> Option<Invite> {
    let event = calendar
        .components
        .iter()
        .find(|component| component.component_type == ICalendarComponentType::VEvent)?;

    let intent = calendar
        .components
        .iter()
        .find(|component| component.component_type == ICalendarComponentType::VCalendar)
        .and_then(|component| text_of(component, &ICalendarProperty::Method))
        .map(|name| Intent::from_name(&name))
        .unwrap_or_default();

    Some(Invite {
        intent,
        uid: text_of(event, &ICalendarProperty::Uid),
        sequence: event
            .property(&ICalendarProperty::Sequence)
            .and_then(|entry| entry.values.first())
            .and_then(|value| value.as_integer()),
        summary: text_of(event, &ICalendarProperty::Summary),
        description: text_of(event, &ICalendarProperty::Description),
        location: text_of(event, &ICalendarProperty::Location),
        begins: when_of(event, &ICalendarProperty::Dtstart),
        ends: when_of(event, &ICalendarProperty::Dtend),
        organizer: person_of(event, &ICalendarProperty::Organizer).next(),
        attendees: person_of(event, &ICalendarProperty::Attendee).collect(),
        repeats: event.has_property(&ICalendarProperty::Rrule),
    })
}

fn text_of(component: &ICalendarComponent, property: &ICalendarProperty) -> Option<String> {
    let text = component
        .property(property)?
        .values
        .first()?
        .as_text()?
        .trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn when_of(component: &ICalendarComponent, property: &ICalendarProperty) -> Option<When> {
    let entry = component.property(property)?;
    let stamp = entry.values.first()?.as_partial_date_time()?;
    let zone = match entry.parameter(&ICalendarParameterName::Tzid) {
        Some(ICalendarParameterValue::Text(name)) => Some(name.clone()),
        _ => None,
    };
    let all_day = !stamp.has_time();
    Some(When {
        // A zone named by `TZID` is a wall-clock time: what instant it is
        // depends on a database this crate does not carry. A date is not an
        // instant at all.
        at: (zone.is_none() && !all_day)
            .then(|| stamp.to_timestamp())
            .flatten(),
        written: written(stamp),
        zone,
        all_day,
    })
}

/// The date and time as digits, leaving out what the sender left out.
fn written(stamp: &PartialDateTime) -> String {
    let date = match (stamp.year, stamp.month, stamp.day) {
        (Some(year), Some(month), Some(day)) => format!("{year:04}-{month:02}-{day:02}"),
        _ => String::new(),
    };
    match (stamp.hour, stamp.minute) {
        (Some(hour), Some(minute)) if date.is_empty() => format!("{hour:02}:{minute:02}"),
        (Some(hour), Some(minute)) => format!("{date} {hour:02}:{minute:02}"),
        _ => date,
    }
}

fn person_of<'a>(
    component: &'a ICalendarComponent,
    property: &'a ICalendarProperty,
) -> impl Iterator<Item = Person> + 'a {
    component.properties(property).map(|entry| {
        let address = entry
            .values
            .first()
            .and_then(|value| value.as_text())
            .map(|value| strip_mailto_scheme(value.trim()).trim().to_string())
            .filter(|address| !address.is_empty());
        let name = match entry.parameter(&ICalendarParameterName::Cn) {
            Some(ICalendarParameterValue::Text(name)) if !name.trim().is_empty() => {
                Some(name.trim().to_string())
            }
            _ => None,
        };
        let answer = match entry.parameter(&ICalendarParameterName::Partstat) {
            Some(ICalendarParameterValue::Partstat(status)) => match status {
                ICalendarParticipationStatus::Accepted => Some(Answer::Yes),
                ICalendarParticipationStatus::Declined => Some(Answer::No),
                ICalendarParticipationStatus::Tentative => Some(Answer::Maybe),
                ICalendarParticipationStatus::NeedsAction => Some(Answer::Waiting),
                ICalendarParticipationStatus::Delegated => Some(Answer::Delegated),
                _ => None,
            },
            _ => None,
        };
        Person {
            address,
            name,
            answer,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a calendar program sends, with the line endings mail uses.
    fn part(lines: &[&str]) -> String {
        format!("{}\r\n", lines.join("\r\n"))
    }

    fn request() -> String {
        part(&[
            "BEGIN:VCALENDAR",
            "PRODID:-//Example//Kalender//DE",
            "VERSION:2.0",
            "METHOD:REQUEST",
            "BEGIN:VEVENT",
            "UID:4f8e-2026-11",
            "SEQUENCE:2",
            "DTSTAMP:20261030T101500Z",
            "DTSTART;TZID=Europe/Berlin:20261104T140000",
            "DTEND;TZID=Europe/Berlin:20261104T150000",
            "SUMMARY:Quartalsbesprechung",
            "LOCATION:Besprechungsraum 2",
            "ORGANIZER;CN=Erika Mustermann:mailto:erika@example.de",
            "ATTENDEE;CN=Hans Beispiel;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:hans@example.org",
            "ATTENDEE;PARTSTAT=ACCEPTED:mailto:erika@example.de",
            "END:VEVENT",
            "END:VCALENDAR",
        ])
    }

    #[test]
    fn an_invitation_is_read_as_the_sender_wrote_it() {
        let invite = read(&request()).expect("a REQUEST is an invitation");
        assert_eq!(invite.intent, Intent::Request);
        assert_eq!(invite.uid.as_deref(), Some("4f8e-2026-11"));
        assert_eq!(invite.sequence, Some(2));
        assert_eq!(invite.summary.as_deref(), Some("Quartalsbesprechung"));
        assert_eq!(invite.location.as_deref(), Some("Besprechungsraum 2"));
        assert!(!invite.repeats, "this one has no RRULE");

        let organizer = invite.organizer.expect("an ORGANIZER was given");
        assert_eq!(organizer.address.as_deref(), Some("erika@example.de"));
        assert_eq!(organizer.name.as_deref(), Some("Erika Mustermann"));

        assert_eq!(invite.attendees.len(), 2);
        assert_eq!(
            invite.attendees[0].address.as_deref(),
            Some("hans@example.org")
        );
        assert_eq!(invite.attendees[0].name.as_deref(), Some("Hans Beispiel"));
        assert_eq!(invite.attendees[0].answer, Some(Answer::Waiting));
        assert_eq!(invite.attendees[1].answer, Some(Answer::Yes));
        assert_eq!(
            invite.attendees[1].name, None,
            "a CN nobody wrote is not invented"
        );
    }

    #[test]
    fn a_time_named_by_zone_is_not_turned_into_an_instant() {
        let invite = read(&request()).unwrap();
        let begins = invite.begins.expect("a DTSTART was given");
        assert_eq!(
            begins.at, None,
            "the instant depends on a zone database this does not carry"
        );
        assert_eq!(begins.written, "2026-11-04 14:00");
        assert_eq!(begins.zone.as_deref(), Some("Europe/Berlin"));
        assert!(!begins.all_day);
    }

    #[test]
    fn a_utc_time_is_an_instant() {
        let text = part(&[
            "BEGIN:VCALENDAR",
            "METHOD:REQUEST",
            "BEGIN:VEVENT",
            "UID:z",
            "DTSTART:20261104T140000Z",
            "DTEND:20261104T150000Z",
            "SUMMARY:Abstimmung",
            "END:VEVENT",
            "END:VCALENDAR",
        ]);
        let invite = read(&text).unwrap();
        assert_eq!(invite.begins.as_ref().unwrap().at, Some(1_793_800_800));
        assert_eq!(invite.ends.as_ref().unwrap().at, Some(1_793_804_400));
        assert_eq!(invite.begins.as_ref().unwrap().written, "2026-11-04 14:00");
    }

    #[test]
    fn a_whole_day_has_a_date_and_no_instant() {
        let text = part(&[
            "BEGIN:VCALENDAR",
            "METHOD:REQUEST",
            "BEGIN:VEVENT",
            "UID:z",
            "DTSTART;VALUE=DATE:20261104",
            "SUMMARY:Betriebsausflug",
            "END:VEVENT",
            "END:VCALENDAR",
        ]);
        let begins = read(&text).unwrap().begins.unwrap();
        assert!(begins.all_day);
        assert_eq!(begins.at, None);
        assert_eq!(begins.written, "2026-11-04");
    }

    #[test]
    fn a_repeating_event_says_so_without_being_expanded() {
        let text = part(&[
            "BEGIN:VCALENDAR",
            "METHOD:REQUEST",
            "BEGIN:VEVENT",
            "UID:z",
            "DTSTART:20261104T140000Z",
            "RRULE:FREQ=WEEKLY;COUNT=12",
            "SUMMARY:Wochenrunde",
            "END:VEVENT",
            "END:VCALENDAR",
        ]);
        assert!(read(&text).unwrap().repeats);
    }

    #[test]
    fn a_cancellation_is_not_a_question() {
        let text = request().replace("METHOD:REQUEST", "METHOD:CANCEL");
        let invite = read(&text).unwrap();
        assert_eq!(invite.intent, Intent::Cancel);
        assert!(!invite.intent.asks_an_answer());
    }

    #[test]
    fn a_calendar_without_a_method_is_news_and_not_a_question() {
        let text = request().replace("METHOD:REQUEST\r\n", "");
        let invite = read(&text).unwrap();
        assert_eq!(invite.intent, Intent::Published);
        assert!(!invite.intent.asks_an_answer());
    }

    #[test]
    fn what_is_not_an_event_is_not_an_invitation() {
        let text = part(&[
            "BEGIN:VCALENDAR",
            "METHOD:REQUEST",
            "BEGIN:VTODO",
            "UID:z",
            "SUMMARY:Rechnung zahlen",
            "END:VTODO",
            "END:VCALENDAR",
        ]);
        assert_eq!(
            read(&text),
            None,
            "a VTODO is a todo, and nothing reads one yet"
        );
    }

    #[test]
    fn nonsense_is_refused_rather_than_guessed_at() {
        for text in [
            "",
            "BEGIN:VCALENDAR",
            "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\n",
            "BEGIN:VEVENT\r\nDTSTART:nicht-ein-datum\r\nEND:VEVENT\r\n",
            "\u{feff}BEGIN:VCALENDAR\r\n\u{0}\u{1}\u{2}\r\nEND:VCALENDAR\r\n",
            &"BEGIN:VCALENDAR\r\n".repeat(500),
        ] {
            let _ = read(text);
        }
    }

    #[test]
    fn an_event_with_nothing_in_it_still_reads() {
        let text = part(&[
            "BEGIN:VCALENDAR",
            "BEGIN:VEVENT",
            "END:VEVENT",
            "END:VCALENDAR",
        ]);
        let invite = read(&text).expect("an empty VEVENT is still an event");
        assert_eq!(invite.uid, None);
        assert_eq!(invite.summary, None);
        assert_eq!(invite.begins, None);
        assert!(invite.attendees.is_empty());
    }
}
