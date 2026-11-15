//! Interface languages: English, German, Turkish, Spanish, Polish, Italian.
//!
//! Covers what people see every day: the tray menu, notifications and the rewrite
//! window. The language follows the system (`ui_language = "auto"`) or the setting
//! (`vtt set ui-language`). Terminal setup and `vtt help` stay English.
//!
//! Texts are looked up by key; `{}` marks where values go, in order.

use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    De,
    Tr,
    Es,
    Pl,
    It,
}

/// Code and name (in the language itself), in table column order.
pub const LANGS: [(Lang, &str, &str); 6] = [
    (Lang::En, "en", "English"),
    (Lang::De, "de", "Deutsch"),
    (Lang::Tr, "tr", "Türkçe"),
    (Lang::Es, "es", "Español"),
    (Lang::Pl, "pl", "Polski"),
    (Lang::It, "it", "Italiano"),
];

static CURRENT: OnceLock<Lang> = OnceLock::new();

pub fn from_code(code: &str) -> Option<Lang> {
    let code = code.trim().to_lowercase();
    LANGS.iter().find(|(_, c, _)| code.starts_with(c)).map(|l| l.0)
}

/// The language of the desktop session, if it is one of ours.
pub fn system() -> Option<Lang> {
    #[cfg(windows)]
    {
        // Primary language id: low 10 bits of the UI language.
        let id = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() } & 0x3ff;
        match id {
            0x09 => Some(Lang::En),
            0x07 => Some(Lang::De),
            0x1f => Some(Lang::Tr),
            0x0a => Some(Lang::Es),
            0x15 => Some(Lang::Pl),
            0x10 => Some(Lang::It),
            _ => None,
        }
    }
    #[cfg(not(windows))]
    {
        // LANGUAGE may list several ("de:en"); the first of ours wins.
        ["LC_ALL", "LC_MESSAGES", "LANGUAGE", "LANG"]
            .iter()
            .filter_map(|v| std::env::var(v).ok().filter(|s| !s.is_empty() && s != "C" && s != "POSIX"))
            .next()
            .and_then(|v| v.split(':').find_map(from_code))
    }
}

/// The interface language of this process: the setting (`ui_language`), else the
/// system's, else English. Read once; a changed setting restarts vtt anyway.
pub fn lang() -> Lang {
    *CURRENT.get_or_init(|| {
        let setting = crate::config::load().map(|c| c.ui_language).unwrap_or_default();
        from_code(&setting).or_else(system).unwrap_or(Lang::En)
    })
}

fn column(l: Lang) -> usize {
    LANGS.iter().position(|x| x.0 == l).unwrap_or(0)
}

/// The text for `key` in the current language (English if missing; the key itself if
/// unknown, which the tests rule out).
pub fn t(key: &str) -> &'static str {
    t_in(lang(), key)
}

pub fn t_in(l: Lang, key: &str) -> &'static str {
    match TEXTS.iter().find(|(k, _)| *k == key) {
        Some((_, v)) => {
            let s = v[column(l)];
            if s.is_empty() { v[0] } else { s }
        }
        None => {
            debug_assert!(false, "missing text {key}");
            ""
        }
    }
}

/// `t(key)` with each `{}` replaced by the next value.
pub fn tf(key: &str, values: &[&str]) -> String {
    fill(t(key), values)
}

fn fill(template: &str, values: &[&str]) -> String {
    let mut out = String::with_capacity(template.len() + 32);
    let mut parts = template.split("{}");
    let mut values = values.iter();
    if let Some(first) = parts.next() {
        out.push_str(first);
    }
    for part in parts {
        out.push_str(values.next().copied().unwrap_or(""));
        out.push_str(part);
    }
    out
}

/// "on"/"off" after a setting's name: "Beeps: on".
pub fn switched(name_key: &str, on: bool) -> String {
    tf("msg.switched", &[t(name_key), t(if on { "word.on" } else { "word.off" })])
}

type Row = (&'static str, [&'static str; 6]);

