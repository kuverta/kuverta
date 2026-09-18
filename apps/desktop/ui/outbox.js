// Sending later: choosing when, and the outbox of what is waiting.
//
// When is chosen from a few common answers, typed ("tomorrow 8pm", "monday
// 9:00", "in 2 hours", "morgen 20 Uhr"), or picked on a calendar. Whatever
// way it is chosen, it is read back in full — "Tuesday 22 September at 20:00"
// — before anything is scheduled, because "8" meaning 08:00 when 20:00 was
// meant is exactly the mistake a person would not notice until too late.
//
// The core checks the message when it is scheduled and sends it from its own
// thread when it is due; this only asks and shows.

const scheduleMenu = el("schedule-menu");
const scheduleText = el("schedule-text");
const scheduleAt = el("schedule-at");
const scheduleRead = el("schedule-read");
const scheduleConfirm = el("schedule-confirm");
const outboxSheet = el("outbox-sheet");

/// The time being offered, as a Date, or null.
let scheduleWhen = null;

const whenFormat = new Intl.DateTimeFormat(undefined, {
  weekday: "long",
  day: "numeric",
  month: "long",
  hour: "2-digit",
  minute: "2-digit",
});

const shortWhen = new Intl.DateTimeFormat(undefined, {
  weekday: "short",
  day: "numeric",
  month: "short",
  hour: "2-digit",
  minute: "2-digit",
});

// -- reading a time --------------------------------------------------------

const WEEKDAYS = {
  sunday: 0, sun: 0, sonntag: 0, so: 0,
  monday: 1, mon: 1, montag: 1, mo: 1,
  tuesday: 2, tue: 2, tues: 2, dienstag: 2, di: 2,
  wednesday: 3, wed: 3, mittwoch: 3, mi: 3,
  thursday: 4, thu: 4, thur: 4, thurs: 4, donnerstag: 4, do: 4,
  friday: 5, fri: 5, freitag: 5, fr: 5,
  saturday: 6, sat: 6, samstag: 6, sonnabend: 6, sa: 6,
};

const MONTHS = {
  jan: 0, january: 0, januar: 0,
  feb: 1, february: 1, februar: 1,
  mar: 2, march: 2, mär: 2, maerz: 2, märz: 2,
  apr: 3, april: 3,
  may: 4, mai: 4,
  jun: 5, june: 5, juni: 5,
  jul: 6, july: 6, juli: 6,
  aug: 7, august: 7,
  sep: 8, sept: 8, september: 8,
  oct: 9, october: 9, okt: 9, oktober: 9,
  nov: 10, november: 10,
  dec: 11, december: 11, dez: 11, dezember: 11,
};

/// Parts of the day, when no hour is given.
const DAYPARTS = {
  morning: 8, morgens: 8, früh: 8,
  noon: 12, midday: 12, mittag: 12, mittags: 12,
  afternoon: 15, nachmittag: 15, nachmittags: 15,
  evening: 19, abend: 19, abends: 19,
  tonight: 20, night: 21, nacht: 21,
};

