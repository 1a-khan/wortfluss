# Wortfluss installieren und benutzen

*English: [INSTALL.md](INSTALL.md)*

Wortfluss macht aus Sprache Text, in jedem Programm: Taste halten, sprechen, loslassen, und
der Text wird dort getippt, wo der Cursor steht. Außerdem verbessert es markierten Text und
macht aus Vorlesungen oder Besprechungen Notizen.

**Download:** immer von der offiziellen Seite
**[github.com/1a-khan/wortfluss/releases/latest](https://github.com/1a-khan/wortfluss/releases/latest)**
(Abschnitt *Assets*). Danach aktualisiert sich Wortfluss selbst.

## 1. API-Schlüssel besorgen (5 Minuten, einmalig)

Wortfluss schickt Ihre Sprache an einen Spracherkennungsdienst Ihrer Wahl, mit **Ihrem
eigenen Schlüssel**. Sie zahlen also nur, was Sie nutzen (oft gar nichts).

**Empfohlen: Groq** (der kostenlose Tarif reicht für normales Diktieren):

1. Auf [console.groq.com](https://console.groq.com) registrieren (Google-Konto oder E-Mail).
2. **API Keys** → **Create API Key** öffnen, einen Namen vergeben, z. B. „Wortfluss“.
3. Den Schlüssel kopieren (beginnt mit `gsk_`). Sie fügen ihn in Schritt 3 ein.

Alternativen: OpenRouter, OpenAI oder Mistral; die Einrichtung erklärt jeden Dienst.
Halten Sie den Schlüssel geheim: nicht per E-Mail schicken und nirgends veröffentlichen.

## 2. Installieren

### Windows 10 / 11

1. **`Wortfluss-Setup-<Version>.exe`** von der
   [Download-Seite](https://github.com/1a-khan/wortfluss/releases/latest) herunterladen.
2. Doppelklicken. Windows zeigt eventuell *„Der Computer wurde durch Windows geschützt“*
   (das Installationsprogramm ist noch nicht signiert): **Weitere Informationen** →
   **Trotzdem ausführen**.
3. Durch die Installation klicken. Administratorrechte sind nicht nötig.
   **„Wortfluss bei der Anmeldung starten“** angehakt lassen.
4. Am Ende **„Wortfluss jetzt einrichten“** angehakt lassen und **Fertigstellen** klicken.

**Ist das sicher? (Warnungen von Virenscannern)** Wortfluss muss Dinge tun, die
Virenscanner auch bei Spionage-Software sehen: Es achtet auf seine Diktiertaste, nimmt das
Mikrofon auf, schickt den Ton an den gewählten Sprachdienst, fügt Text in andere Apps ein
und aktualisiert sich selbst. Ein Scanner kann deshalb warnen, vor allem solange das
Programm noch nicht signiert ist.

- Prüfen, ob es die echte Datei ist: Rechtsklick auf `vtt.exe` → **Eigenschaften** →
  **Details** zeigt *Wortfluss*, *MIAK IT (Ammad Khan)* und die Version. Die Datei
  `SHA256SUMS` auf der Download-Seite enthält die Prüfsumme jeder Datei.
- Der vollständige Quellcode ist öffentlich:
  [github.com/1a-khan/wortfluss](https://github.com/1a-khan/wortfluss). Ton und Text
  gehen nur an den eingerichteten Dienst; sonst fragt Wortfluss nur bei GitHub nach
  Updates.
- Wenn ein Scanner das Programm blockiert, bitte uns (oder dem Hersteller des Scanners)
  Bescheid geben, damit es freigegeben wird. Firmen-PCs: Die IT kann den Ordner
  `%LOCALAPPDATA%\Programs\Wortfluss` freigeben.

### Linux (Ubuntu und andere Distributionen, Wayland oder X11)

Ein Terminal öffnen und diese Befehle nacheinander ausführen:

```bash
# 1. Programm herunterladen und installieren
mkdir -p ~/.local/bin
curl -L -o ~/.local/bin/vtt https://github.com/1a-khan/wortfluss/releases/latest/download/vtt-linux-x86_64
chmod +x ~/.local/bin/vtt

# 2. Diktiertaste und Tippen erlauben (fragt einmal pro Computer nach Ihrem Passwort)
sudo usermod -aG input "$USER"
echo 'KERNEL=="uinput", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"' \
  | sudo tee /etc/udev/rules.d/80-vtt-uinput.rules
sudo udevadm control --reload-rules && sudo udevadm trigger
```

Dann **ab- und wieder anmelden** (einmalig nötig für Schritt 2), wieder ein Terminal öffnen
und ausführen:

```bash
vtt setup
```

Falls `vtt` nicht gefunden wird: `~/.local/bin/vtt setup`.
Schritt 2 erlaubt Wortfluss, die Diktiertaste zu sehen und für Sie zu tippen. Jedes Programm
in der Gruppe `input` kann die Tastatur lesen; nutzen Sie das daher auf Computern, die nur
Sie verwenden.

## 3. Ersteinrichtung

Die Einrichtung (ein Fenster mit Fragen) fragt:

1. **Dienst:** *Groq* wählen (oder den, für den Sie einen Schlüssel haben).
2. **API-Schlüssel:** den Schlüssel aus Schritt 1 einfügen. Er wird sofort geprüft und
   verschlüsselt auf Ihrem Computer gespeichert.
3. **Gesprochene Sprache:** z. B. Deutsch, oder *automatisch*, wenn Sie die Sprache wechseln.
4. **Diktiertaste:** die Taste drücken, die Sie beim Sprechen halten möchten.
   Standard: **rechte Strg-Taste**.
5. **Mikrofon** und **Start bei der Anmeldung:** die Vorgaben passen.

Danach läuft Wortfluss im Hintergrund. Sie sehen sein Symbol im Infobereich / in der
Taskleiste (unter Windows evtl. hinter dem kleinen Pfeil ^ neben der Uhr).

**Linux mit GNOME (z. B. Ubuntu):** Wortfluss installiert eine kleine GNOME-Erweiterung für
die Zuhör-Blase und das Einfügen. Wenn eine Meldung darum bittet, **einmal ab- und wieder
anmelden**, damit GNOME sie lädt. Bis dahin funktioniert alles, der Text wird nur getippt
statt eingefügt, und es gibt keine Blase.

## 4. Im Alltag

| Sie möchten … | So geht's |
|---|---|
| **Diktieren** | In ein Textfeld klicken, **rechte Strg-Taste halten**, sprechen, loslassen. Der Text erscheint nach etwa einer Sekunde. |
| Sehen, dass es zuhört | Ein kurzer Piepton und eine kleine Blase mit bewegter Schallwelle neben dem Text-Cursor (in Apps, die ihren Cursor nicht melden: unten am Bildschirm). Graue Punkte: Der Text ist unterwegs. |
| **Markierten Text verbessern** | Text markieren, **Strg+Alt+R**, Ton mit 1–6 wählen (korrigieren, professionell, zwanglos, höflich, Social Media, Emojis), **Enter** ersetzt ihn. In einer anderen Sprache: unter *Übersetzen in* die Sprache wählen und *Übersetzen* drücken. |
| **Vorlesung, Video oder Besprechung mitschreiben** | **Strg+Alt+S** startet und stoppt; Transkript und Notizen landen in *Dokumente/vtt* (Symbol → *Ordner mit Transkripten öffnen*). Andere Personen nur mit deren Einverständnis aufnehmen. |
| Einstellungen ändern | Symbol → *Alle Einstellungen …* (oder **Strg+Alt+V**), Startmenü → *Wortfluss settings*, oder `vtt setup`. |
| Prüfen, ob alles funktioniert | `vtt check` (Windows: in der Eingabeaufforderung) |
| Hilfe | Startmenü → *Wortfluss help*, oder `vtt help` |

Tipps:

- In ganzen Sätzen sprechen; Satzzeichen werden automatisch gesetzt.
- Namen und Fachbegriffe: Symbol → *Alle Einstellungen …* → *Phrases*, oder
  `vtt phrases add "Name, Begriff"`.
- Die Oberfläche folgt der Systemsprache (English, Deutsch, Türkçe, Español, Polski, Italiano).
- Der Text wird **eingefügt** (schnell, jede Sprache und Emojis). Die Zwischenablage wird dafür
  kurz benutzt und danach zurückgesetzt, vorher Kopiertes ist also weiterhin da. (Linux: Ein
  kopiertes Bild wird ersetzt.) Lieber Buchstabe für Buchstabe tippen lassen? *Alle
  Einstellungen …* → *Put text in by pasting* → No.
- Blase ausschalten: *Alle Einstellungen …* → *Listening bubble at the text cursor*.

## 5. Updates

Nichts zu tun: Wortfluss sucht alle paar Stunden nach neuen Versionen und installiert sie im
Hintergrund, wenn Sie gerade nicht diktieren. Es werden nur vom Herausgeber signierte
Versionen angenommen. Danach meldet es *„Wortfluss wurde auf Version X aktualisiert.“*
Sofort prüfen: Symbol → *Nach Updates suchen* (meldet Suche und Download), oder
`vtt update`. Linux mit GNOME: Bittet ein Update ums Ab- und Anmelden, das bei Gelegenheit
tun.

## 6. Deinstallieren

- **Windows:** Einstellungen → Apps → *Wortfluss* → Deinstallieren (oder Startmenü →
  *Uninstall Wortfluss*). Einstellungen und Transkripte bleiben in `%APPDATA%\Wortfluss` und
  *Dokumente\vtt*; bei Bedarf von Hand löschen.
- **Linux:** `vtt stop`, `vtt autostart off`, dann `~/.local/bin/vtt` löschen, und wenn Sie
  möchten `~/.config/vtt` (Einstellungen) und `~/Documents/vtt` (Transkripte).

## Probleme?

1. `vtt check` ausführen: prüft Schlüssel, Mikrofon, Tastaturzugriff und Autostart und sagt,
   was zu tun ist.
2. In einem Programm, das **als Administrator** läuft, erscheint nichts (Windows): Das lässt
   Windows nicht zu; das Programm normal starten.
3. Linux: Der Text wird langsam getippt statt eingefügt, oder es gibt keine Blase: einmal ab-
   und wieder anmelden (dann lädt GNOME die Wortfluss-Erweiterung). Im Terminal erscheint
   `^V`: Bitte melden, welches Terminal-Programm es ist.
4. Eine App zeigt seltsame oder doppelte Buchstaben: prüfen, ob *Put text in by pasting*
   eingeschaltet ist (`vtt set paste`).
5. Weiterhin Probleme: [Problem melden](https://github.com/1a-khan/wortfluss/issues/new/choose)
   (nie den API-Schlüssel posten), oder [MIAK IT](https://miak-it.de) kontaktieren.

---
Wortfluss wird von Ammad Khan, [MIAK IT](https://miak-it.de) entwickelt. Frei und quelloffen
(Apache License 2.0).