/// Columns: en, de, tr, es, pl, it.
const TEXTS: &[Row] = &[
    // ---- words
    ("word.on", ["on", "an", "açık", "activado", "wł.", "attivo"]),
    ("word.off", ["off", "aus", "kapalı", "desactivado", "wył.", "disattivo"]),
    ("msg.switched", ["{}: {}", "{}: {}", "{}: {}", "{}: {}", "{}: {}", "{}: {}"]),
    // ---- tray
    ("tray.status.session", [
        "Session running: transcribing",
        "Sitzung läuft: wird transkribiert",
        "Oturum sürüyor: metne dönüştürülüyor",
        "Sesión en curso: transcribiendo",
        "Sesja trwa: transkrypcja",
        "Sessione in corso: trascrizione",
    ]),
    ("tray.status.listening", ["Listening...", "Hört zu …", "Dinliyor…", "Escuchando…", "Słucham…", "In ascolto…"]),
    ("tray.status.hold", [
        "Ready: hold {} to dictate",
        "Bereit: {} gedrückt halten zum Diktieren",
        "Hazır: dikte etmek için {} tuşunu basılı tutun",
        "Listo: mantén {} pulsada para dictar",
        "Gotowe: przytrzymaj {}, aby dyktować",
        "Pronto: tieni premuto {} per dettare",
    ]),
    ("tray.status.press", [
        "Ready: press {} to dictate",
        "Bereit: {} drücken zum Diktieren",
        "Hazır: dikte etmek için {} tuşuna basın",
        "Listo: pulsa {} para dictar",
        "Gotowe: naciśnij {}, aby dyktować",
        "Pronto: premi {} per dettare",
    ]),
    ("tray.stop_session", [
        "Stop session and write notes",
        "Sitzung beenden und Notizen schreiben",
        "Oturumu bitir ve notları yaz",
        "Detener la sesión y redactar notas",
        "Zakończ sesję i utwórz notatki",
        "Termina la sessione e scrivi le note",
    ]),
    ("tray.session_system", [
        "Transcribe what the computer plays",
        "Transkribieren, was der Computer abspielt",
        "Bilgisayarda çalanı metne dönüştür",
        "Transcribir lo que reproduce el ordenador",
        "Transkrybuj dźwięk z komputera",
        "Trascrivi ciò che riproduce il computer",
    ]),
    ("tray.session_mic", [
        "Transcribe the microphone (room)",
        "Mikrofon transkribieren (Raum)",
        "Mikrofonu metne dönüştür (oda)",
        "Transcribir el micrófono (sala)",
        "Transkrybuj mikrofon (pomieszczenie)",
        "Trascrivi il microfono (stanza)",
    ]),
    ("tray.session_call", [
        "Transcribe a call (You / Others, headphones)",
        "Anruf transkribieren (Du / Andere, Kopfhörer)",
        "Görüşmeyi metne dönüştür (Siz / Diğerleri, kulaklık)",
        "Transcribir una llamada (Tú / Otros, auriculares)",
        "Transkrybuj rozmowę (Ty / Inni, słuchawki)",
        "Trascrivi una chiamata (Tu / Altri, cuffie)",
    ]),
    ("tray.open", [
        "Open transcripts folder",
        "Ordner mit Transkripten öffnen",
        "Metin dökümleri klasörünü aç",
        "Abrir la carpeta de transcripciones",
        "Otwórz folder z transkrypcjami",
        "Apri la cartella delle trascrizioni",
    ]),
    ("tray.rewrite", [
        "Rewrite selected text...",
        "Markierten Text umschreiben …",
        "Seçili metni yeniden yaz…",
        "Reescribir el texto seleccionado…",
        "Przeredaguj zaznaczony tekst…",
        "Riscrivi il testo selezionato…",
    ]),
    ("tray.correction", [
        "Spelling & grammar correction",
        "Rechtschreib- und Grammatikkorrektur",
        "Yazım ve dil bilgisi düzeltmesi",
        "Corrección ortográfica y gramatical",
        "Korekta pisowni i gramatyki",
        "Correzione ortografica e grammaticale",
    ]),
    ("tray.mode", [
        "Dictation key mode",
        "Diktiertaste",
        "Dikte tuşu modu",
        "Modo de la tecla de dictado",
        "Tryb klawisza dyktowania",
        "Modalità del tasto di dettatura",
    ]),
    ("tray.mode_hold", [
        "Hold {} while speaking",
        "{} beim Sprechen gedrückt halten",
        "Konuşurken {} tuşunu basılı tutun",
        "Mantén {} pulsada mientras hablas",
        "Przytrzymaj {} podczas mówienia",
        "Tieni premuto {} mentre parli",
    ]),
    ("tray.mode_toggle", [
        "Press {} to start, again to stop",
        "{} drücken zum Starten, erneut zum Beenden",
        "Başlatmak için {} tuşuna basın, durdurmak için tekrar basın",
        "Pulsa {} para empezar y otra vez para parar",
        "Naciśnij {}, aby zacząć, i ponownie, aby zakończyć",
        "Premi {} per iniziare e di nuovo per fermare",
    ]),
    ("tray.mic", ["Microphone", "Mikrofon", "Mikrofon", "Micrófono", "Mikrofon", "Microfono"]),
    ("tray.mic_default", [
        "System default",
        "Systemstandard",
        "Sistem varsayılanı",
        "Predeterminado del sistema",
        "Domyślny systemowy",
        "Predefinito di sistema",
    ]),
    ("tray.notes", [
        "Notes after a session",
        "Notizen nach einer Sitzung",
        "Oturumdan sonra notlar",
        "Notas tras una sesión",
        "Notatki po sesji",
        "Note dopo una sessione",
    ]),
    ("notes.auto", ["Automatic", "Automatisch", "Otomatik", "Automático", "Automatycznie", "Automatico"]),
    ("notes.lecture", ["Lecture notes", "Vorlesungsnotizen", "Ders notları", "Apuntes de clase", "Notatki z wykładu", "Appunti della lezione"]),
    ("notes.meeting", ["Meeting minutes", "Besprechungsprotokoll", "Toplantı tutanağı", "Acta de reunión", "Protokół ze spotkania", "Verbale della riunione"]),
    ("notes.summary", ["Summary", "Zusammenfassung", "Özet", "Resumen", "Podsumowanie", "Riepilogo"]),
    ("notes.none", ["None", "Keine", "Yok", "Ninguna", "Brak", "Nessuna"]),
    ("tray.speakers", [
        "Separate speakers (Speaker 1, 2 ...)",
        "Sprecher trennen (Sprecher 1, 2 …)",
        "Konuşmacıları ayır (Konuşmacı 1, 2 …)",
        "Separar hablantes (Hablante 1, 2 …)",
        "Rozdziel mówców (Mówca 1, 2 …)",
        "Separa i parlanti (Parlante 1, 2 …)",
    ]),
    ("tray.keep_audio", [
        "Keep session audio (asks to delete)",
        "Sitzungsaudio behalten (fragt vor dem Löschen)",
        "Oturum sesini sakla (silmeden önce sorar)",
        "Conservar el audio de la sesión (pregunta antes de borrar)",
        "Zachowaj nagranie sesji (pyta o usunięcie)",
        "Conserva l'audio della sessione (chiede prima di eliminare)",
    ]),
    ("tray.beeps", ["Beeps", "Signaltöne", "Bip sesleri", "Pitidos", "Sygnały dźwiękowe", "Segnali acustici"]),
    ("tray.settings", ["All settings...", "Alle Einstellungen …", "Tüm ayarlar…", "Todos los ajustes…", "Wszystkie ustawienia…", "Tutte le impostazioni…"]),
    ("tray.updates", [
        "Check for updates",
        "Nach Updates suchen",
        "Güncellemeleri denetle",
        "Buscar actualizaciones",
        "Sprawdź aktualizacje",
        "Cerca aggiornamenti",
    ]),
    ("tray.about", ["About Wortfluss", "Über Wortfluss", "Wortfluss hakkında", "Acerca de Wortfluss", "O Wortfluss", "Informazioni su Wortfluss"]),
    ("tray.made_by", ["Made by {}", "Entwickelt von {}", "Geliştiren: {}", "Creado por {}", "Autor: {}", "Creato da {}"]),
    ("tray.website", ["Website", "Website", "Web sitesi", "Sitio web", "Strona internetowa", "Sito web"]),
    ("tray.github", ["GitHub profile", "GitHub-Profil", "GitHub profili", "Perfil de GitHub", "Profil GitHub", "Profilo GitHub"]),
    ("tray.source", [
        "Source code (Apache 2.0, free to use)",
        "Quellcode (Apache 2.0, frei nutzbar)",
        "Kaynak kodu (Apache 2.0, ücretsiz kullanım)",
        "Código fuente (Apache 2.0, uso libre)",
        "Kod źródłowy (Apache 2.0, do swobodnego użytku)",
        "Codice sorgente (Apache 2.0, uso libero)",
    ]),
    ("tray.quit", ["Quit Wortfluss", "Wortfluss beenden", "Wortfluss'tan çık", "Salir de Wortfluss", "Zamknij Wortfluss", "Esci da Wortfluss"]),
    // ---- setting changes (shown as "<name>: on")
    ("set.speakers", [
        "Separate speakers in sessions",
        "Sprecher in Sitzungen trennen",
        "Oturumlarda konuşmacıları ayır",
        "Separar hablantes en las sesiones",
        "Rozdzielanie mówców w sesjach",
        "Separa i parlanti nelle sessioni",
    ]),
    ("set.keep_audio", [
        "Keep session audio",
        "Sitzungsaudio behalten",
        "Oturum sesini sakla",
        "Conservar el audio de la sesión",
        "Zachowywanie nagrania sesji",
        "Conserva l'audio della sessione",
    ]),
    ("set.no_speakers", [
        "{} can't separate speakers. Switch to OpenRouter, OpenAI or Mistral",
        "{} kann Sprecher nicht trennen. Wechsle zu OpenRouter, OpenAI oder Mistral",
        "{} konuşmacıları ayıramaz. OpenRouter, OpenAI veya Mistral'a geçin",
        "{} no puede separar hablantes. Cambia a OpenRouter, OpenAI o Mistral",
        "{} nie potrafi rozdzielać mówców. Przełącz na OpenRouter, OpenAI lub Mistral",
        "{} non può separare i parlanti. Passa a OpenRouter, OpenAI o Mistral",
    ]),
    // ---- messages and errors (technical details after the colon stay English)
    ("err.not_set_up", [
        "Wortfluss is not set up ({}). Open a terminal and run: vtt setup",
        "Wortfluss ist nicht eingerichtet ({}). Öffne ein Terminal und führe aus: vtt setup",
        "Wortfluss kurulmamış ({}). Bir terminal açıp şunu çalıştırın: vtt setup",
        "Wortfluss no está configurado ({}). Abre una terminal y ejecuta: vtt setup",
        "Wortfluss nie jest skonfigurowany ({}). Otwórz terminal i uruchom: vtt setup",
        "Wortfluss non è configurato ({}). Apri un terminale ed esegui: vtt setup",
    ]),
    ("err.typing", [
        "Typing failed: {}",
        "Tippen fehlgeschlagen: {}",
        "Yazma başarısız: {}",
        "Error al escribir: {}",
        "Wpisywanie nie powiodło się: {}",
        "Digitazione non riuscita: {}",
    ]),
    ("err.typing_crash", [
        "Typing failed unexpectedly; the text was not typed.",
        "Tippen unerwartet fehlgeschlagen; der Text wurde nicht getippt.",
        "Yazma beklenmedik şekilde başarısız oldu; metin yazılmadı.",
        "La escritura falló inesperadamente; el texto no se escribió.",
        "Wpisywanie nieoczekiwanie się nie powiodło; tekst nie został wpisany.",
        "La digitazione non è riuscita; il testo non è stato digitato.",
    ]),
    ("err.piece_crash", [
        "A piece of the recording failed unexpectedly and was skipped.",
        "Ein Teil der Aufnahme ist unerwartet fehlgeschlagen und wurde übersprungen.",
        "Kaydın bir bölümü beklenmedik şekilde başarısız oldu ve atlandı.",
        "Una parte de la grabación falló inesperadamente y se omitió.",
        "Fragment nagrania nieoczekiwanie się nie powiódł i został pominięty.",
        "Una parte della registrazione non è riuscita ed è stata saltata.",
    ]),
    ("err.rewrite", ["Rewrite: {}", "Umschreiben: {}", "Yeniden yazma: {}", "Reescribir: {}", "Redagowanie: {}", "Riscrittura: {}"]),
    ("err.settings_not_applied", [
        "New settings not applied: {}",
        "Neue Einstellungen nicht übernommen: {}",
        "Yeni ayarlar uygulanmadı: {}",
        "No se aplicaron los nuevos ajustes: {}",
        "Nowe ustawienia nie zostały zastosowane: {}",
        "Nuove impostazioni non applicate: {}",
    ]),
    ("err.session", ["Session: {}", "Sitzung: {}", "Oturum: {}", "Sesión: {}", "Sesja: {}", "Sessione: {}"]),
    ("err.setting_not_changed", [
        "Setting not changed: {}",
        "Einstellung nicht geändert: {}",
        "Ayar değiştirilmedi: {}",
        "Ajuste no modificado: {}",
        "Ustawienie nie zostało zmienione: {}",
        "Impostazione non modificata: {}",
    ]),
    ("ind.relogin", [
        "Log out and back in once: then the listening bubble appears next to the text cursor and text is pasted instead of typed.",
        "Einmal ab- und wieder anmelden: Dann erscheint die Zuhör-Anzeige neben dem Textcursor, und Text wird eingefügt statt getippt.",
        "Bir kez oturumu kapatıp yeniden açın: Ardından dinleme göstergesi metin imlecinin yanında görünür ve metin yazılmak yerine yapıştırılır.",
        "Cierra la sesión y vuelve a entrar una vez: entonces el indicador de escucha aparece junto al cursor y el texto se pega en lugar de escribirse.",
        "Wyloguj się i zaloguj ponownie: wtedy wskaźnik nasłuchiwania pojawi się obok kursora tekstu, a tekst będzie wklejany zamiast pisany.",
        "Esci e rientra una volta: poi l'indicatore di ascolto appare accanto al cursore e il testo viene incollato invece che digitato.",
    ]),
    ("msg.no_speech", [
        "Recording stopped: no speech for {} seconds.",
        "Aufnahme beendet: {} Sekunden lang nichts gesagt.",
        "Kayıt durduruldu: {} saniyedir konuşma yok.",
        "Grabación detenida: {} segundos sin voz.",
        "Nagrywanie zatrzymane: brak mowy przez {} s.",
        "Registrazione interrotta: nessun parlato per {} secondi.",
    ]),
    ("msg.limit", [
        "Recording stopped at the {} limit (change it with `vtt set mode`).",
        "Aufnahme beim Limit von {} beendet (ändern mit `vtt set mode`).",
        "Kayıt {} sınırında durduruldu (`vtt set mode` ile değiştirin).",
        "Grabación detenida al llegar al límite de {} (cámbialo con `vtt set mode`).",
        "Nagrywanie zatrzymane po osiągnięciu limitu {} (zmień przez `vtt set mode`).",
        "Registrazione interrotta al limite di {} (modificalo con `vtt set mode`).",
    ]),
    ("msg.audio_saved", [
        "{}\nThe audio was saved ({}); `vtt retry` transcribes it later.",
        "{}\nDas Audio wurde gespeichert ({}); `vtt retry` transkribiert es später.",
        "{}\nSes kaydedildi ({}); `vtt retry` daha sonra metne dönüştürür.",
        "{}\nEl audio se guardó ({}); `vtt retry` lo transcribe más tarde.",
        "{}\nNagranie zapisano ({}); `vtt retry` przepisze je później.",
        "{}\nL'audio è stato salvato ({}); `vtt retry` lo trascrive più tardi.",
    ]),
    ("msg.audio_not_saved", [
        "{} (could not save the audio: {})",
        "{} (Audio konnte nicht gespeichert werden: {})",
        "{} (ses kaydedilemedi: {})",
        "{} (no se pudo guardar el audio: {})",
        "{} (nie udało się zapisać nagrania: {})",
        "{} (impossibile salvare l'audio: {})",
    ]),
    ("sup.gave_up", [
        "{} stopped after {} crashes within {} s. See {}",
        "{} wurde nach {} Abstürzen in {} s beendet. Siehe {}",
        "{} durduruldu: {} çökme, {} saniye içinde. Bkz. {}",
        "{} se detuvo tras {} fallos en {} s. Consulta {}",
        "{} zatrzymano po {} awariach w ciągu {} s. Zobacz {}",
        "{} è stato fermato dopo {} arresti anomali in {} s. Vedi {}",
    ]),
    ("sup.restarting", [
        "{} stopped unexpectedly and is being restarted.",
        "{} wurde unerwartet beendet und wird neu gestartet.",
        "{} beklenmedik şekilde durdu ve yeniden başlatılıyor.",
        "{} se detuvo inesperadamente y se está reiniciando.",
        "{} nieoczekiwanie się zatrzymał i jest uruchamiany ponownie.",
        "{} si è arrestato inaspettatamente e viene riavviato.",
    ]),
    // ---- sessions
    ("session.title", ["Wortfluss session", "Wortfluss-Sitzung", "Wortfluss oturumu", "Sesión de Wortfluss", "Sesja Wortfluss", "Sessione Wortfluss"]),
    ("source.system", [
        "system audio (what the computer plays)",
        "Systemaudio (was der Computer abspielt)",
        "sistem sesi (bilgisayarda çalan)",
        "audio del sistema (lo que reproduce el ordenador)",
        "dźwięk systemowy (to, co odtwarza komputer)",
        "audio di sistema (ciò che riproduce il computer)",
    ]),
    ("source.mic", ["microphone", "Mikrofon", "mikrofon", "micrófono", "mikrofon", "microfono"]),
    ("source.both", [
        "system audio + microphone",
        "Systemaudio + Mikrofon",
        "sistem sesi + mikrofon",
        "audio del sistema + micrófono",
        "dźwięk systemowy + mikrofon",
        "audio di sistema + microfono",
    ]),
    ("session.started", [
        "Transcribing {}. Only record other people with their consent.{}\nStop: vtt session stop",
        "Transkribiere {}. Nimm andere Personen nur mit ihrer Zustimmung auf.{}\nBeenden: vtt session stop",
        "Metne dönüştürülüyor: {}. Başkalarını yalnızca izinleriyle kaydedin.{}\nDurdur: vtt session stop",
        "Transcribiendo {}. Graba a otras personas solo con su consentimiento.{}\nDetener: vtt session stop",
        "Transkrypcja: {}. Nagrywaj inne osoby tylko za ich zgodą.{}\nZatrzymaj: vtt session stop",
        "Trascrizione di {}. Registra altre persone solo con il loro consenso.{}\nFerma: vtt session stop",
    ]),
    ("session.headphones", [
        "\nUse headphones: with loudspeakers the other side may be transcribed twice.",
        "\nKopfhörer benutzen: Mit Lautsprechern wird die Gegenseite evtl. doppelt transkribiert.",
        "\nKulaklık kullanın: hoparlörle karşı taraf iki kez metne dönüştürülebilir.",
        "\nUsa auriculares: con altavoces, la otra parte puede transcribirse dos veces.",
        "\nUżyj słuchawek: przy głośnikach druga strona może zostać zapisana podwójnie.",
        "\nUsa le cuffie: con gli altoparlanti l'altra parte può essere trascritta due volte.",
    ]),
    ("session.no_speakers", [
        "{} can't separate speakers; this session is transcribed without. Use OpenRouter, OpenAI or Mistral for that (`vtt set service`).",
        "{} kann Sprecher nicht trennen; diese Sitzung wird ohne transkribiert. Dafür OpenRouter, OpenAI oder Mistral nutzen (`vtt set service`).",
        "{} konuşmacıları ayıramaz; bu oturum ayırmadan metne dönüştürülür. Bunun için OpenRouter, OpenAI veya Mistral kullanın (`vtt set service`).",
        "{} no puede separar hablantes; esta sesión se transcribe sin ello. Usa OpenRouter, OpenAI o Mistral para eso (`vtt set service`).",
        "{} nie potrafi rozdzielać mówców; ta sesja zostanie zapisana bez tego. Użyj do tego OpenRouter, OpenAI lub Mistral (`vtt set service`).",
        "{} non può separare i parlanti; questa sessione viene trascritta senza. Usa OpenRouter, OpenAI o Mistral (`vtt set service`).",
    ]),
    ("session.stopped_quiet", [
        "Stopped after 30 minutes without speech. ",
        "Nach 30 Minuten ohne Sprache beendet. ",
        "30 dakika konuşma olmadığı için durduruldu. ",
        "Detenida tras 30 minutos sin voz. ",
        "Zatrzymano po 30 minutach bez mowy. ",
        "Interrotta dopo 30 minuti senza parlato. ",
    ]),
    ("session.saved", [
        "{}Saved ({}):\n{}",
        "{}Gespeichert ({}):\n{}",
        "{}Kaydedildi ({}):\n{}",
        "{}Guardado ({}):\n{}",
        "{}Zapisano ({}):\n{}",
        "{}Salvato ({}):\n{}",
    ]),
    ("session.delete_title", [
        "Wortfluss: delete the session audio?",
        "Wortfluss: Sitzungsaudio löschen?",
        "Wortfluss: oturum sesi silinsin mi?",
        "Wortfluss: ¿borrar el audio de la sesión?",
        "Wortfluss: usunąć nagranie sesji?",
        "Wortfluss: eliminare l'audio della sessione?",
    ]),
    ("session.delete_now", [
        "Delete the audio now?",
        "Audio jetzt löschen?",
        "Ses şimdi silinsin mi?",
        "¿Borrar el audio ahora?",
        "Usunąć nagranie teraz?",
        "Eliminare l'audio ora?",
    ]),
    ("session.delete", ["Delete audio", "Audio löschen", "Sesi sil", "Borrar audio", "Usuń nagranie", "Elimina audio"]),
    ("session.keep", ["Keep for now", "Vorerst behalten", "Şimdilik sakla", "Conservar por ahora", "Zachowaj na razie", "Conserva per ora"]),
    // ---- rewrite window
    ("rw.title", [
        "Wortfluss – Rewrite text",
        "Wortfluss – Text umschreiben",
        "Wortfluss – Metni yeniden yaz",
        "Wortfluss – Reescribir texto",
        "Wortfluss – Przeredaguj tekst",
        "Wortfluss – Riscrivi il testo",
    ]),
    ("rw.selected", ["Selected text", "Markierter Text", "Seçili metin", "Texto seleccionado", "Zaznaczony tekst", "Testo selezionato"]),
    ("rw.suggestion", ["Suggestion", "Vorschlag", "Öneri", "Sugerencia", "Propozycja", "Suggerimento"]),
    ("rw.working", ["Rewriting …", "Wird umgeschrieben …", "Yeniden yazılıyor…", "Reescribiendo…", "Redaguję…", "Riscrittura in corso…"]),
    ("rw.replace", ["Replace", "Ersetzen", "Değiştir", "Reemplazar", "Zastąp", "Sostituisci"]),
    ("rw.copy", ["Copy", "Kopieren", "Kopyala", "Copiar", "Kopiuj", "Copia"]),
    ("rw.copied", ["Copied ✔", "Kopiert ✔", "Kopyalandı ✔", "Copiado ✔", "Skopiowano ✔", "Copiato ✔"]),
    ("rw.retry", ["Try again", "Erneut versuchen", "Tekrar dene", "Reintentar", "Spróbuj ponownie", "Riprova"]),
    ("rw.not_back", [
        "The app with the selection is no longer in front. The new text is in the clipboard: paste it with Ctrl+V.",
        "Die App mit der Markierung ist nicht mehr im Vordergrund. Der neue Text ist in der Zwischenablage: mit Strg+V einfügen.",
        "Seçimin bulunduğu uygulama artık önde değil. Yeni metin panoda: Ctrl+V ile yapıştırın.",
        "La aplicación con la selección ya no está en primer plano. El texto nuevo está en el portapapeles: pégalo con Ctrl+V.",
        "Aplikacja z zaznaczeniem nie jest już na pierwszym planie. Nowy tekst jest w schowku: wklej go Ctrl+V.",
        "L'app con la selezione non è più in primo piano. Il nuovo testo è negli appunti: incollalo con Ctrl+V.",
    ]),
    ("rw.cancel", ["Cancel", "Abbrechen", "İptal", "Cancelar", "Anuluj", "Annulla"]),
    ("rw.nothing", [
        "No text selected. Select text in an app and press the shortcut again.",
        "Kein Text markiert. Markiere Text in einer App und drücke das Tastenkürzel erneut.",
        "Seçili metin yok. Bir uygulamada metin seçip kısayola tekrar basın.",
        "No hay texto seleccionado. Selecciona texto en una aplicación y vuelve a pulsar el atajo.",
        "Nie zaznaczono tekstu. Zaznacz tekst w aplikacji i ponownie naciśnij skrót.",
        "Nessun testo selezionato. Seleziona del testo in un'app e premi di nuovo la scorciatoia.",
    ]),
    ("rw.waiting", [
        "Reading the selected text …",
        "Markierten Text lesen …",
        "Seçili metin okunuyor…",
        "Leyendo el texto seleccionado…",
        "Odczytuję zaznaczony tekst…",
        "Lettura del testo selezionato…",
    ]),
    ("rw.keys", [
        "Enter: replace · 1–6: tone · Esc: cancel",
        "Enter: ersetzen · 1–6: Stil · Esc: abbrechen",
        "Enter: değiştir · 1–6: üslup · Esc: iptal",
        "Intro: reemplazar · 1–6: tono · Esc: cancelar",
        "Enter: zastąp · 1–6: styl · Esc: anuluj",
        "Invio: sostituisci · 1–6: tono · Esc: annulla",
    ]),
    ("tone.correct", ["Correct", "Korrigieren", "Düzelt", "Corregir", "Popraw", "Correggi"]),
    ("tone.professional", ["Professional", "Professionell", "Profesyonel", "Profesional", "Profesjonalnie", "Professionale"]),
    ("tone.casual", ["Casual", "Zwanglos", "Samimi", "Informal", "Swobodnie", "Informale"]),
    ("tone.polite", ["Polite", "Höflich", "Kibar", "Cortés", "Uprzejmie", "Cortese"]),
    ("tone.social", ["#social", "#social", "#social", "#social", "#social", "#social"]),
    ("tone.emojis", ["Add emojis", "Emojis hinzufügen", "Emoji ekle", "Añadir emojis", "Dodaj emoji", "Aggiungi emoji"]),
    // ---- updates
    ("upd.done", [
        "Wortfluss was updated to version {}.",
        "Wortfluss wurde auf Version {} aktualisiert.",
        "Wortfluss {} sürümüne güncellendi.",
        "Wortfluss se actualizó a la versión {}.",
        "Wortfluss zaktualizowano do wersji {}.",
        "Wortfluss è stato aggiornato alla versione {}.",
    ]),
    ("upd.checking", [
        "Searching for updates …",
        "Suche nach Updates …",
        "Güncellemeler aranıyor…",
        "Buscando actualizaciones…",
        "Szukam aktualizacji…",
        "Ricerca aggiornamenti…",
    ]),
    ("upd.downloading", [
        "Version {} found. Downloading … Wortfluss restarts by itself when it is installed.",
        "Version {} gefunden. Wird heruntergeladen … Wortfluss startet nach der Installation von selbst neu.",
        "{} sürümü bulundu. İndiriliyor… Kurulumdan sonra Wortfluss kendiliğinden yeniden başlar.",
        "Versión {} encontrada. Descargando… Wortfluss se reinicia solo al instalarla.",
        "Znaleziono wersję {}. Pobieranie… Po instalacji Wortfluss uruchomi się ponownie.",
        "Trovata la versione {}. Download in corso… Wortfluss si riavvia da solo dopo l'installazione.",
    ]),
    ("upd.none", [
        "Wortfluss {} is up to date.",
        "Wortfluss {} ist aktuell.",
        "Wortfluss {} güncel.",
        "Wortfluss {} está actualizado.",
        "Wortfluss {} jest aktualny.",
        "Wortfluss {} è aggiornato.",
    ]),
    ("upd.failed", [
        "Update failed: {}",
        "Update fehlgeschlagen: {}",
        "Güncelleme başarısız: {}",
        "Error de actualización: {}",
        "Aktualizacja nie powiodła się: {}",
        "Aggiornamento non riuscito: {}",
    ]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_text_is_translated_with_the_same_placeholders() {
        let mut keys = std::collections::HashSet::new();
        for (key, v) in TEXTS {
            assert!(keys.insert(*key), "duplicate key {key}");
            let n = v[0].matches("{}").count();
            for (i, s) in v.iter().enumerate() {
                assert!(!s.is_empty(), "{key}: {} missing", LANGS[i].1);
                assert_eq!(s.matches("{}").count(), n, "{key}: {} has other placeholders", LANGS[i].1);
            }
        }
    }

    #[test]
    fn lookup_and_fill() {
        assert_eq!(t_in(Lang::De, "rw.replace"), "Ersetzen");
        assert_eq!(t_in(Lang::Tr, "tone.polite"), "Kibar");
        assert_eq!(fill("{}: {}", &["Beeps", "on"]), "Beeps: on");
        assert_eq!(from_code("de_DE.UTF-8"), Some(Lang::De));
        assert_eq!(from_code("pl"), Some(Lang::Pl));
        assert_eq!(from_code("auto"), None);
    }
}