/// Reads a time a person typed, relative to `now`. Returns a Date, or null
/// when it is not sure — a guess would be worse than asking again.
function parseWhen(input, now = new Date()) {
  let text = input.trim().toLowerCase().replace(/\s+/g, " ");
  if (!text) return null;
  // Words that only join the rest: "at 8", "um 8", "next monday". German
  // "am" ("am Montag") too, but only before a word — "8 am" is a time.
  text = text
    .replace(/[,]/g, " ")
    .replace(/(^|\s)am (?=[a-zäöü])/g, " ")
    .replace(/(^|\s)(at|um|on|next|nächsten|nächster|naechsten|kommenden)(?=\s|$)/g, " ");
  text = text.replace(/\s+/g, " ").trim();

  // "in 2 hours", "in 30 min", "in 3 days", "in 2 stunden"
  const relative = text.match(/^in (\d+(?:[.,]\d+)?|an?|einer?|einem) ?(minutes?|mins?|m|hours?|hrs?|h|stunden?|days?|d|tagen?|weeks?|w|wochen?)$/);
  if (relative) {
    const amount = /^\d/.test(relative[1]) ? parseFloat(relative[1].replace(",", ".")) : 1;
    const unit = relative[2];
    const minutes = /^(minutes?|mins?|m)$/.test(unit)
      ? amount
      : /^(hours?|hrs?|h|stunden?)$/.test(unit)
        ? amount * 60
        : /^(days?|d|tagen?)$/.test(unit)
          ? amount * 1440
          : amount * 10080;
    return new Date(now.getTime() + minutes * 60000);
  }

  let day = null; // a Date at midnight
  let hour = null;
  let minute = 0;
  const midnight = (date) => new Date(date.getFullYear(), date.getMonth(), date.getDate());
  const addDays = (date, n) => new Date(date.getFullYear(), date.getMonth(), date.getDate() + n);

  const words = text.split(" ");
  const rest = [];
  for (let i = 0; i < words.length; i++) {
    const word = words[i];
    if (["today", "heute"].includes(word)) day = midnight(now);
    // "Montag morgen" is Monday morning; "morgen" alone is tomorrow.
    else if (word === "morgen" && day) hour ??= 8;
    else if (["tomorrow", "tmrw", "morgen"].includes(word)) day = addDays(midnight(now), 1);
    else if (["übermorgen", "uebermorgen"].includes(word)) day = addDays(midnight(now), 2);
    else if (word in WEEKDAYS && !(word === "so" && words.length === 1)) {
      const want = WEEKDAYS[word];
      let ahead = (want - now.getDay() + 7) % 7;
      if (ahead === 0) ahead = 7; // "monday" on a Monday means the next one
      day = addDays(midnight(now), ahead);
    } else if (word in DAYPARTS) {
      if (hour === null) hour = DAYPARTS[word];
      if (word === "tonight" && !day) day = midnight(now);
    } else rest.push(word);
  }

  let remaining = rest.join(" ");

  // Dates: 2026-09-20, 20.09.2026, 20.09., 20/9, 20 sep, sep 20
  let match;
  if ((match = remaining.match(/\b(\d{4})-(\d{1,2})-(\d{1,2})\b/))) {
    day = new Date(+match[1], +match[2] - 1, +match[3]);
    remaining = remaining.replace(match[0], " ");
  } else if ((match = remaining.match(/\b(\d{1,2})\.(\d{1,2})\.(\d{2,4})?(?!\d)/))) {
    const year = match[3] ? (match[3].length === 2 ? 2000 + +match[3] : +match[3]) : now.getFullYear();
    day = new Date(year, +match[2] - 1, +match[1]);
    if (!match[3] && day < midnight(now)) day.setFullYear(year + 1);
    remaining = remaining.replace(match[0], " ");
  } else if ((match = remaining.match(/\b(\d{1,2}) ?([a-zäö]{3,9})\.?\b/)) && match[2] in MONTHS) {
    day = new Date(now.getFullYear(), MONTHS[match[2]], +match[1]);
    if (day < midnight(now)) day.setFullYear(now.getFullYear() + 1);
    remaining = remaining.replace(match[0], " ");
  } else if ((match = remaining.match(/\b([a-zäö]{3,9})\.? (\d{1,2})(?:st|nd|rd|th)?\b/)) && match[1] in MONTHS) {
    day = new Date(now.getFullYear(), MONTHS[match[1]], +match[2]);
    if (day < midnight(now)) day.setFullYear(now.getFullYear() + 1);
    remaining = remaining.replace(match[0], " ");
  }

  // Times: 8pm, 8:30 pm, 20:00, 20.30 uhr, 8 uhr, 20h
  remaining = remaining.trim();
  if ((match = remaining.match(/\b(\d{1,2})(?:[:.](\d{2}))? ?(am|pm|a\.m\.|p\.m\.)\b/))) {
    hour = +match[1] % 12 + (match[3].startsWith("p") ? 12 : 0);
    minute = match[2] ? +match[2] : 0;
    remaining = remaining.replace(match[0], " ");
  } else if ((match = remaining.match(/\b(\d{1,2})[:.](\d{2})(?: ?uhr| ?h)?\b/))) {
    hour = +match[1];
    minute = +match[2];
    remaining = remaining.replace(match[0], " ");
  } else if ((match = remaining.match(/\b(\d{1,2}) ?(uhr|h|o'clock)\b/))) {
    hour = +match[1];
    remaining = remaining.replace(match[0], " ");
  } else if ((match = remaining.match(/^(\d{1,2})$/))) {
    // A bare number is an hour only beside a day or a part of the day, and
    // is read on the clock face nearest "evening" if the part says so.
    if (!day && hour === null) return null;
    const bare = +match[1];
    hour = hour !== null && hour >= 12 && bare < 12 ? bare + 12 : bare;
    remaining = "";
  }

  // "evening 8" / "8 tonight": an hour under 12 in the second half of the day.
  if (hour !== null && hour < 12 && /\b(evening|tonight|night|abend|abends|nacht)\b/.test(text)) hour += 12;

  // Anything left over means it was not understood.
  if (remaining.trim()) return null;
  if (hour !== null && (hour > 23 || minute > 59)) return null;
  if (day === null && hour === null) return null;

  if (day === null) {
    // A time with no day: today if still to come, else tomorrow.
    day = midnight(now);
    const today = new Date(day.getFullYear(), day.getMonth(), day.getDate(), hour, minute);
    if (today <= now) day = addDays(day, 1);
  }
  if (hour === null) hour = 8; // "tomorrow" alone means the start of the working day

  return new Date(day.getFullYear(), day.getMonth(), day.getDate(), hour, minute);
}

// -- choosing ---------------------------------------------------------------

/// A few answers that cover most of what "later" means.
function presets(now = new Date()) {
  const at = (days, hour, minute = 0) =>
    new Date(now.getFullYear(), now.getMonth(), now.getDate() + days, hour, minute);
  const list = [];
  if (now.getHours() < 16) list.push(["This evening", at(0, 18)]);
  else list.push(["In an hour", new Date(now.getTime() + 3600e3)]);
  list.push(["Tomorrow morning", at(1, 8)]);
  list.push(["Tomorrow evening", at(1, 20)]);
  const toMonday = ((1 - now.getDay() + 7) % 7) || 7;
  list.push(["Monday morning", at(toMonday, 8)]);
  return list;
}

function toLocalInput(date) {
  const pad = (n) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

function offer(date, { fromText = false } = {}) {
  scheduleWhen = date;
  if (!date) {
    scheduleRead.textContent = scheduleText.value.trim() ? "Not sure when that is — try “tomorrow 8pm”." : "";
    scheduleRead.className = "hint";
    scheduleConfirm.disabled = true;
    return;
  }
  if (date <= new Date()) {
    scheduleRead.textContent = `${whenFormat.format(date)} has passed.`;
    scheduleRead.className = "hint warn";
    scheduleConfirm.disabled = true;
    return;
  }
  scheduleRead.textContent = `Sends ${whenFormat.format(date)}`;
  scheduleRead.className = "hint";
  scheduleConfirm.disabled = false;
  if (fromText) scheduleAt.value = toLocalInput(date);
}

function openScheduleMenu() {
  if (!scheduleMenu.hidden) {
    scheduleMenu.hidden = true;
    return;
  }
  const list = el("schedule-presets");
  list.textContent = "";
  for (const [label, date] of presets()) {
    const button = document.createElement("button");
    button.type = "button";
    const name = document.createElement("span");
    name.textContent = label;
    const when = document.createElement("span");
    when.className = "when";
    when.textContent = shortWhen.format(date);
    button.append(name, when);
    button.onclick = () => scheduleDraft(date);
    list.append(button);
  }
  scheduleText.value = "";
  scheduleAt.value = "";
  offer(null);
  scheduleMenu.hidden = false;
  scheduleText.focus();
}

/// Set while a schedule is on its way to the core, so a second click on a
/// preset cannot put the same message in the outbox twice.
let scheduling = false;

async function scheduleDraft(date) {
  if (scheduling || !date || date <= new Date()) return;
  scheduling = true;
  try {
    await invoke("schedule_send", {
      email: state.email,
      draft: draftInput(),
      sendAt: Math.floor(date.getTime() / 1000),
    });
    scheduleMenu.hidden = true;
    say(`scheduled for ${whenFormat.format(date)}`);
    closeCompose();
    await renderOutboxNav();
  } catch (err) {
    say(String(err), true);
  } finally {
    scheduling = false;
  }
}

el("compose-later").onclick = openScheduleMenu;
el("schedule-cancel").onclick = () => (scheduleMenu.hidden = true);
scheduleConfirm.onclick = () => scheduleDraft(scheduleWhen);
scheduleText.addEventListener("input", () => offer(parseWhen(scheduleText.value), { fromText: true }));
scheduleText.addEventListener("keydown", (event) => {
  if (event.key === "Enter") {
    // Enter here means "schedule it", never "send it now".
    event.preventDefault();
    event.stopPropagation();
    if (!scheduleConfirm.disabled) scheduleDraft(scheduleWhen);
  }
});
scheduleAt.addEventListener("input", () => {
  scheduleText.value = "";
  offer(scheduleAt.value ? new Date(scheduleAt.value) : null);
});

// -- the outbox ---------------------------------------------------------------

let outboxEntries = [];

/// "Scheduled" in the sidebar, only while something is waiting.
async function renderOutboxNav() {
  const nav = el("outbox-nav");
  try {
    outboxEntries = await invoke("outbox");
  } catch {
    outboxEntries = [];
  }
  nav.textContent = "";
  if (!outboxEntries.length) return;
  const failed = outboxEntries.filter((entry) => entry.state === "failed").length;
  nav.append(
    navItem({
      label: failed ? `Scheduled — ${failed} not sent` : "Scheduled",
      icon: "sent",
      count: outboxEntries.length,
      title: "Mail waiting to be sent later",
      onClick: openOutbox,
    }),
  );
}

function relativeWhen(seconds) {
  const when = new Date(seconds * 1000);
  const minutes = Math.round((when - new Date()) / 60000);
  if (minutes < 0) return `was due ${shortWhen.format(when)}`;
  if (minutes < 60) return `in ${minutes} min`;
  return shortWhen.format(when);
}

async function openOutbox() {
  await renderOutboxNav();
  const list = el("outbox-list");
  list.dataset.empty = "Nothing is waiting to be sent.";
  list.textContent = "";
  for (const entry of outboxEntries) list.append(outboxItem(entry));
  outboxSheet.hidden = false;
}

function outboxItem(entry) {
  const item = document.createElement("div");
  item.className = "item";

  const text = document.createElement("div");
  text.className = "grow";
  const title = document.createElement("div");
  title.className = "title";
  title.textContent = entry.subject || "(no subject)";
  const sub = document.createElement("div");
  sub.className = entry.state === "failed" ? "sub bad" : "sub";
  sub.textContent =
    entry.state === "failed"
      ? `Not sent: ${entry.last_error ?? "unknown reason"}. It may have gone anyway — check Sent before sending it again.`
      : `To ${entry.recipients} · from ${entry.account_email}`;
  text.append(title, sub);

  const when = document.createElement("span");
  when.className = entry.state === "failed" ? "chip bad" : entry.state === "sending" ? "chip accent" : "chip";
  when.textContent = entry.state === "sending" ? "sending…" : relativeWhen(entry.send_at);

  const picker = document.createElement("input");
  picker.type = "datetime-local";
  picker.hidden = true;
  picker.value = toLocalInput(new Date(Math.max(entry.send_at * 1000, Date.now() + 3600e3)));

  const buttons = [];
  const button = (label, action, className = "") => {
    const b = document.createElement("button");
    b.type = "button";
    b.textContent = label;
    if (className) b.className = className;
    b.onclick = action;
    buttons.push(b);
    return b;
  };

  if (entry.state !== "sending") {
    button("Send now", async () => {
      try {
        await invoke("reschedule", { id: entry.id, sendAt: Math.floor(Date.now() / 1000) });
        const sent = await invoke("send_due");
        reportSent(sent);
      } catch (err) {
        say(String(err), true);
      }
      await openOutbox();
    });
    button("Edit", () => editScheduled(entry));
    const move = button(entry.state === "failed" ? "Retry at…" : "Move…", async () => {
      if (picker.hidden) {
        picker.hidden = false;
        move.textContent = "Save";
        picker.focus();
        return;
      }
      const date = new Date(picker.value);
      if (!(date > new Date())) {
        say("choose a time still to come", true);
        return;
      }
      try {
        await invoke("reschedule", { id: entry.id, sendAt: Math.floor(date.getTime() / 1000) });
        say(`moved to ${whenFormat.format(date)}`);
      } catch (err) {
        say(String(err), true);
      }
      await openOutbox();
    });
    button("Cancel", async () => {
      if (!confirm(`Cancel “${entry.subject || "(no subject)"}”? It will not be sent.`)) return;
      try {
        await invoke("cancel_scheduled", { id: entry.id });
        say("cancelled");
      } catch (err) {
        say(String(err), true);
      }
      await openOutbox();
    }, "danger");
  }

  item.append(text, when, picker, ...buttons);
  return item;
}

/// Back into compose, and out of the outbox — sending it is then an ordinary
/// send or a new schedule. From the account it was going to be sent from.
async function editScheduled(entry) {
  try {
    const [email, draft] = await invoke("scheduled_draft", { id: entry.id });
    await invoke("cancel_scheduled", { id: entry.id });
    closeDialog(outboxSheet);
    const account = state.accounts.find((a) => a.email === email);
    if (account && account.id !== state.account) await selectAccount(account);
    await openComposeWith(draft);
    await renderOutboxNav();
    say("taken out of the outbox — send it, or schedule it again");
  } catch (err) {
    say(String(err), true);
  }
}

function reportSent(results) {
  for (const result of results) {
    if (result.error) say(`“${result.subject}” was not sent: ${result.error}`, true);
    else say(`sent “${result.subject}”`);
  }
}

el("outbox-close").onclick = () => closeDialog(outboxSheet);

// The scheduler runs in the app, not here; it says when something went.
window.__TAURI__?.event
  ?.listen("outbox-sent", async (event) => {
    reportSent(event.payload ?? []);
    await renderOutboxNav();
    if (!outboxSheet.hidden) await openOutbox();
  })
  .catch(() => {});
