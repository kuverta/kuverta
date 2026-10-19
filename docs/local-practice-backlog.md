# Lokale Entwicklung: Funktionsstand und nächste Schritte

Stand: 19. Oktober 2026. Diese öffentliche Übersicht enthält technische
Ergebnisse und neutrale Beispiele. Individuelle Postfachnamen, Profile, Pfade,
Statistiken, Dokumentinhalte und Installationsprotokolle gehören ausschließlich
in lokale, von Git ausgeschlossene Aufzeichnungen. Technische Prüfungen:
[releasing.md](releasing.md).

## Umgesetzt

- Anbieterspezifische Modellauswahl für alle KI-Aufgaben; Auswahl direkt im
  Assistenten und sichtbarer Hinweis auf Verarbeitung durch einen Onlinedienst.
- Korrigierte Phishing-Priorität ohne erfundene Handlungspflicht;
  rustls-/HTML-Fuzz-Korrekturen im lokalen Entwicklungsstand integriert.
- Windows-Entwicklungsumgebung, eM-Client-Kontoimport und verständliche IMAP-Namen.
  Der Assistent berücksichtigt Server-Namensräume beim Anlegen von Ordnern.
- Optionale graue Hervorhebung ungelesener Mails, Ungelesenfilter über Ansichten
  hinweg, Lesen beim Öffnen und Thread-Gruppierung. HTML/Text auch in Threads.
  Optionale Vor-/Zurück-Navigation für besuchte Mails, Ordner und Profile,
  unabhängig von Rückgängig/Wiederherstellen.
- Stabilere Aufmerksamkeitsnavigation und editierbare kontobezogene Kriterien.
  Assistentenvorschläge brauchen Bestätigung. Große Bestände zuerst über
  paginierte Absender-/Betrefflisten prüfen, dann ausgewählte Inhalte nachladen.
- E-Mail-only-Einrichtung, optionale Kategorien und Windows-Autostart;
  DNS-Veröffentlichungsprüfung mit Grenzen der DKIM-Selector-Erkennung.
- Reparatur der Cleanup-Nachrichtenliste, verständlichere Fehler und Filter.
- Kontobezogene Einzelablage: vorhandenen Stammordner anbinden, ausschließlich
  Ordnernamen lesen und einzelne Anhangkopien nach Zielvorschau bestätigen.
- Gemeinsame Rechnungssammlung je Profil über alle zugeordneten Postfächer.
  Andere Profile bleiben getrennt. Der Assistent kann eine Sammlung für Profil
  und Zeitraum vorschlagen, aber nicht selbst freigeben.
- Einrichtungs-Wizard je Profil: Firmenidentität/Schreibweisen, Anschrift,
  Personen, Belegarten, Prüfbegriffe, Kontenbestätigung, Ablage und Standardzeitraum.
  Entwürfe bleiben gespeichert; vorhandene Konten allein bestätigen kein Profil.
- Monat, Quartal, Jahr und freie Von-/Bis-Zeiträume, auch über Jahresgrenzen.
  Maildatum begrenzt die Suche, geprüftes Rechnungsdatum bestimmt die Zuordnung.
- Optionale Analyse lesbaren Dokumenttexts: Datum, Aussteller, Empfänger,
  Nummer, Betrag und Korrekturhinweise mit Fundstellen. Unbelegte, persönliche,
  Test-/Musterbelege und mögliche Fassungen bleiben zur Prüfung. Ordinaldaten
  werden am Original validiert; alte Analysen können ohne neuen Provideraufruf
  erneut lokal geprüft werden. Scans bleiben manuell.
- Vorschaufilter für Zeitraum, Auswahl, andere Dokumente, Dubletten und Prüfbedarf.
  Quelle ansehen ohne Lesestatusänderung. Korrekturen mit Grund und Rücknahme
  dauerhaft je Profil/exakten Dateiinhalt speichern, nicht pauschal je Absender.
  Gültige Vorschau nach Neustart ohne neue Analyse fortsetzen. Geänderte
  Einrichtung verlangt erneute Prüfung alter Entscheidungen.
- SHA-256-Dublettenprüfung über Profilpostfächer und frühere Kuverta-Exporte;
  mögliche Versionen zusätzlich über belegte Rechnungsidentitäten. Wiederaufnahme
  unterbrochener Kopien, kein Überschreiben fremder Dateien, Ablageprotokoll.
- Windows-Ordner-Picker in Wizard/Sammlung. Ablage direkt im Basisordner oder
  Jahr/Quartal/Monat, etwa `2026/Q3/09`. Vorhandene gleichnamige Ordner verwenden,
  fehlende nur nach Freigabe anlegen. Zielordner vorab sichtbar. Vorhandene
  Dateien werden nicht automatisch umorganisiert oder inhaltlich indexiert.
