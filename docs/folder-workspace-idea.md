# Produktidee für eine gemeinsame Ablage mit IMAP Ordnern

## Ergänzung: Rechnungen und bestehende Dateiablage

Die erste lokale Umsetzung bindet je Konto einen vorhandenen Stammordner an,
liest ausschließlich Verzeichnisnamen und bietet bestätigbare Kopien einzelner
Mailanhänge. Sie greift nicht auf bestehende Dokumentinhalte zu. Tests verwenden
eine separate Ablage. Beispielpfade aktivieren keine Kontozuordnung.

Konkreter Folgeanwendungsfall: „Lege alle Rechnungen aus Q3 2026 in Ordner XYZ ab“
für vorbereitende Buchhaltung von Beispielfirma A. **Präzisierung vom 18. Oktober 2026:
Die Sammlung erfolgt je Profil.** Alle diesem Profil zugeordneten Postfächer
liefern gemeinsam Belege für einen Lauf und einen gemeinsamen Ablagevorschlag.
Andere Profile bleiben getrennt; es gibt weder eine globale Sammlung aller
Konten noch einen eigenen Quartalsexport pro Postfach. Welche Konten dazugehören,
bestimmt ihre bewusste Profilzuordnung.

Je Profil werden Ablageziel, wiederkehrender Auftrag und Exporthistorie geführt.
Die Dublettenprüfung umfasst alle beteiligten Postfächer dieses Profils und
frühere Läufe desselben Profils. Gleiche Belege in einem anderen Profil werden
nicht deshalb stillschweigend ausgelassen. Die Vorschau nennt das Profil, die
einbezogenen Postfächer und die Herkunft jedes Belegs; anschließend wird die
Sammlung gemeinsam freigegeben. Bestehende Profile oder Kontozuordnungen werden
durch diese Planung nicht automatisch geändert.

Ein wiederholbarer Sammellauf braucht außerdem eine belegbezogene
Zeitraumprüfung (Rechnungsdatum, nicht bloß Maildatum), getrennte Gutschriften und
unklare Fälle, nachvollziehbare Suchabdeckung, Dublettenprüfung über wiederholte
Läufe, gemeinsame Zielvorschau, ausdrückliche Freigabe und ein Ablageprotokoll.
Links auf externe Rechnungsportale müssen als nicht abgelegte Belege erscheinen.
Der profilbezogene Export ist inzwischen mit gemeinsamer Vorschau umgesetzt;
wiederkehrende automatische Sammelläufe bleiben offen.

## Ergänzung vom 17. Oktober 2026: Kontext und Aufmerksamkeit

Der Nutzer möchte anpassbare Aufmerksamkeitskriterien und eine sparsame Prüfung
großer ungelesener Bestände: zuerst Absender/Betreff, danach ausgewählte Inhalte.
Die lokale Umsetzung bietet kontobezogene Begriffslisten, bestätigbare Vorschläge
des Assistenten und eine paginierte Kopfzeilenprüfung. „Braucht Aufmerksamkeit“
bleibt eine begrenzte Bewertung jüngerer Posteingangsmails, kein vollständiger
Nachweis aller wichtigen Nachrichten. Details stehen im Benutzerhandbuch
`website/docs/user-guide/attention.md`.

Die derzeitige KI-Sortierung vergibt Kategorien; die Aufmerksamkeitsbewertung
ordnet eine Ansicht. Erst Assistentenwerkzeuge oder eingerichtete Aufgaben
verschieben Mails, setzen Lesestatus oder legen Ordner an. Der Mail-Assistent
arbeitet innerhalb eines gewählten Kontos und kann dort gespeicherte Mails,
Anhänge, Gespräche, Notizen und von ihm erzeugte Dokumente verwenden. Er kann
Entwürfe vorbereiten, aber keine E-Mail selbst versenden. Webrecherche erfordert
die vorhandene gesonderte Einstellung.

Eine gemeinsame Ablage, beliebige lokale Dateien, andere Konten oder ERP-Daten
werden dadurch nicht automatisch zugänglich. Für die spätere Umsetzung müssen
Ablageorte ausdrücklich angebunden, Projekte/Kunden mit stabilen Kennungen
verknüpft, Quellen benannt und Zugriffe begrenzt werden. Gemeinsame Ordnernamen
allein belegen keine inhaltliche Beziehung. Als nächste Ausbaustufe ist eine
Suche über Metadaten/Index und anschließendes gezieltes Nachladen sinnvoll;
diese übergreifende Ablageintegration ist noch nicht implementiert.

Idee aus Nutzerfeedback, festgehalten am 16. Oktober 2026. Status: Vorschlag zur
Produktentwicklung, noch keine beschlossene Umsetzung.

Kuverta soll E-Mails in echten Ordnern des Postfachs organisieren. Die Ablage
soll dadurch auch in anderen Mailprogrammen und im Webmail sichtbar bleiben.
Eine Sortierung, die nur als Kategorie oder Tag innerhalb von Kuverta existiert,
erfüllt diesen Wunsch nicht.

## Die vorhandene Ablage als Ausgangspunkt

Die IMAP-Ordner sollen sich an der Struktur orientieren, die eine Person oder
ein Unternehmen bereits für seine Arbeit verwendet. Beim Einrichten eines
Postfachs soll Kuverta diese Struktur übernehmen oder gemeinsam mit dem Nutzer
abbilden können. Die KI soll passende Zielordner innerhalb dieser Struktur
vorschlagen und perspektivisch beim Einsortieren helfen.

Das konkrete Beispiel ist Beispielfirma A: Wenn dort etwa zwölf Ablageordner verwendet
werden, soll das Beispielfirma A-Postfach eine entsprechende Struktur bekommen. Die
genauen Ordnernamen und ihre Zuordnung sind noch zu bestimmen. Die Zahl zwölf
ist ein Beispiel, keine feste Vorgabe.

