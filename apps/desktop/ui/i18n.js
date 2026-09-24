// What the window says, in the language it is read in.
//
// Loaded before everything else, because every other file calls `t`.
//
// The key is the English text itself. A catalogue of invented keys would mean
// reading two files to know what a line says, and a missing key would show as
// `sidebar.folders.title` to whoever hit it; a missing translation here shows
// the English, which is a sentence. The cost is that two English strings that
// happen to match must mean the same thing — which, in one window, they do.
//
// Static text in index.html is translated by walking the document rather than
// by marking every element up: the markup would be three hundred attributes to
// keep in step with three hundred strings. Anything the window builds as it
// goes calls `t` where it builds it.

/// Deutsch. What is not in here is shown in English, so a half-finished
/// translation is a window in two languages rather than a window of keys.
///
/// Terms kept steady, because the window is read as one thing: mail is
/// "E-Mail", paper post is "Post", a mailbox is "Postfach", a folder on the
/// server is "Ordner", filing is "einsortieren", a smart mailbox is a
/// "Suchordner", and the assistant is "Assistent".
const GERMAN = {
  // -- the header ----------------------------------------------------------
  "New message": "Neue Nachricht",
  "New message (c)": "Neue Nachricht (c)",
  "Search  /": "Suchen  /",
  Cleanup: "Aufräumen",
  "Clear out newsletters, marketing and notifications, and unsubscribe":
    "Newsletter, Werbung und Benachrichtigungen ausräumen und abbestellen",
  Assistant: "Assistent",
  "Assistant (i)": "Assistent (i)",
  Settings: "Einstellungen",
  "Settings (,)": "Einstellungen (,)",
  "starting…": "startet…",
  Download: "Herunterladen",
  "What's new": "Was ist neu",
  Later: "Später",

  // -- the sidebar ---------------------------------------------------------
  Post: "Post",
  Mailboxes: "Ordner",
  "New mailbox": "Neuer Ordner",
  "Smart mailboxes": "Suchordner",
  "New smart mailbox": "Neuer Suchordner",
  Categories: "Kategorien",
  "All mail": "Alle E-Mails",
  "Unread only": "Nur ungelesene",
  Outbox: "Postausgang",
  "Needs attention": "Braucht Aufmerksamkeit",
  People: "Personen",
  Messages: "Nachrichten",
  "Paperless is running": "Paperless läuft",
  "Paperless is not running": "Paperless läuft nicht",
  "Paperless rejected kuverta's token": "Paperless hat kuvertas Token abgelehnt",
  Start: "Starten",
  "Sign in again": "Neu anmelden",
  "Start Docker if need be, then Paperless":
    "Docker starten, falls nötig, und dann Paperless",
  "Never synced — sync now": "Noch nie abgerufen — jetzt abrufen",

  // -- the list ------------------------------------------------------------
  "Select a message": "Nachricht auswählen",
  "Nothing here": "Nichts hier",
  "nothing new": "nichts Neues",
  "no accounts — add one in settings":
    "keine Konten — in den Einstellungen eines anlegen",
  "no accounts yet": "noch keine Konten",
  "setting up": "wird eingerichtet",
  Reply: "Antworten",
  "Reply (R)": "Antworten (R)",
  "Reply all": "Allen antworten",
  "Reply all (A)": "Allen antworten (A)",
  Forward: "Weiterleiten",
  "Forward (f)": "Weiterleiten (f)",
  "Ask the assistant": "Den Assistenten fragen",
  "Ask the assistant about this message (Shift+I)":
    "Den Assistenten zu dieser Nachricht fragen (Umschalt+I)",
  Archive: "Archivieren",
  // A folder called Archive, not the act of archiving: the example in a
  // smart mailbox's rule, where what is meant is a name on the server.
  "Archive|a folder's name": "Archiv",
  "Archive (e)": "Archivieren (e)",
  Delete: "Löschen",
  "Move to Trash (⌫)": "In den Papierkorb (⌫)",
  Send: "Senden",
  Cancel: "Abbrechen",
  Save: "Speichern",
  Done: "Fertig",
  Copy: "Kopieren",
  copied: "kopiert",
  Verify: "Prüfen",
  Remove: "Entfernen",
  Close: "Schließen",
  Undo: "Rückgängig",
  saved: "gespeichert",

  // -- categories, as the list and the sidebar name them -------------------
  personal: "persönlich",
  transactional: "geschäftlich",
  newsletter: "Newsletter",
  marketing: "Werbung",
  notification: "Benachrichtigung",
  unknown: "unbekannt",

  // -- settings ------------------------------------------------------------
  Accounts: "Konten",
  Mail: "E-Mail",
  "+ Add": "+ Neu",
  Account: "Konto",
  "Add account": "Konto hinzufügen",
  "New account": "Neues Konto",
  "New account…": "Neues Konto…",
  "Postal addresses": "Postadressen",
  "Add postal address": "Postadresse hinzufügen",
  Models: "Modelle",
  "Add model provider": "Modellanbieter hinzufügen",
  Profiles: "Profile",
  Keys: "Schlüssel",
  General: "Allgemein",
  Diagnostics: "Diagnose",
  Language: "Sprache",
  "Follow the system": "Wie das System",
  English: "English",
  Deutsch: "Deutsch",
  "The language kuverta is shown in. What is not translated yet stays English.":
    "Die Sprache, in der kuverta angezeigt wird. Was noch nicht übersetzt ist, bleibt englisch.",
  Setup: "Einrichtung",
  "Open the setup assistant": "Einrichtungsassistenten öffnen",
  "Deleting mail": "Nachrichten löschen",
  Updates: "Aktualisierungen",
  "Check for updates": "Nach Aktualisierungen suchen",
  Import: "Übernehmen",
  "Import from other mail programs…": "Aus anderen E-Mail-Programmen übernehmen…",
  "Accounts already set up in Thunderbird or Apple Mail can be taken over with their servers and saved passwords.":
    "Konten, die in Thunderbird oder Apple Mail schon eingerichtet sind, lassen sich mit ihren Servern und gespeicherten Passwörtern übernehmen.",
  Description: "Bezeichnung",
  "Email address": "E-Mail-Adresse",
  Username: "Benutzername",
  Password: "Passwort",
  "same as the email address": "wie die E-Mail-Adresse",
  "Remove account": "Konto entfernen",
  Address: "Adresse",
  "Paperless-ngx address": "Paperless-ngx-Adresse",
  Profile: "Profil",
  "Sign-in": "Anmeldung",
  "API token": "API-Token",
  "leave blank to keep the stored one": "leer lassen, um das gespeicherte zu behalten",
  "A token is stored in the keychain. Sign in again below if Paperless stops accepting it.":
    "Ein Token liegt im Schlüsselbund. Melden Sie sich unten neu an, wenn Paperless es nicht mehr annimmt.",
  "No token stored yet. The button below opens Paperless at its profile page, where the API token is; sign in there, copy it, and paste it above.":
    "Noch kein Token gespeichert. Der Knopf unten öffnet Paperless auf der Profilseite, auf der das API-Token steht; dort anmelden, kopieren und oben einfügen.",
  "Open Paperless to get a token": "Paperless öffnen, um ein Token zu holen",
  "Show Paperless sign-in": "Paperless-Anmeldung zeigen",
  "Hide Paperless sign-in": "Paperless-Anmeldung verbergen",
  "User name": "Benutzername",
  "Remove address": "Adresse entfernen",
  "This signs in to Paperless's own pages — where the API token is. It is kept in the keychain; kuverta reads post with the token, never with this.":
    "Damit melden Sie sich auf den Seiten von Paperless an — dort steht das API-Token. Es liegt im Schlüsselbund; kuverta liest Post mit dem Token, nie hiermit.",
  "kuverta did not install this Paperless, so it has no sign-in for it":
    "kuverta hat dieses Paperless nicht installiert und hat daher keine Anmeldung dafür",
  "fill in the Paperless address first": "zuerst die Paperless-Adresse eintragen",
  "A password is stored in the keychain.": "Ein Passwort liegt im Schlüsselbund.",
  "No password stored yet.": "Noch kein Passwort gespeichert.",

  // -- the setup assistant --------------------------------------------------
  "Set up later": "Später einrichten",
  Continue: "Weiter",
  Back: "Zurück",
  "Open kuverta": "kuverta öffnen",
  Finish: "Fertig",
  "Skip for now": "Vorerst überspringen",
  "Local models": "Lokale Modelle",
  "Paper mail": "Papierpost",
  "Mail accounts": "E-Mail-Konten",
  // The folders on a shelf are Aktenordner, to keep them apart from the
  // Ordner that mail sits in.
  Folders: "Aktenordner",
  "Add a profile": "Profil hinzufügen",
  "Looking for accounts in your other mail programs…":
    "Suche nach Konten in Ihren anderen E-Mail-Programmen…",
  "No other mail programs found.": "Keine anderen E-Mail-Programme gefunden.",
  "not allowed to look": "darf nicht nachsehen",
  "Open System Settings": "Systemeinstellungen öffnen",
  "Look again": "Erneut suchen",
  "Use it": "Übernehmen",
  "Find settings": "Einstellungen suchen",
  "Looking…": "Suche…",
  "Servers and sign-in": "Server und Anmeldung",
  "IMAP server": "IMAP-Server",
  "SMTP server": "SMTP-Server",
  Port: "Port",
  Security: "Sicherheit",
  "Name in the sidebar": "Name in der Seitenleiste",
  "Sign in with": "Anmelden mit",
  "Password or app password": "Passwort oder App-Passwort",
  "already in kuverta": "schon in kuverta",
  "servers unknown — enter them below": "Server unbekannt — unten eintragen",
  "Looking for Paperless…": "Suche nach Paperless…",
  "Folders on the shelf": "Aktenordner im Regal",
  "The paper still has to go somewhere. Name the folders it goes in, and kuverta keeps them in Paperless — where the scanner reads them too, so it can say on its screen which folder a letter belongs in.":
    "Das Papier muss trotzdem irgendwo hin. Benennen Sie die Aktenordner, in die es kommt; kuverta führt sie in Paperless — wo auch der Scanner sie liest und auf seinem Bildschirm sagen kann, in welchen Ordner ein Brief gehört.",
  "Give a folder words that only its letters contain, separated by commas — an insurer, a number plate, “Finanzamt”. A folder without words is learnt instead, from the letters you file in it by hand.":
    "Geben Sie einem Aktenordner Wörter mit, die nur in seinen Briefen vorkommen, durch Kommas getrennt — eine Versicherung, ein Kennzeichen, „Finanzamt“. Ein Ordner ohne Wörter wird stattdessen gelernt, aus den Briefen, die Sie von Hand hineinlegen.",
  "Add a folder": "Aktenordner hinzufügen",
  "Save folders": "Aktenordner speichern",
  "Name of the folder": "Name des Aktenordners",
  "Words, separated by commas (optional)": "Wörter, durch Kommas getrennt (optional)",
  "Remove this folder": "Diesen Aktenordner entfernen",
  "No paper post connected yet — the page before this one.":
    "Noch keine Papierpost verbunden — auf der Seite davor.",
  "Two folders are called {name}.": "Zwei Aktenordner heißen {name}.",
  "Too many words for {name} — leave some out.":
    "Zu viele Wörter für {name} — lassen Sie einige weg.",
  "Setting the folders up…": "Aktenordner werden eingerichtet…",
  "Folders on the shelf: {list}.": "Aktenordner im Regal: {list}.",
  "No folders on the shelf — the scanner cannot say where a letter goes.":
    "Keine Aktenordner im Regal — der Scanner kann nicht sagen, wohin ein Brief gehört.",
  "Who gets post here": "Wer hier Post bekommt",
  "More than one person in a household gets letters — a spouse, a child, someone whose papers you keep. Name them, and the scanner says who a letter is for as well as which folder it goes in. Add another spelling of a name — “E. Mustermann” — if letters come both ways.":
    "In einem Haushalt bekommt oft mehr als eine Person Briefe — Partnerin oder Partner, Kind, jemand, dessen Unterlagen Sie verwahren. Tragen Sie sie ein, dann sagt der Scanner nicht nur, in welchen Ordner ein Brief gehört, sondern auch, für wen er ist. Schreiben Sie eine zweite Schreibweise dazu — „E. Mustermann“ —, wenn Briefe auf beide Arten kommen.",
  "Add a person": "Person hinzufügen",
  "Other spellings of the name (optional)": "Andere Schreibweisen des Namens (optional)",
  "Remove this person": "Diese Person entfernen",
  "Nobody yet — one person's post needs nobody named.":
    "Noch niemand — für die Post einer einzelnen Person muss niemand eingetragen werden.",
  "Post here is for: {list}.": "Post hier ist für: {list}.",
  "{count} folders and {people} people are set up in Paperless. The scanner takes its list from there.":
    "{count} Aktenordner und {people} Personen sind in Paperless eingerichtet. Der Scanner übernimmt seine Liste von dort.",
  "{count} folders are set up in Paperless. The scanner takes its list from there.":
    "{count} Aktenordner sind in Paperless eingerichtet. Der Scanner übernimmt seine Liste von dort.",
  "Install and start Paperless": "Paperless installieren und starten",
  "Paperless user name": "Paperless-Benutzername",
  "leave empty and kuverta makes one": "leer lassen, dann erzeugt kuverta eines",
  "made for you, kept in the keychain": "wird erzeugt und im Schlüsselbund verwahrt",
  "Start Ollama": "Ollama starten",
  "Download all": "Alle herunterladen",
  "Check again": "Erneut prüfen",
  "Use this one": "Dieses verwenden",
  "Start it": "Starten",

  // -- the assistant --------------------------------------------------------
  "Thinking…": "Denkt nach…",
  Tasks: "Aufgaben",
  Chat: "Gespräch",
  Ask: "Fragen",

  // -- the list, as it names what it is showing ----------------------------
  "{count} messages": "{count} Nachrichten",
  "{count} in {where}": "{count} in {where}",
  "one letter to {name}": "ein Brief an {name}",
  "{count} letters to {name}": "{count} Briefe an {name}",
  "{name}: one letter in Paperless": "{name}: ein Brief in Paperless",
  "{name}: {count} letters in Paperless": "{name}: {count} Briefe in Paperless",
  "one needs attention": "eine braucht Aufmerksamkeit",
  "{count} need attention": "{count} brauchen Aufmerksamkeit",
  "one person": "eine Person",
  "{count} people": "{count} Personen",
  "{count} picked": "{count} ausgewählt",
  unpick: "Auswahl aufheben",
  clear: "zurücksetzen",
  "search results": "Suchergebnisse",
  unread: "ungelesen",
  newest: "neueste",
  oldest: "älteste",
  "Newest mail at the top": "Neueste E-Mails oben",
  "Oldest mail at the top": "Älteste E-Mails oben",
  "letter date": "Briefdatum",
  scanned: "eingescannt",
  "Sort and date by the date on the letter": "Nach dem Datum auf dem Brief sortieren und datieren",
  "Sort and date by when it was scanned": "Danach sortieren und datieren, wann er eingescannt wurde",
  "Categories in {where}": "Kategorien in {where}",
  "{folder} — {unread} unread of {total}": "{folder} — {unread} ungelesen von {total}",

  // -- reading -------------------------------------------------------------
  "(no subject)": "(kein Betreff)",
  "(untitled)": "(ohne Titel)",
  "(no answer)": "(keine Antwort)",
  "one page": "eine Seite",
  "{count} pages": "{count} Seiten",
  "Paperless document {id}": "Paperless-Dokument {id}",
  "(no text yet — Paperless may still be reading this scan)":
    "(noch kein Text — Paperless liest diesen Scan vielleicht noch)",
  "Read again": "Erneut lesen",
  "Read with the vision model": "Mit dem Bildmodell lesen",
  "Reading the scan with the vision model… it takes a little while a page.":
    "Der Scan wird mit dem Bildmodell gelesen… das dauert pro Seite einen Moment.",
  "Read from the scan by {model}. Where the scan is unclear it can get a word wrong — the PDF tab has the page itself.":
    "Von {model} aus dem Scan gelesen. Wo der Scan undeutlich ist, kann ein Wort falsch sein — im PDF-Reiter steht die Seite selbst.",
  "Loading the scan from Paperless…": "Der Scan wird aus Paperless geladen…",
  "could not load the scan: {error}": "Der Scan konnte nicht geladen werden: {error}",
  "the vision model could not read the scan: {error}":
    "Das Bildmodell konnte den Scan nicht lesen: {error}",
  "Mark read": "Als gelesen markieren",
  "marked read": "als gelesen markiert",
  "marked unread": "als ungelesen markiert",
  "could not mark it read: {error}": "Ließ sich nicht als gelesen markieren: {error}",
  "could not mark it unread: {error}": "Ließ sich nicht als ungelesen markieren: {error}",
  "could not open: {error}": "Ließ sich nicht öffnen: {error}",
  "Open message": "Nachricht öffnen",
  "Move to Trash": "In den Papierkorb",
  "Move to {folder}": "Nach {folder} verschieben",
  // -- the menu under the right button, over the list ----------------------
  Open: "Öffnen",
  "Mark as read": "Als gelesen markieren",
  "Mark as unread": "Als ungelesen markieren",
  "Move to mailbox…": "In einen Ordner verschieben…",
  "← Back": "← Zurück",
  "they are already here": "Sie liegen schon hier",
  "no mailboxes yet": "noch keine Ordner",
  "New smart mailbox from these {count}…": "Suchordner aus diesen {count}…",
  "File as {category}": "Als {category} einsortieren",
  "this account has no Archive folder": "Dieses Konto hat keinen Archiv-Ordner",
  "this account has no Trash folder": "Dieses Konto hat keinen Papierkorb",
  "nothing to undo": "nichts rückgängig zu machen",
  "undone: {what}": "rückgängig gemacht: {what}",
  "Undone ({count})": "Rückgängig gemacht ({count})",
  "select a message first": "zuerst eine Nachricht auswählen",

  // -- writing -------------------------------------------------------------
  "Scheduled message": "Geplante Nachricht",
  "New message to {to}": "Neue Nachricht an {to}",
  "Reply to {to}": "Antwort an {to}",
  "Sending…": "Wird gesendet…",
  "sent to {recipients}": "gesendet an {recipients}",
  "sent to {count} — but not filed: {error}":
    "an {count} gesendet — aber nicht abgelegt: {error}",
  "sent — but not filed: {error}": "gesendet — aber nicht abgelegt: {error}",
  "not sent": "nicht gesendet",
  "post cannot be answered from here": "Post lässt sich von hier aus nicht beantworten",
  "Open in compose": "Im Schreibfenster öffnen",
  "Edit in compose": "Im Schreibfenster bearbeiten",
  "Send this reply": "Diese Antwort senden",
  "Send this reply?": "Diese Antwort senden?",
  "Send this reply to {sender}?": "Diese Antwort an {sender} senden?",
  "Send this to {to}?": "Das an {to} senden?",
  "Already sent — undo it in the mailbox": "Schon gesendet — im Postfach rückgängig machen",
  "Queued — it reaches the server at the next sync.":
    "Eingereiht — beim nächsten Abruf geht es an den Server.",

  // -- syncing -------------------------------------------------------------
  "{account} — syncing…": "{account} — wird abgerufen…",
  "{account} — syncing {percent}%: {folder}{within}":
    "{account} — wird abgerufen, {percent} %: {folder}{within}",
  "{account}: {count} new": "{account}: {count} neu",
  "{account}: {error}": "{account}: {error}",
  "Sync {account}": "{account} abrufen",
  "Last synced {when} — sync now": "Zuletzt abgerufen {when} — jetzt abrufen",
  "one change waiting for the next sync": "eine Änderung wartet auf den nächsten Abruf",
  "{count} changes waiting for the next sync":
    "{count} Änderungen warten auf den nächsten Abruf",
  "failed to start: {error}": "Start fehlgeschlagen: {error}",

  // -- post ----------------------------------------------------------------
  "one new letter to {name}": "ein neuer Brief an {name}",
  "{count} new letters to {name}": "{count} neue Briefe an {name}",
  "{name} has no Paperless token stored — add it in settings (,)":
    "Für {name} ist kein Paperless-Token gespeichert — in den Einstellungen (,) eintragen",
  "{name} is post — that is done in Paperless": "{name} ist Post — das macht Paperless",
  "People lists people, not messages — open one, or switch to Messages":
    "Personen listet Personen, keine Nachrichten — eine öffnen oder zu Nachrichten wechseln",
  "could not start Paperless: {error}": "Paperless ließ sich nicht starten: {error}",
  "Sign in again for {where}": "Für {where} neu anmelden",
  "Nothing answers at {where}": "Unter {where} antwortet nichts",
  " — it runs elsewhere, so start it there":
    " — es läuft woanders, also dort starten",
  "{bulk} newsletter, marketing and notification messages in the Inbox · {senders} senders you can unsubscribe from":
    "{bulk} Newsletter, Werbung und Benachrichtigungen im Posteingang · {senders} Absender zum Abbestellen",

  // -- settings ------------------------------------------------------------
  "New address…": "Neue Adresse…",
  "New provider…": "Neuer Anbieter…",
  Encryption: "Verschlüsselung",
  "Model for each job": "Modell für jede Aufgabe",
  "Remove {account} and everything synced for it?":
    "{account} und alles dafür Abgerufene entfernen?",
  "Remove {name}? Its documents stay in Paperless; only the address and its stored token are forgotten here.":
    "{name} entfernen? Die Dokumente bleiben in Paperless; hier werden nur die Adresse und das gespeicherte Token vergessen.",
  "removed {what}": "{what} entfernt",
  "enter an email address first": "zuerst eine E-Mail-Adresse eintragen",
  "Verifying…": "Wird geprüft…",
  "Connecting…": "Verbindung wird aufgebaut…",
  "Trying…": "Wird ausprobiert…",
  "Try them": "Ausprobieren",
  "Asking…": "Wird gefragt…",
  "Asking for its models…": "Die Modelle werden abgefragt…",
  "Could not list its models: {error}": "Die Modelle ließen sich nicht auflisten: {error}",
  "Edit “{name}”": "„{name}“ bearbeiten",
  Edit: "Bearbeiten",
  "Delete it": "Löschen",
  Deleted: "Gelöscht",
  Sent: "Gesendet",
  Off: "Aus",
  "running…": "läuft…",

  // -- the assistant --------------------------------------------------------
  "the assistant works on mail accounts": "Der Assistent arbeitet mit E-Mail-Konten",
  "Choose a mail account first.": "Wählen Sie zuerst ein E-Mail-Konto.",
  "choose a mail account first": "zuerst ein E-Mail-Konto wählen",
  "Ask about the mail on {email}, or say what to do with it. Everything it changes can be undone, and it never sends mail itself.":
    "Fragen Sie nach der Post auf {email} oder sagen Sie, was damit geschehen soll. Alles, was der Assistent ändert, lässt sich rückgängig machen, und er sendet nie selbst.",
  "About:": "Es geht um:",
  "About {count} messages:": "Es geht um {count} Nachrichten:",
  "about {messages}": "zu {messages}",
  "Leave this one out": "Diese weglassen",
  "Leave out {subject}": "{subject} weglassen",
  "Reply and confirm.": "Antworten und bestätigen.",
  "What does this need from me?": "Was erwartet das von mir?",
  "Summarise this for me.": "Fasse mir das zusammen.",
  "Write a summary of this I can send to someone.":
    "Schreibe eine Zusammenfassung, die ich jemandem schicken kann.",
  "What needs my attention today?": "Was braucht heute meine Aufmerksamkeit?",
  "Summarise my unread mail from people.":
    "Fasse meine ungelesene Post von Menschen zusammen.",
  "Find my invoices from this month.": "Finde meine Rechnungen aus diesem Monat.",
  "From now on, move invoices into a folder called Rechnungen.":
    "Verschiebe Rechnungen ab jetzt in einen Ordner namens Rechnungen.",
  "Draft replies to the support requests from this week — I'll check them.":
    "Entwirf Antworten auf die Support-Anfragen dieser Woche — ich sehe sie durch.",
  "{model}, on this computer. A small model may miss things; a larger one (Settings → Models → Assistant) does better.":
    "{model}, auf diesem Rechner. Ein kleines Modell übersieht schon einmal etwas; ein größeres (Einstellungen → Modelle → Assistent) macht es besser.",
  "{model}, hosted: what the assistant reads is sent to that service.":
    "{model}, gehostet: Was der Assistent liest, geht an diesen Dienst.",

  // -- tasks ----------------------------------------------------------------
  Tasks: "Aufgaben",
  "New task": "Neue Aufgabe",
  "New task: {name}": "Neue Aufgabe: {name}",
  "there are no tasks yet": "es gibt noch keine Aufgaben",
  "Check it in Tasks": "Unter Aufgaben nachsehen",
  "For mail where {rules}: {what}. One message matches now.":
    "Für Post, auf die {rules} zutrifft: {what}. Eine Nachricht passt gerade.",
  "For mail where {rules}: {what}. {count} messages match now.":
    "Für Post, auf die {rules} zutrifft: {what}. {count} Nachrichten passen gerade.",
  "One message matches these rules now.": "Eine Nachricht passt gerade auf diese Regeln.",
  "{count} messages match these rules now.":
    "{count} Nachrichten passen gerade auf diese Regeln.",
  "When {rules}: {what}": "Wenn {rules}: {what}",
  "When {rules}: {what} — asks first": "Wenn {rules}: {what} — fragt vorher",
  "On — runs after every sync": "An — läuft nach jedem Abruf",
  "Last run: {summary}": "Zuletzt gelaufen: {summary}",
  "{count} waiting": "{count} warten",
  "{count} waiting for your approval": "{count} warten auf Ihre Freigabe",
  "{done} done, {waiting} waiting for you": "{done} erledigt, {waiting} warten auf Sie",
  "tasks: {done} done, {waiting} waiting for you":
    "Aufgaben: {done} erledigt, {waiting} warten auf Sie",
  "task saved — it runs after every sync": "Aufgabe gespeichert — sie läuft nach jedem Abruf",
  "Delete this task? What it did stays done.":
    "Diese Aufgabe löschen? Was sie getan hat, bleibt getan.",
  "Replies always wait for you: nothing a model writes is sent without you reading it.":
    "Antworten warten immer auf Sie: Nichts, was ein Modell schreibt, geht ungelesen hinaus.",
  "The model reads each message and picks move, archive, trash, mark read, file or a reply — or leaves it.":
    "Das Modell liest jede Nachricht und wählt verschieben, archivieren, löschen, als gelesen markieren, einsortieren oder eine Antwort — oder lässt sie liegen.",
  Approve: "Freigeben",
  Reject: "Ablehnen",
  "approved {count} — each can be undone from the list with z":
    "{count} freigegeben — jede lässt sich in der Liste mit z rückgängig machen",
  "asking the model: {done} of {total}": "das Modell wird gefragt: {done} von {total}",
  "from {sender}": "von {sender}",

  // -- the outbox -----------------------------------------------------------
  Scheduled: "Geplant",
  "Scheduled — {count} not sent": "Geplant — {count} nicht gesendet",
  "Mail waiting to be sent later": "Post, die später gesendet werden soll",
  "Nothing is waiting to be sent.": "Es wartet nichts auf den Versand.",
  "To {recipients} · from {account}": "An {recipients} · von {account}",
  "Sends {when}": "Geht {when} hinaus",
  "scheduled for {when}": "geplant für {when}",
  "was due {when}": "war fällig {when}",
  "in {minutes} min": "in {minutes} Min.",
  "sending…": "wird gesendet…",
  "Send now": "Jetzt senden",
  "Retry at…": "Erneut versuchen um…",
  "Move…": "Verschieben…",
  "choose a time still to come": "einen Zeitpunkt wählen, der noch kommt",
  "moved to {when}": "verschoben auf {when}",
  "Cancel “{subject}”? It will not be sent.": "„{subject}“ abbrechen? Es wird dann nicht gesendet.",
  cancelled: "abgebrochen",
  "taken out of the outbox — send it, or schedule it again":
    "aus dem Postausgang genommen — senden Sie es oder planen Sie es neu",
  "“{subject}” was not sent: {error}": "„{subject}“ wurde nicht gesendet: {error}",
  "sent “{subject}”": "„{subject}“ gesendet",
  "Not sent: {error}. It may have gone anyway — check Sent before sending it again.":
    "Nicht gesendet: {error}. Vielleicht ist es trotzdem hinausgegangen — sehen Sie in Gesendet nach, bevor Sie es erneut senden.",
  "unknown reason": "unbekannter Grund",
  "This evening": "Heute Abend",
  "In an hour": "In einer Stunde",
  "Tomorrow morning": "Morgen früh",
  "Tomorrow evening": "Morgen Abend",
  "Monday morning": "Montagmorgen",
  "Not sure when that is — try “tomorrow 8pm”.":
    "Unklar, wann das ist — versuchen Sie „morgen 20 Uhr“.",
  "{when} has passed.": "{when} ist vorbei.",

  // -- the setup assistant, page by page ------------------------------------
  "Download {what}": "{what} herunterladen",
  "Download ({size})": "Herunterladen ({size})",
  "Or in a terminal:": "Oder im Terminal:",
  "{size} GB": "{size} GB",
  "{size} MB": "{size} MB",
  "You can continue and come back to this later.":
    "Sie können weitergehen und später hierher zurückkommen.",
  "reading scanned letters": "eingescannte Briefe lesen",
  "sorting mail": "Post einsortieren",
  "Ollama at {url} answers (version {version})":
    "Ollama unter {url} antwortet (Version {version})",
  "Ollama at {url} is not answering. It is on another computer; start it there.":
    "Ollama unter {url} antwortet nicht. Es läuft auf einem anderen Rechner; starten Sie es dort.",
  "Ollama is not installed on this computer.": "Ollama ist auf diesem Rechner nicht installiert.",
  "Ollama is installed.": "Ollama ist installiert.",
  "It is not running.": "Es läuft nicht.",
  "Ollama is running (version {version}).": "Ollama läuft (Version {version}).",
  "Starting…": "Wird gestartet…",
  "starting…": "startet…",
  "{done} of {total}": "{done} von {total}",
  "{model} is ready": "{model} ist bereit",
  "{model} for {purpose} is ready.": "{model} für {purpose} ist bereit.",
  "{model} for {purpose} is not downloaded yet.":
    "{model} für {purpose} ist noch nicht heruntergeladen.",
  "Downloading {model} for {purpose}": "{model} für {purpose} wird heruntergeladen",
  "{url} is connected.": "{url} ist verbunden.",
  "{url} is connected but not answering.": "{url} ist verbunden, antwortet aber nicht.",
  "Paperless answers at {url}.": "Paperless antwortet unter {url}.",
  "The Paperless kuverta installed ({url}) is not running.":
    "Das von kuverta installierte Paperless ({url}) läuft nicht.",
  "No Paperless found on this computer or at paperless.local.":
    "Kein Paperless auf diesem Rechner oder unter paperless.local gefunden.",
  "Connect to one elsewhere on your network, or install one here with Docker.":
    "Verbinden Sie sich mit einem anderen im Netzwerk oder installieren Sie hier eines mit Docker.",
  "Connected: {matching} of {total} documents belong here.":
    "Verbunden: {matching} von {total} Dokumenten gehören hierher.",
  "Looking for Docker…": "Suche nach Docker…",
  "Docker is not installed. Paperless runs in it.":
    "Docker ist nicht installiert. Paperless läuft darin.",
  "Docker is installed.": "Docker ist installiert.",
  "It is not running: {problem}": "Es läuft nicht: {problem}",
  "Start Docker": "Docker starten",
  "Docker is starting; check again in a moment":
    "Docker startet; sehen Sie gleich noch einmal nach",
  "Docker is running (version {version}).": "Docker läuft (Version {version}).",
  "Installing — this takes a few minutes the first time.":
    "Wird installiert — beim ersten Mal dauert das ein paar Minuten.",
  "Paperless is running and connected. Its web page is where you upload and manage documents; sign in as {user}.":
    "Paperless läuft und ist verbunden. Auf seiner Webseite laden Sie Dokumente hoch und verwalten sie; melden Sie sich als {user} an.",
  "Paperless is running and connected. Its web page is where you upload and manage documents; sign in as {user}. kuverta made the password and put it in the keychain: Settings → the address → Show Paperless sign-in.":
    "Paperless läuft und ist verbunden. Auf seiner Webseite laden Sie Dokumente hoch und verwalten sie; melden Sie sich als {user} an. kuverta hat das Passwort erzeugt und in den Schlüsselbund gelegt: Einstellungen → die Adresse → Paperless-Anmeldung zeigen.",
  "Thunderbird's primary password": "Thunderbirds Hauptpasswort",
  "Thunderbird keeps its saved passwords behind a primary password. Enter it and kuverta can take them over; otherwise enter each password below.":
    "Thunderbird hält seine gespeicherten Passwörter hinter einem Hauptpasswort. Geben Sie es ein, dann kann kuverta sie übernehmen; sonst tragen Sie jedes Passwort unten ein.",
  "{imap}, sends via {host}:{port}": "{imap}, sendet über {host}:{port}",
  "{imap}, cannot send (no outgoing server)": "{imap}, kann nicht senden (kein Postausgangsserver)",
  "from {source}": "aus {source}",
  "Use the password saved in {source}": "Das in {source} gespeicherte Passwort verwenden",
  "Password (only if you switch to a password below)":
    "Passwort (nur wenn Sie unten auf Passwort umstellen)",
  Open: "Öffnen",
  "same as the address": "wie die Adresse",
  "OAuth2 (sign in with kuverta login)": "OAuth2 (Anmeldung über kuverta login)",
  "OAuth2 client id": "OAuth2-Client-ID",
  "Add one account": "Ein Konto hinzufügen",
  "Add {count} accounts": "{count} Konten hinzufügen",
  "Enter the password first.": "Geben Sie zuerst das Passwort ein.",
  "Checking…": "Wird geprüft…",
  "Added. Sign in once in a terminal: kuverta login --email {email}":
    "Hinzugefügt. Melden Sie sich einmal im Terminal an: kuverta login --email {email}",
  "Could not sign in: {error}": "Anmeldung fehlgeschlagen: {error}",
  "Sending works.": "Senden funktioniert.",
  "Sending did not work: {error}": "Senden hat nicht funktioniert: {error}",
  "Signed in — {count} messages.": "Angemeldet — {count} Nachrichten.",
  "Saved, but: {error}": "Gespeichert, aber: {error}",
  "Fix the accounts marked in red, untick them, or continue without them.":
    "Bringen Sie die rot markierten Konten in Ordnung, haken Sie sie ab oder fahren Sie ohne sie fort.",
  "Name, e.g. Private": "Name, z. B. Privat",
  "Models are ready on this computer.": "Die Modelle sind auf diesem Rechner bereit.",
  "Models are not ready — scanned letters cannot be read yet. Settings → Setup assistant.":
    "Die Modelle sind nicht bereit — eingescannte Briefe lassen sich noch nicht lesen. Einstellungen → Einrichtungsassistent.",
  "Paper mail: {list}.": "Papierpost: {list}.",
  "No paper mail connected.": "Keine Papierpost verbunden.",
  "Profiles: {list}.": "Profile: {list}.",
  "one mail account.": "ein E-Mail-Konto.",
  "{count} mail accounts.": "{count} E-Mail-Konten.",
  "No mail accounts yet — add them in settings.":
    "Noch keine E-Mail-Konten — legen Sie sie in den Einstellungen an.",
  "Downloading mail for one account — you can start using kuverta meanwhile.":
    "Die Post eines Kontos wird heruntergeladen — Sie können kuverta währenddessen schon benutzen.",
  "Downloading mail for {count} accounts — you can start using kuverta meanwhile.":
    "Die Post von {count} Konten wird heruntergeladen — Sie können kuverta währenddessen schon benutzen.",
  "Downloading mail for {email} — starting…":
    "Post für {email} wird heruntergeladen — Start…",
  "Downloading mail for {email} ({nth} of {total}) — starting…":
    "Post für {email} wird heruntergeladen ({nth} von {total}) — Start…",
  "Downloading mail for {email} — {percent}%: {folder} ":
    "Post für {email} wird heruntergeladen — {percent} %: {folder} ",
  "Downloading mail for {email} ({nth} of {total}) — {percent}%: {folder} ":
    "Post für {email} wird heruntergeladen ({nth} von {total}) — {percent} %: {folder} ",
  "Downloading mail for {email} — {percent}%: {folder}, {done} of {messages} ":
    "Post für {email} wird heruntergeladen — {percent} %: {folder}, {done} von {messages} ",
  "Downloading mail for {email} ({nth} of {total}) — {percent}%: {folder}, {done} of {messages} ":
    "Post für {email} wird heruntergeladen ({nth} von {total}) — {percent} %: {folder}, {done} von {messages} ",
  "{email}: {count} messages downloaded": "{email}: {count} Nachrichten heruntergeladen",
  "Mail for one account is downloaded.": "Die Post eines Kontos ist heruntergeladen.",
  "Mail for {count} accounts is downloaded.":
    "Die Post von {count} Konten ist heruntergeladen.",

  // -- smart mailboxes, people, profiles, folders ---------------------------
  All: "Alle",
  "Every account and address": "Alle Konten und Adressen",
  "No profile": "Kein Profil",
  "No profiles yet. Add one below.": "Noch keine Profile. Legen Sie unten eines an.",
  "Add profile": "Profil hinzufügen",
  "Keep private mail and each company's mail apart. The switcher above the sidebar shows one profile at a time, or all of them. An account or address in no profile shows under All only.":
    "Private Post und die Post jeder Firma getrennt halten. Der Umschalter über der Seitenleiste zeigt ein Profil nach dem anderen oder alle. Ein Konto oder eine Adresse ohne Profil erscheint nur unter Alle.",
  "Delete the profile “{name}”? Its accounts and addresses stay, in no profile.":
    "Profil „{name}“ löschen? Seine Konten und Adressen bleiben, dann ohne Profil.",
  "e.g. Private, Company 1": "z. B. Privat, Firma 1",
  "nothing is in this profile yet — add accounts to it in Settings → Profiles":
    "in diesem Profil ist noch nichts — fügen Sie ihm unter Einstellungen → Profile Konten hinzu",
  "one account and address": "ein Konto und eine Adresse",
  "{count} accounts and addresses": "{count} Konten und Adressen",
  "mail account": "E-Mail-Konto",
  "postal address · {url}": "Postadresse · {url}",
  "on {email}": "auf {email}",
  "On {email}": "Auf {email}",
  "goes to {email}": "geht an {email}",
  Name: "Name",
  "What is in each": "Was in jedem steckt",
  "Add a rule to see what it gathers.": "Fügen Sie eine Regel hinzu, um zu sehen, was sie einsammelt.",
  "Remove this rule": "Diese Regel entfernen",
  "Nothing matches these rules yet.": "Auf diese Regeln passt noch nichts.",
  "one message matches": "eine Nachricht passt",
  "{count} messages match": "{count} Nachrichten passen",
  "{field} {op} {value}": "{field} {op} {value}",
  "{rules} — {unread} unread of {total}": "{rules} — {unread} ungelesen von {total}",
  "None yet. Make one here, from the + beside Smart mailboxes in the sidebar, or after deleting a message that has look-alikes.":
    "Noch keine. Legen Sie einen hier an, über das + neben Suchordner in der Seitenleiste, oder nachdem Sie eine Nachricht mit Doppelgängern gelöscht haben.",
  "Delete the smart mailbox “{name}”? No mail is deleted — it only stops gathering it.":
    "Suchordner „{name}“ löschen? Es wird keine Post gelöscht — er sammelt sie nur nicht mehr ein.",
  "Neither Thunderbird nor Apple Mail has smart mailboxes on this computer.":
    "Weder Thunderbird noch Apple Mail hat auf diesem Rechner Suchordner.",
  "tick the ones to bring over": "haken Sie an, was übernommen werden soll",
  "none of its rules can be carried over": "keine seiner Regeln lässt sich übernehmen",
  "imported one smart mailbox": "ein Suchordner übernommen",
  "imported {count} smart mailboxes": "{count} Suchordner übernommen",
  "New mailbox inside “{name}”…": "Neuer Ordner in „{name}“…",
  "Nowhere — at the top": "Nirgends — ganz oben",
  "Delete the mailbox “{name}” from the server?": "Ordner „{name}“ auf dem Server löschen?",
  "{name} still has one message — move them out first":
    "In {name} liegt noch eine Nachricht — verschieben Sie sie zuerst heraus",
  "{name} still has {count} messages — move them out first":
    "In {name} liegen noch {count} Nachrichten — verschieben Sie sie zuerst heraus",
  "The account keeps its mail of this kind here": "Das Konto legt Post dieser Art hier ab",
  "Edit…": "Bearbeiten…",
  "Delete “{name}”": "„{name}“ löschen",
  "Delete “{name}”…": "„{name}“ löschen…",
  "Rename by typing; it is saved when you leave the field":
    "Zum Umbenennen tippen; gespeichert wird, sobald Sie das Feld verlassen",
  "Move up": "Nach oben",
  "Move down": "Nach unten",
  "created {name}": "{name} angelegt",
  "deleted {name}": "{name} gelöscht",
  deleted: "gelöscht",
  "{count} in it": "{count} darin",
  "{unread} new · {total}": "{unread} neu · {total}",

  // -- needs attention, people ----------------------------------------------
  "Inbox mail ranked by how soon it needs you, with the reason":
    "Post im Posteingang danach geordnet, wie bald sie Sie braucht, mit Begründung",
  "Newsletters, marketing and notifications are left out unless shown":
    "Newsletter, Werbung und Benachrichtigungen bleiben außen vor, sofern nicht eingeblendet",
  "Needs you {when}{action}{due}: {reason} ({who})":
    "Braucht Sie {when}{action}{due}: {reason} ({who})",
  "no reason given": "keine Begründung angegeben",
  "judged by the rules": "von den Regeln beurteilt",
  "judged by {model}": "von {model} beurteilt",
  "the rules judged {count}": "die Regeln haben {count} beurteilt",
  "{model} judged {count}": "{model} hat {count} beurteilt",
  "nothing new to judge": "nichts Neues zu beurteilen",
  "{what}; the model stopped: {why}": "{what}; das Modell hat aufgehört: {why}",
  "{email} — judging urgency {done} of {total}":
    "{email} — Dringlichkeit wird beurteilt, {done} von {total}",
  "Let the model chosen for sorting mail judge the mail the rules have judged so far":
    "Das für das Einsortieren gewählte Modell die bisher von den Regeln beurteilte Post beurteilen lassen",
  "Ask the model": "Das Modell fragen",
  "hide bulk senders": "Massenabsender ausblenden",
  "show bulk senders": "Massenabsender einblenden",
  "No messages to show.": "Keine Nachrichten zu zeigen.",
  "add a mail account first": "zuerst ein E-Mail-Konto anlegen",
  "Double-click for the whole message": "Doppelklick für die ganze Nachricht",
  "You: {text}": "Sie: {text}",
  "could not open the conversation: {error}": "Das Gespräch ließ sich nicht öffnen: {error}",
  "by {day}": "bis {day}",
  invoice: "Rechnung",
  unsubscribe: "abbestellen",
  and: "und",
  or: "oder",
  yes: "ja",
  no: "nein",
  sent: "gesendet",
  "also {names}": "außerdem {names}",

  // -- who the message is with ----------------------------------------------
  "from them": "von ihnen",
  "from you": "von Ihnen",
  "with a file": "mit Anhang",
  "no mail with them yet": "noch keine Post mit ihnen",
  Since: "Seit",
  "They wrote": "Schrieb",
  "You wrote": "Sie schrieben",
  never: "nie",
  Usually: "Meistens",
  "{category} — bulk mail": "{category} — Massenpost",
  "Filed in": "Abgelegt in",
  "Waiting on you {when}: {reason}": "Wartet auf Sie {when}: {reason}",
  "Last correspondence": "Letzter Schriftwechsel",
  "Whole conversation": "Ganzes Gespräch",
  "Everything with this person, as a messenger shows it":
    "Alles mit dieser Person, wie es ein Messenger zeigt",
  "Who this is — how much mail there is with them, and the last of it":
    "Wer das ist — wie viel Post es mit ihnen gibt und die letzte davon",
  "Move this message to the Trash": "Diese Nachricht in den Papierkorb verschieben",
  "moved to {folder} — z undoes it": "nach {folder} verschoben — z macht es rückgängig",
  "that letter is further down the list than has been loaded":
    "Dieser Brief liegt weiter unten in der Liste, als bisher geladen ist",

  // -- look-alikes ----------------------------------------------------------
  "{count} more like “{subject}”": "{count} weitere wie „{subject}“",
  "From {from}. They look alike because of: {reasons}. Untick any to keep.":
    "Von {from}. Sie ähneln sich wegen: {reasons}. Haken Sie ab, was bleiben soll.",
  Keep: "Behalten",
  "Move {count} to Trash": "{count} in den Papierkorb",
  "moved {count} to {folder}, then: {error}":
    "{count} nach {folder} verschoben, dann: {error}",
  "moved {count} more to {folder} — z undoes them one at a time":
    "{count} weitere nach {folder} verschoben — z macht sie einzeln rückgängig",
  "Left out: {what}": "Ausgelassen: {what}",
  "won't ask again — it can be turned back on in Settings → General":
    "wird nicht wieder gefragt — in Einstellungen → Allgemein lässt es sich wieder einschalten",

  // -- the offer to unsubscribe, after deleting a newsletter ----------------
  // The sheet's own markup, before it is filled in.
  "Stop mail from this sender?": "Keine Post mehr von diesem Absender?",
  "Move their other messages to Trash too":
    "Deren übrige Nachrichten auch in den Papierkorb",
  "Only unsubscribe from senders you recognise. Real spam takes an unsubscribe as proof that somebody reads it — delete that instead.":
    "Bestellen Sie nur bei Absendern ab, die Sie kennen. Echter Spam nimmt eine Abbestellung als Beweis, dass jemand mitliest — löschen Sie den lieber.",
  "No, keep them": "Nein, behalten",
  "After deleting a newsletter, offer to unsubscribe from it":
    "Nach dem Löschen eines Newsletters anbieten, ihn abzubestellen",
  "Deleting one is the moment you decided you did not want it. When the message says how to be taken off the list, kuverta offers to do it there and then — one click where the sender supports it, a mail where they do not, and their page opened where nothing else works.":
    "Der Moment, in dem Sie ihn löschen, ist der Moment der Entscheidung. Wenn die Nachricht sagt, wie man von der Liste kommt, bietet kuverta es gleich dort an — ein Klick, wo der Absender das unterstützt, sonst eine Mail, und andernfalls deren Seite im Browser.",

  "Stop mail from {sender}?": "Keine Post mehr von {sender}?",
  "That one was a newsletter. {way}": "Das war ein Newsletter. {way}",
  "kuverta tells them once, and that is all.": "kuverta sagt einmal Bescheid, mehr ist nicht nötig.",
  "kuverta sends them a mail asking to be taken off.":
    "kuverta schickt eine Mail mit der Bitte, Sie auszutragen.",
  "It opens their page in the browser; the last step is yours.":
    "Es öffnet deren Seite im Browser; den letzten Schritt machen Sie.",
  "Move the one other message from them to Trash too":
    "Die eine weitere Nachricht von ihnen auch in den Papierkorb",
  "Move their other {count} messages to Trash too":
    "Die {count} weiteren Nachrichten von ihnen auch in den Papierkorb",
  "Open their page": "Deren Seite öffnen",
  Unsubscribe: "Abbestellen",
  "Unsubscribing from {sender}…": "{sender} wird abbestellt…",
  "Their page is open in the browser.": "Deren Seite ist im Browser offen.",
  "Unsubscribed from {sender}.": "{sender} abbestellt.",
  "{sender} could not be unsubscribed from: {detail}":
    "{sender} konnte nicht abbestellt werden: {detail}",
  "{count} more moved to Trash.": "{count} weitere in den Papierkorb verschoben.",

  // -- attachments ----------------------------------------------------------
  "Choose a file…": "Datei wählen…",
  "{name} — {type}": "{name} — {type}",
  "{name} — could run something when opened":
    "{name} — könnte beim Öffnen etwas ausführen",
  "could run something when opened: save it only if you trust the sender":
    "könnte beim Öffnen etwas ausführen: nur speichern, wenn Sie dem Absender trauen",
  "Not opened from here: it could run something. Save it instead.":
    "Wird von hier nicht geöffnet: Es könnte etwas ausführen. Speichern Sie es stattdessen.",
  "kuverta can't show this kind of file itself. Save it, or open it in the app your computer has for it.":
    "kuverta kann diese Art Datei nicht selbst zeigen. Speichern Sie sie oder öffnen Sie sie im Programm, das Ihr Rechner dafür hat.",
  "saved to {path}": "gespeichert unter {path}",
  "could not save it: {error}": "Ließ sich nicht speichern: {error}",
  "could not open it: {error}": "Ließ sich nicht öffnen: {error}",
  "could not load it: {error}": "Ließ sich nicht laden: {error}",
  "Loading…": "Wird geladen…",
  "(no readable body)": "(kein lesbarer Text)",
  "(nothing but an attachment)": "(nichts als ein Anhang)",
  "one picture in the text": "ein Bild im Text",
  "{count} pictures in the text": "{count} Bilder im Text",
  "{count} bytes": "{count} Bytes",
  "{count} KB": "{count} KB",
  "{count} MB": "{count} MB",

  // -- encryption -----------------------------------------------------------
  "Encryption settings": "Verschlüsselungseinstellungen",
  "Sign mail so its recipients can tell it is really from you, and encrypt it so only they can read it. kuverta uses OpenPGP (PGP/MIME), which Thunderbird, GnuPG, Mailvelope and most other mail programs understand.":
    "Post signieren, damit die Empfänger sehen, dass sie wirklich von Ihnen ist, und verschlüsseln, damit nur sie sie lesen können. kuverta nutzt OpenPGP (PGP/MIME), das Thunderbird, GnuPG, Mailvelope und die meisten anderen Programme verstehen.",
  "Your keys": "Ihre Schlüssel",
  "Other people's keys": "Schlüssel anderer Leute",
  "You have no key yet. Create one below, or import the one you already use elsewhere.":
    "Sie haben noch keinen Schlüssel. Legen Sie unten einen an oder übernehmen Sie den, den Sie anderswo schon nutzen.",
  "None yet. Import them below, or from a message that carries one.":
    "Noch keine. Übernehmen Sie sie unten oder aus einer Nachricht, die einen mitbringt.",
  "what you encrypt to, and check signatures with":
    "womit Sie verschlüsseln und Signaturen prüfen",
  "Create a key": "Schlüssel anlegen",
  "Create key": "Schlüssel anlegen",
  "Creating…": "Wird angelegt…",
  "A modern Ed25519 key with an encryption subkey. The passphrase protects the key on disk and is kept in the system keychain; without one, kuverta makes one up and keeps that. Give the public half to the people who should write to you encrypted: “Copy public key” above.":
    "Ein moderner Ed25519-Schlüssel mit Unterschlüssel zum Verschlüsseln. Die Passphrase schützt den Schlüssel auf der Platte und liegt im Schlüsselbund des Systems; ohne Angabe denkt kuverta sich eine aus und verwahrt sie. Geben Sie die öffentliche Hälfte an alle, die Ihnen verschlüsselt schreiben sollen: „Öffentlichen Schlüssel kopieren“ oben.",
  "Copy public key": "Öffentlichen Schlüssel kopieren",
  "the public key is on the clipboard — paste it into a message or onto a key server":
    "Der öffentliche Schlüssel liegt in der Zwischenablage — fügen Sie ihn in eine Nachricht oder auf einem Schlüsselserver ein",
  "Import keys": "Schlüssel übernehmen",
  "Import the attached key": "Den angehängten Schlüssel übernehmen",
  "Paste a key, or choose a file.": "Fügen Sie einen Schlüssel ein oder wählen Sie eine Datei.",
  "public or secret, one or several": "öffentlich oder geheim, einer oder mehrere",
  "This message carries an OpenPGP key.": "Diese Nachricht bringt einen OpenPGP-Schlüssel mit.",
  "imported the key of {who}": "Schlüssel von {who} übernommen",
  "imported your key {who}": "Ihr Schlüssel {who} übernommen",
  "updated the key of {who}": "Schlüssel von {who} aktualisiert",
  "updated your key {who}": "Ihr Schlüssel {who} aktualisiert",
  "already imported": "schon übernommen",
  "no keys found": "keine Schlüssel gefunden",
  "not imported: {errors}": "nicht übernommen: {errors}",
  "created a key for {email}": "Schlüssel für {email} angelegt",
  "Delete the key of {who}? You can import it again later.":
    "Schlüssel von {who} löschen? Sie können ihn später wieder übernehmen.",
  "Delete your key {who}? Mail encrypted to it can never be read again unless you have a copy elsewhere.":
    "Ihren Schlüssel {who} löschen? Damit verschlüsselte Post ist nie wieder lesbar, außer Sie haben anderswo eine Kopie.",
  "Passphrase (optional)": "Passphrase (freiwillig)",
  "Passphrase again": "Passphrase wiederholen",
  "Passphrase…": "Passphrase…",
  passphrase: "Passphrase",
  "passphrase kept": "Passphrase verwahrt",
  "needs passphrase": "braucht Passphrase",
  "the two passphrases are not the same": "die beiden Passphrasen sind nicht gleich",
  "key {id}": "Schlüssel {id}",
  "an unknown key": "ein unbekannter Schlüssel",
  "created {date}": "angelegt {date}",
  "expires {date}": "läuft ab {date}",
  expired: "abgelaufen",
  revoked: "zurückgezogen",
  encrypts: "verschlüsselt",
  signs: "signiert",
  "Sign with your key, so recipients can tell it is from you":
    "Mit Ihrem Schlüssel signieren, damit Empfänger sehen, dass es von Ihnen ist",
  "Only the recipients and you will be able to read it. The subject is not encrypted.":
    "Nur die Empfänger und Sie können es lesen. Der Betreff wird nicht verschlüsselt.",
  "Encrypted mail cannot have Bcc: each recipient's key is visible in it.":
    "Verschlüsselte Post kann kein Bcc haben: Der Schlüssel jedes Empfängers ist darin sichtbar.",
  "No key for {who} — import it in Settings → Encryption to encrypt.":
    "Kein Schlüssel für {who} — zum Verschlüsseln unter Einstellungen → Verschlüsselung übernehmen.",
  "Encrypted — decrypted with your key.": "Verschlüsselt — mit Ihrem Schlüssel entschlüsselt.",
  "Encrypted, and kuverta could not decrypt it: {error}.":
    "Verschlüsselt, und kuverta konnte es nicht entschlüsseln: {error}.",
  "Signed by {who} — the signature is valid.":
    "Signiert von {who} — die Signatur ist gültig.",
  "Signed by {who} — the signature is valid, but that is not who the message says it is from.":
    "Signiert von {who} — die Signatur ist gültig, aber das ist nicht der Absender, den die Nachricht nennt.",
  "The signature from {who} does not match: the message may have been changed on the way.":
    "Die Signatur von {who} passt nicht: Die Nachricht wurde unterwegs vielleicht verändert.",
  "Signed with a key kuverta does not have ({id}). Import the sender's key to check it.":
    "Mit einem Schlüssel signiert, den kuverta nicht hat ({id}). Übernehmen Sie den Schlüssel des Absenders, um sie zu prüfen.",

  // -- diagnostics and updates ----------------------------------------------
  "detailed logging is on": "ausführliche Protokollierung ist an",
  "detailed logging is off": "ausführliche Protokollierung ist aus",
  "Kept in {path} ({size}).": "Liegt in {path} ({size}).",
  "(the log is empty)": "(das Protokoll ist leer)",
  "could not export the log: {error}": "Das Protokoll ließ sich nicht ausgeben: {error}",
  "selected — press ⌘C (ctrl+C) to copy": "ausgewählt — ⌘C (Strg+C) zum Kopieren",
  "This is kuverta {version}.": "Dies ist kuverta {version}.",
  "kuverta {current} is the latest version": "kuverta {current} ist die neueste Version",
  "kuverta {latest} is available": "kuverta {latest} ist verfügbar",
  "kuverta {latest} is available (you have {current})":
    "kuverta {latest} ist verfügbar (Sie haben {current})",
  Hello: "Hallo",

  // -- index.html: the chrome the window is built from ----------------------
  //
  // Brand, protocol and example values are not here on purpose: DeepSeek,
  // OpenAI, IMAP, TLS, an address like you@example.com and a path like
  // paperless/ read the same in both languages, and translating them would
  // be translating data.
  "Welcome to kuverta": "Willkommen bei kuverta",
  "Set up kuverta": "kuverta einrichten",
  "kuverta puts your email and your paper post in one place and sorts both. This assistant sets it up with you — every step can be skipped and done later from settings.":
    "kuverta bringt Ihre E-Mails und Ihre Papierpost an einen Ort und sortiert beides. Dieser Assistent richtet es mit Ihnen ein — jeder Schritt lässt sich überspringen und später in den Einstellungen nachholen.",
  "First: how do you want to keep your mail apart?":
    "Zuerst: Wie möchten Sie Ihre Post auseinanderhalten?",
  "Later steps ask which profile each account and postal address goes in.":
    "Die nächsten Schritte fragen, in welches Profil jedes Konto und jede Postadresse gehört.",
  "is a set of accounts and postal addresses that belong together — your private mail, one company's, another's. The window shows one profile at a time, or all of them. Leave this empty to keep everything together.":
    "ist eine Gruppe von Konten und Postadressen, die zusammengehören — Ihre private Post, die einer Firma, die einer anderen. Das Fenster zeigt ein Profil nach dem anderen oder alle. Lassen Sie es leer, um alles zusammenzuhalten.",
  profile: "Profil",
  Private: "Privat",
  "Company 1": "Firma 1",
  "Company 2": "Firma 2",
  Work: "Arbeit",
  Home: "Zuhause",
  "All of them": "Alle",
  "Only the one": "Nur das eine",

  // Models
  "Models on this computer": "Modelle auf diesem Rechner",
  "Models available here": "Hier verfügbare Modelle",
  "Reading photographed letters and sorting mail use two small models, run by Ollama on this computer.":
    "Fotografierte Briefe lesen und Post einsortieren brauchen zwei kleine Modelle, die Ollama auf diesem Rechner betreibt.",
  "Would you rather use a hosted service such as DeepSeek or OpenAI? Add it later under Settings → Models; what a job sends then leaves this computer.":
    "Lieber ein gehosteter Dienst wie DeepSeek oder OpenAI? Fügen Sie ihn später unter Einstellungen → Modelle hinzu; was eine Aufgabe dann sendet, verlässt diesen Rechner.",
  "Models run on this computer with Ollama unless you choose otherwise. A hosted service needs no download and is often better and faster — and whatever a job sends it leaves this computer.":
    "Modelle laufen mit Ollama auf diesem Rechner, sofern Sie es nicht anders wählen. Ein gehosteter Dienst braucht keinen Download und ist oft besser und schneller — und was eine Aufgabe ihm sendet, verlässt diesen Rechner.",
  "Looking for Ollama…": "Suche nach Ollama…",
  "Ollama on another computer": "Ollama auf einem anderen Rechner",
  "Another OpenAI-compatible service": "Ein anderer OpenAI-kompatibler Dienst",
  "Reading scanned letters": "Eingescannte Briefe lesen",
  "Sorting mail": "Post einsortieren",
  "The assistant": "Der Assistent",
  "needs a model that can see images": "braucht ein Modell, das Bilder sehen kann",
  "needs a model that can use tools; larger is better":
    "braucht ein Modell, das Werkzeuge benutzen kann; größer ist besser",
  "kuverta classify — any chat model": "kuverta classify — ein beliebiges Chat-Modell",
  "Sees images": "Sieht Bilder",
  Model: "Modell",
  Provider: "Anbieter",
  Service: "Dienst",
  "API key": "API-Schlüssel",
  Refresh: "Neu laden",
  "Remove provider": "Anbieter entfernen",
  Ready: "Bereit",
  Size: "Größe",

  // Paper mail
  "Paper mail": "Papierpost",
  "Scanned letters live in Paperless-ngx. kuverta reads them from there and never changes them.":
    "Eingescannte Briefe liegen in Paperless-ngx. kuverta liest sie von dort und verändert sie nie.",
  "Post scanned into Paperless-ngx shows up beside your mail. Paperless does the scanning, OCR and archive; kuverta only reads it, and never changes a document.":
    "Post, die in Paperless-ngx eingescannt wurde, erscheint neben Ihrer E-Mail. Paperless übernimmt Scannen, Texterkennung und Archiv; kuverta liest nur und ändert nie ein Dokument.",
  "Connect to my Paperless": "Mit meinem Paperless verbinden",
  "Install Paperless here": "Paperless hier installieren",
  "No paper mail for now": "Vorerst keine Papierpost",
  "Paperless address": "Paperless-Adresse",
  "Paperless runs in Docker. kuverta writes its settings to":
    "Paperless läuft in Docker. kuverta schreibt seine Einstellungen nach",
  "in its data folder and starts it; the first start downloads about a gigabyte and takes a few minutes.":
    "in seinem Datenordner und startet es; der erste Start lädt etwa ein Gigabyte und dauert ein paar Minuten.",
  "Reachable from other devices on this network — a scanner has to reach it to deliver post":
    "Von anderen Geräten in diesem Netz erreichbar — ein Scanner muss es erreichen, um Post abzuliefern",
  "Which documents": "Welche Dokumente",
  "Which post belongs here": "Welche Post hierher gehört",
  "one Paperless can hold several addresses' post":
    "ein Paperless kann die Post mehrerer Adressen halten",
  Documents: "Dokumente",
  "all of them": "alle",
  "with the tag": "mit dem Schlagwort",
  "with a tag": "mit einem Schlagwort",
  "from the correspondent": "vom Korrespondenten",
  "from a correspondent": "von einem Korrespondenten",
  "in the storage path": "im Speicherpfad",
  "in a storage path": "in einem Speicherpfad",
  "Name, as it is in Paperless": "Name, wie er in Paperless steht",
  "Use an API token instead": "Stattdessen ein API-Token verwenden",
  "in Paperless under your name → My Profile": "in Paperless unter Ihrem Namen → Mein Profil",
  Connect: "Verbinden",
  "Add another address": "Weitere Adresse hinzufügen",

  // Mail accounts
  "Thunderbird's saved passwords come with its accounts. Apple Mail's stay in the login keychain, so enter each of those once; kuverta keeps it in the system keychain.":
    "Thunderbirds gespeicherte Passwörter kommen mit seinen Konten. Die von Apple Mail bleiben im Anmeldeschlüsselbund, tragen Sie diese also je einmal ein; kuverta verwahrt sie im Schlüsselbund des Systems.",
  "Incoming mail": "Posteingang",
  "Outgoing mail": "Postausgang",
  "SMTP — without it this account cannot send":
    "SMTP — ohne ihn kann dieses Konto nicht senden",
  Server: "Server",
  Method: "Verfahren",
  "Client id": "Client-ID",
  Tenant: "Mandant",
  "Authorise with": "Autorisieren mit",
  "kuverta login --email …": "kuverta login --email …",
  "; the browser step needs a terminal.": "; der Browser-Schritt braucht ein Terminal.",
  "Folders not synced": "Nicht abgerufene Ordner",
  "one per line — a name, or an attribute like \\All":
    "eine pro Zeile — ein Name oder ein Attribut wie \\All",
  "Look for smart mailboxes": "Nach Suchordnern suchen",
  "from Thunderbird and Apple Mail": "aus Thunderbird und Apple Mail",
  "Reads the saved searches Thunderbird keeps in its profiles and the smart mailboxes Apple Mail keeps. Rules kuverta cannot express are listed, never silently dropped.":
    "Liest die gespeicherten Suchen, die Thunderbird in seinen Profilen hält, und die Suchordner von Apple Mail. Regeln, die kuverta nicht ausdrücken kann, werden aufgeführt, nie stillschweigend verworfen.",
  "Bring them over": "Übernehmen",
  "Import the chosen ones": "Die ausgewählten übernehmen",

  // Writing
  "To — comma separated": "An — mit Komma getrennt",
  "Bcc (never appears in the message)": "Bcc (steht nie in der Nachricht)",
  Subject: "Betreff",
  Text: "Text",
  Sign: "Signieren",
  Encrypt: "Verschlüsseln",
  Schedule: "Planen",
  "Send later": "Später senden",
  "Send later…": "Später senden…",
  "Send this later": "Das später senden",
  "Or say when": "Oder sagen, wann",
  "tomorrow 8pm, monday 9:00, in 2 hours": "morgen 20 Uhr, Montag 9:00, in 2 Stunden",
  "Date and time": "Datum und Uhrzeit",
  "Sent at its time while kuverta is open. If it is closed then, a message goes the next time kuverta opens.":
    "Geht zu seiner Zeit hinaus, solange kuverta offen ist. Ist es dann geschlossen, geht die Nachricht beim nächsten Start hinaus.",
  Discard: "Verwerfen",
  "⌘/ctrl+enter sends · escape discards": "⌘/Strg+Enter sendet · Escape verwirft",
  "⌘/ctrl+enter sends, as a reply to their latest message. Double-click a bubble for the whole message.":
    "⌘/Strg+Enter sendet, als Antwort auf die letzte Nachricht. Doppelklick auf eine Sprechblase zeigt die ganze Nachricht.",
  "Write a reply…": "Antwort schreiben…",

  // The assistant panel
  "Ask about your mail, or say what to do with it…":
    "Fragen Sie nach Ihrer Post oder sagen Sie, was damit geschehen soll…",
  "New chat": "Neues Gespräch",
  "Start a new conversation": "Ein neues Gespräch beginnen",
  "Close the assistant": "Den Assistenten schließen",
  "Close (i)": "Schließen (i)",
  "Suggestions:": "Vorschläge:",
  "Waiting for you": "Wartet auf Sie",
  "Run all now": "Alle jetzt ausführen",
  "Approve all moves": "Alle Verschiebungen freigeben",

  // Tasks
  "New task…": "Neue Aufgabe…",
  "For mail that matches": "Für Post, auf die zutrifft",
  "of these rules:": "dieser Regeln:",
  all: "alle",
  any: "eine",
  "Do this": "Das tun",
  "Move to a folder": "In einen Ordner verschieben",
  "File under a category": "Unter einer Kategorie einsortieren",
  "Draft a reply (the model writes it)": "Eine Antwort entwerfen (das Modell schreibt sie)",
  "Let the model decide": "Das Modell entscheiden lassen",
  "Instruction for the model": "Anweisung für das Modell",
  "Thank them, say we will look into it within two working days, and ask for their order number if they did not give it.":
    "Bedanke dich, sage zu, dass wir uns innerhalb von zwei Werktagen darum kümmern, und frage nach der Bestellnummer, falls sie fehlt.",
  "Invoices to Rechnungen": "Rechnungen nach Rechnungen",
  Receipts: "Belege",
  "Ask me first — leave each action under “Waiting for you”":
    "Mich zuerst fragen — jede Aktion unter „Wartet auf Sie“ lassen",
  "Also for mail already there, not only new mail":
    "Auch für Post, die schon da ist, nicht nur für neue",
  "Runs after every sync on mail it has not dealt with yet. Every change it makes can be undone.":
    "Läuft nach jedem Abruf über Post, um die es sich noch nicht gekümmert hat. Jede Änderung lässt sich rückgängig machen.",

  // Smart mailboxes
  "New smart mailbox…": "Neuer Suchordner…",
  "Smart mailbox…": "Suchordner…",
  "A smart mailbox gathers mail by rules — who sent it, what it says, how old it is — without moving any of it. It sits in the sidebar and works like any mailbox.":
    "Ein Suchordner sammelt Post nach Regeln — wer sie geschickt hat, was darin steht, wie alt sie ist — ohne etwas zu verschieben. Er steht in der Seitenleiste und verhält sich wie jeder Ordner.",
  "Show mail that matches": "Post zeigen, auf die zutrifft",
  "+ Add a rule": "+ Regel hinzufügen",
  common: "häufig",
  "Rules are checked against every message on the account. Nothing is moved.":
    "Die Regeln werden gegen jede Nachricht des Kontos geprüft. Es wird nichts verschoben.",
  Create: "Anlegen",

  // Folders, attachments, similar
  Folder: "Ordner",
  Inside: "In",
  Category: "Kategorie",
  "Similar messages": "Ähnliche Nachrichten",
  "Don't ask again": "Nicht mehr fragen",
  "The attachment": "Der Anhang",
  "The scanned letter": "Der eingescannte Brief",
  "Save to Downloads": "In Downloads sichern",
  "Open in another app": "In einem anderen Programm öffnen",
  PDF: "PDF",

  // General and diagnostics
  "The assistant that opened on the first run: local models, Paperless, and the mail accounts other programs on this computer know.":
    "Der Assistent, der beim ersten Start aufging: lokale Modelle, Paperless und die E-Mail-Konten, die andere Programme auf diesem Rechner kennen.",
  "After deleting a message, offer to delete messages like it too":
    "Nach dem Löschen einer Nachricht anbieten, ähnliche mitzulöschen",
  "Spam and bulk mail come in runs. When a message you delete has look-alikes — the same sender and subject pattern, the same list, or the same text from different senders — kuverta shows them and lets you delete them together or gather them in a smart mailbox.":
    "Spam und Massenpost kommen in Serien. Hat eine gelöschte Nachricht Doppelgänger — derselbe Absender und Betreffmuster, dieselbe Liste oder derselbe Text von verschiedenen Absendern — zeigt kuverta sie und lässt Sie sie zusammen löschen oder in einem Suchordner sammeln.",
  "After each sync, let the model for sorting mail judge how soon new mail needs me":
    "Nach jedem Abruf das Modell zum Einsortieren beurteilen lassen, wie bald neue Post mich braucht",
  "kuverta's urgency agent always checks what it can itself — whether you write to the sender, whether you have answered, deadline and payment words. With this on, it also shows the model chosen under Models → Sorting mail the start of each new message that is not bulk mail, with those findings, and asks how soon it needs you and why. With a hosted model, that text leaves this computer.":
    "kuvertas Dringlichkeitsagent prüft immer selbst, was er kann — ob Sie dem Absender schreiben, ob Sie geantwortet haben, Fristen und Zahlungswörter. Ist das an, zeigt er dem unter Modelle → Post einsortieren gewählten Modell zusätzlich den Anfang jeder neuen Nachricht, die keine Massenpost ist, samt dieser Funde, und fragt, wie bald sie Sie braucht und warum. Bei einem gehosteten Modell verlässt dieser Text den Rechner.",
  "Detailed logging": "Ausführliche Protokollierung",
  "Record what kuverta does in detail": "Ausführlich festhalten, was kuverta tut",
  "Turn this on, do what went wrong again, then export the log and attach it to a bug report. Detailed logs grow faster; turn it off again when you are done. The log never holds passwords or tokens. It does hold addresses and subjects, and with detailed logging on it can hold what a model answered about a message, which may quote from it.":
    "Schalten Sie das ein, machen Sie das Fehlerhafte noch einmal, geben Sie dann das Protokoll aus und hängen Sie es an einen Fehlerbericht. Ausführliche Protokolle wachsen schneller; schalten Sie es danach wieder aus. Das Protokoll enthält nie Passwörter oder Token. Es enthält Adressen und Betreffzeilen, und bei ausführlicher Protokollierung auch, was ein Modell über eine Nachricht geantwortet hat, worin aus ihr zitiert sein kann.",
  "Recent log": "Letztes Protokoll",
  "Export log…": "Protokoll ausgeben…",
  "None (localhost only)": "Keine (nur localhost)",

  // The keyboard strip at the foot of the window
  "j/k move": "j/k bewegen",
  "J/K select": "J/K auswählen",
  "enter read": "Enter lesen",
  "e archive": "e archivieren",
  "⌫ trash": "⌫ löschen",
  "u unread": "u ungelesen",
  "z undo": "z rückgängig",
  "r sync": "r abrufen",
  "c compose": "c schreiben",
  "R reply": "R antworten",
  "A reply all": "A allen antworten",
  "i assistant": "i Assistent",
  "v text/pdf": "v Text/PDF",
  "/ search": "/ suchen",
  ", settings": ", Einstellungen",
  "I ask about this": "I dazu fragen",
  "newest at the bottom": "neueste unten",

  // -- tables the window looks strings up in --------------------------------
  //
  // Passed to `t` as values rather than written out at the call, so the test
  // that checks the catalogue against the code cannot see them: these are
  // the urgency levels and actions, and a smart mailbox's fields and
  // operators. Keep them in step by hand.
  today: "heute",
  "this week": "diese Woche",
  "can wait": "kann warten",
  "nothing to do": "nichts zu tun",
  reply: "antworten",
  pay: "zahlen",
  attend: "hingehen",
  decide: "entscheiden",
  read: "lesen",
  Sender: "Absender",
  Recipient: "Empfänger",
  "Message text": "Nachrichtentext",
  "Mailing list": "Verteiler",
  Mailbox: "Ordner",
  Unread: "Ungelesen",
  "Has an attachment": "Hat einen Anhang",
  "Can be unsubscribed from": "Lässt sich abbestellen",
  "Older than (days)": "Älter als (Tage)",
  "Received in the last (days)": "Erhalten in den letzten (Tagen)",
  contains: "enthält",
  "does not contain": "enthält nicht",
  is: "ist",
  "is not": "ist nicht",
  "begins with": "beginnt mit",
  "ends with": "endet auf",
  "Every message, newest first": "Jede Nachricht, neueste zuerst",
  "One row per person; open one to see your exchange as a conversation":
    "Eine Zeile je Person; öffnen Sie eine, um Ihren Austausch als Gespräch zu sehen",

  // -- what the window says as it goes -------------------------------------
  "syncing…": "wird abgerufen…",
  "Paperless is running.": "Paperless läuft.",
  "starting Paperless…": "Paperless wird gestartet…",
  "starting Docker…": "Docker wird gestartet…",
  "could not start Paperless": "Paperless ließ sich nicht starten",
  "could not load messages": "Nachrichten konnten nicht geladen werden",
  "nothing to show": "nichts zu zeigen",
};