- Lokaler Assistent: 16.384 Kontext-Tokens statt eines zu kleinen Serverstandards,
  maximal 2.048 Ausgabetokens pro Antwort. Verständlicher Kontextüberlauf,
  kein stilles Entfernen des Verlaufs und kein Cloud-Fallback.

Details: [Rechnungssammlung](../website/docs/user-guide/filing.md).
Jüngste Validierung: 59 gezielte automatische Tests und ein echter lokaler
Modellaufruf mit synthetischem Inhalt. Das bestätigt keine allgemeine
Erkennungsqualität bei echten Postfächern.

## Offen oder bewusst begrenzt

- Lokale Modelle mit begrenzter, manuell bewerteter Stichprobe getrennt für
  Sortierung, Spam/Phishing, Belege und Werkzeuge messen. Fehlalarme wichtiger
  Mails, Enthaltungen, Laufzeit und RAM-/VRAM-Spitzen ausweisen. Der Speicherbedarf
  steigt mit der Kontextgröße; keine automatische maximale Auslastung.
- Aufmerksamkeit bewertet jüngere Posteingangsmails, nicht den gesamten Altbestand.
  Suchabdeckung und ausgelassene Bereiche benennen. Kategorienvergleich bedeutet
  kein IMAP-Verschieben. Belastbarer Spam-Fehlalarmvergleich bleibt offen.
- Regeln aus wiederholten Korrekturen nur mit Beispielen/Gegenbeispielen vorschlagen
  und nach Freigabe übernehmen; bearbeitbar halten. Einzelne Privat-/Testbelege
  begründen keine pauschale Lieferantensperre.
- Wiederkehrende Sammelläufe, Import bereits manuell abgelegter Belege und
  Status externer Übermittlungen fehlen noch. Vorhandene lokale Dateien gehören
  nicht automatisch zur Dublettenhistorie.
- Vollständige SPF-/DKIM-/DMARC-Versandvalidierung und weitere Vergleiche mit einem
  zweiten Client; verständliche Updates und Fehlerberichte mit vorab prüfbaren,
  von Mailinhalten/Zugangsdaten bereinigten Diagnosen.
- Android-Sammelbenachrichtigungen, gemeinsame Datei-/Aufgabenablage und
  unternehmensweite Zuordnung: [Produktidee](folder-workspace-idea.md).

## Mögliche Buchhaltungskette mit Finom und DATEV

Vorgeschlagen: profilbezogene Belegsammlung in Kuverta → gemeinsame Freigabe →
Übernahme bestätigter Originalbelege in Finom → Transaktionsabgleich → DATEV und
Buchhaltung. Eine Ordnerkopie ist keine Zahlungsbestätigung. Diese Anbindung ist
geplant; Kuverta richtet keine Bankverbindung ein und übermittelt noch keine Belege.

Offizielle Informationen, geprüft am 19.10.2026:

- [Finom Accounting](https://finom.co/de-de/business-account-accounting/)
  beschreibt Belegabgleich und Hinweise auf fehlende Dokumente. Erweiterte
  GmbH/UG-Funktionen sind teilweise Beta bzw. angekündigt; Verfügbarkeit im
  konkreten Firmenkonto vor Integration prüfen.
- [DATEV-Belegbilderservice](https://help.finom.co/de/articles/10257039-wie-verbinde-ich-mein-finom-konto-mit-datev-belegbilderservice)
  überträgt Belege an DATEV Unternehmen Online. Benötigt aktiviertes DUO,
  Berater-/Mandantennummer und DATEV SmartLogin über den Steuerberater.
- [Kontoumsätze in DATEV](https://help.finom.co/de/articles/9777018-wie-verbinde-ich-mein-finom-konto-mit-datev-unternehmen-online)
  sind getrennt angebunden; der Einrichtungsweg hängt auch vom Kontotyp ab.

Mandanten-/Profilzuordnung und einen einzigen Beleg-Übertragungsweg mit der
Buchhaltung festlegen. Zuerst wenige freigegebene Testbelege mit Herkunft, Hash
und Rückmeldung über Upload/Abgleich prüfen. Später Übertragungsstatus je Profil/
Datei speichern und Wiederholungen ohne doppelte Anlage erlauben. Andere
Zahlungswege, privat verauslagte betriebliche Belege, Gutschriften und Sammelzahlungen
berücksichtigen; ein einzelnes Bankkonto beweist keine Vollständigkeit.

## Grenzen und Veröffentlichung

Reale Massenänderungen, Löschungen, Exporte, DNS-Änderungen und Übermittlungen
brauchen einen konkret freigegebenen Auftrag. Tests verwenden synthetische Daten.
Öffentliche Dokumentation enthält technische Erkenntnisse, keine privaten
Praxisprotokolle. Alte lokale Commits können solche Protokolle weiterhin enthalten:
vor Veröffentlichung einen bereinigten Patch auf öffentlicher Basis erstellen,
einschließlich Prüfung der Historie. Siehe
[Veröffentlichungshinweise](releasing.md#publication-hygiene).