Kategorien und Tags können ergänzende Ansichten liefern. Die eigentliche
Ablage soll jedoch im Postfach erhalten bleiben und unabhängig von Kuverta
nutzbar sein.

## E Mail als Teil eines gemeinsamen Arbeitsbereichs

Langfristig sollen E-Mails, Aufgabenverwaltung und Dokumentenablage ineinander
greifen. Eine Nachricht könnte beispielsweise mit einer Aufgabe und den
zugehörigen Unterlagen verbunden werden, wobei alle dieselbe fachliche
Zuordnung verwenden.

Kuverta wäre damit ein wesentlicher Bestandteil eines umfassenderen Werkzeugs
für private Organisation oder betriebliche Arbeit. Je nach Nutzung sollen
Struktur und Funktionen dazu passen. Ein vollständiges ERP mit etwa Warenwirtschaft
und Buchhaltung ist damit noch nicht als Produktumfang festgelegt.

## Vorhandene Grundlagen und die gewünschte Erweiterung

Kuverta unterstützt bereits IMAP-Ordner, das Verschieben von Nachrichten,
Profile sowie einen Assistenten und Aufgaben. Die sechs allgemeinen
Mailkategorien und der Modellvergleich zur Klassifizierung bilden dagegen
noch keine individuelle betriebliche Ordnerstruktur ab. Der derzeitige
`Core::model_pass` speichert KI-Kategorien zum Vergleich und verschiebt dadurch
keine Nachrichten.

Der nächste fachliche Schritt wäre deshalb eine Zuordnung von Nachrichten zu
den vom Nutzer gewählten Zielordnern. Die bestehende Ordnerverwaltung und die
Operationen zum Verschieben wären dafür die Grundlage.

## Fragen für die weitere Ausarbeitung

- Welche vorhandene Ablage ist maßgeblich: ein Verzeichnis, eine Dokumentenverwaltung
  oder eine von Hand gepflegte Struktur?
- Wie werden diese Bereiche auf IMAP-Ordner abgebildet, einschließlich Unterordnern
  und abweichenden Namen beim Mailanbieter?
- Wann genügt ein Vorschlag, und wann darf Kuverta automatisch einsortieren?
  Wie werden unklare Fälle und Korrekturen behandelt?
- Wie bleiben Zuordnungen zu Aufgaben und Dokumenten erhalten, wenn Nachrichten
  oder Ordner auch in einem anderen Mailprogramm verschoben oder umbenannt werden?
- Was gehört zur privaten Grundausstattung, was zu einem Unternehmensprofil?

Ein erster begrenzter Versuch könnte die tatsächliche Beispielfirma A-Struktur aufnehmen,
an einer Postfachkopie passende Ordner vorschlagen und anschließend mit ausgewählten
Testnachrichten prüfen, ob eine Ablage über IMAP in einem zweiten Client sichtbar
bleibt. Dieser Vorschlag ist ein möglicher nächster Schritt, kein bereits
ausgeführter Test.

## Bezug zur bestehenden Dokumentation

Der [ursprüngliche Projektentwurf](original-brief.md) nennt automatische
Sortierung und die spätere Anbindung von KI-Agenten. Die
[Funktionsübersicht](../README.md) beschreibt die vorhandenen Ordner, Profile,
Assistenten und Aufgaben. Diese Idee konkretisiert die gewünschte Ablage über
Programmgrenzen hinweg.

## Ergänzende Produktnotizen aus Nutzerfeedback (17. Oktober 2026)

Die folgenden Beobachtungen stammen aus der Nutzung **vor** den lokalen Änderungen
in diesem Chat. Sie sind Ideen bzw. noch zu überprüfende Fehlerberichte, keine
Bestätigung, dass der aktuelle Build alle genannten Probleme weiterhin hat.

- Name: „KUVERTAI“ oder „QVERTA“ als Alternativen diskutieren; Nutzerfeedback verweist auf
  die kroatische Bedeutung von „Kuverta“.
- Im Einrichtungsassistenten eine Option „Mit Windows starten“ anbieten.
- Verständliche Update-Prüfung und automatische Updates prüfen.
- Gleich im ersten Einrichtungsschritt zwischen „Nur E-Mail“ und „E-Mail und
  Papierpost“ wählen; ohne Papierpost die entsprechenden Schritte auslassen.
- Bei benötigter Zusatzsoftware einen verständlichen Windows-Download und einen
  direkten Weg zur Installation anbieten. Den Ablauf für Menschen ohne
  IT-Hintergrund gestalten.
- Früherer Fehler in Schritt 5 bei der Suche nach E-Mail-Einstellungen:
  `TypeError: Cannot set properties of null (setting 'hidden')`.
  Danach ließ sich die Einrichtung nicht fortsetzen. Aktuellen Stand gezielt prüfen.
- Eine Funktion „Fehler melden“ direkt in Kuverta anbieten, mit überprüfbaren
  Diagnosedaten und ohne versehentliche Weitergabe von Mailinhalten oder Passwörtern.
- Android: statt einer Meldung für nahezu jede Mail eine Zusammenfassung wie
  „Du hast N wichtige Mails in N Konten“. Der aktuelle Vergleich ist Gmail mit
  vielen, teils unregelmäßigen Benachrichtigungen pro Konto.

Kurzfristiges Ziel bleibt eine alltagstaugliche lokale Windows-Installation für
bestehende Postfächer der nutzenden Person; diese Ideen erweitern den aktuellen Importauftrag
nicht automatisch um eine vollständige Umsetzung.