const CATALOGUES = { de: GERMAN };

/// What the window is being read in: a choice that was made, or the system's.
///
/// The choice is kept where the other things the window remembers are kept —
/// beside the sort order and whether the assistant was open — because it is
/// the same kind of thing: how this person likes their window, not what is in
/// their mail.
function languageChoice() {
  return storedChoice();
}

function storedChoice() {
  try {
    return localStorage.getItem("language") ?? "system";
  } catch {
    return "system";
  }
}

function systemLanguage() {
  const tag = (navigator.language || "en").toLowerCase();
  return tag.startsWith("de") ? "de" : "en";
}

/// The language in force: `en` or `de`.
function language() {
  const choice = storedChoice();
  return choice === "system" ? systemLanguage() : choice;
}

/// One string, in the language in force.
///
/// `vars` fills `{name}` placeholders, so a sentence stays one sentence in
/// both languages rather than three pieces glued together in English order.
function t(text, vars = null) {
  const catalogue = CATALOGUES[language()];
  // A key may carry a qualifier after a bar, for the few English words that
  // are two different things: `t("Archive|a folder's name")` is a folder,
  // `t("Archive")` is the button that puts mail in one. The qualifier is
  // never shown — in English it is cut off, in German it picks the entry.
  let out = (catalogue && catalogue[text]) || text.split("|")[0];
  if (vars) {
    for (const [name, value] of Object.entries(vars)) {
      out = out.replaceAll(`{${name}}`, String(value));
    }
  }
  return out;
}

/// Everything already on the page, translated in place.
///
/// Walks text and the attributes people read — placeholder, title, the label
/// a screen reader says — rather than asking index.html to carry a key on
/// every element. Run once at startup and again when the language changes.
function translateDom(root = document.body) {
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const texts = [];
  for (let node = walker.nextNode(); node; node = walker.nextNode()) texts.push(node);
  for (const node of texts) {
    const trimmed = node.nodeValue.trim();
    if (!trimmed) continue;
    const translated = t(trimmed);
    if (translated === trimmed) continue;
    // Keep whatever spacing the markup had around it, so inline text that
    // sits beside an element does not lose the gap in front of it.
    node.nodeValue = node.nodeValue.replace(trimmed, translated);
  }
  for (const element of root.querySelectorAll("[placeholder], [title], [aria-label]")) {
    for (const name of ["placeholder", "title", "aria-label"]) {
      const value = element.getAttribute(name);
      if (value) element.setAttribute(name, t(value.trim()));
    }
  }
  document.documentElement.lang = language();
}

/// Changes the language and redraws what is on screen.
///
/// The page is not reloaded: a reload would lose the open message, the place
/// in the list and anything half typed. What the window built itself is drawn
/// again by whoever owns it — the sidebar and the list — and the rest is
/// static text this walks over.
function setLanguageChoice(choice) {
  try {
    localStorage.setItem("language", choice);
  } catch {
    // A window that cannot remember the choice can still honour it now.
  }
  translateDom();
}
