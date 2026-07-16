# GeoRuggine (Progetto 2.1)

**GeoRuggine** è un sistema distribuito client/server in Rust progettato per la geolocalizzazione e la comunicazione bidirezionale con una flotta di veicoli emulati.

---

## Struttura del codice

Cargo Workspace composto da tre crate distinti:

1. **`common`**: Libreria condivisa che contiene i modelli dati (`Coordinate`, `UserStatus`), i messaggi (`ClientMessage`, `ServerMessage`), il parser e l'algoritmo per il calcolo delle distanze geodesiche (Haversine).
2. **`server`**: La logica centrale che traccia lo stato della flotta, gestisce la persistenza, calcola le metriche dei tragitti dei veicoli in tempo reale e monitora il proprio carico CPU.
3. **`client`**: L'emulatore del veicolo che simula gli spostamenti (tramite file, random walk o inserimento manuale) e consente l'invio e la ricezione di messaggi.

---

## Dettagli Implementativi

- **Framing dei Messaggi TCP**: Ogni messaggio è serializzato come stringa JSON su una singola riga terminata dal carattere `\n` (NDJSON consente una lettura semplice tramite `BufReader::read_line`).
- **Architettura a Canali**: La gestione dello stato globale e le operazioni di I/O di rete sono disaccoppiate.
  - Al login, viene creata una coda di messaggi thread-safe (`std::sync::mpsc::channel`) ed associata alla sessione utente.
  - Viene avviato un thread dedicato per ciascun client connesso.
  - Quando il server deve trasmettere messaggi, l'invio avviene inserendo il messaggio nel canale in tempo costante (non-blocking).
  - La scrittura su socket avviene asincronamente sul thread al di fuori del lock del server, impedendo a client lenti o bloccati di congelare il Mutex globale del server.
- **Prevenzione da poisoning del Mutex**: Per evitare che il thread panic di un client causi il crash a catena del server, le acquisizioni del lock utilizzano `.lock().unwrap_or_else(|e| e.into_inner())` (recupero guardiano del mutex).
- **Sicurezza credenziali**: Viene implementato l'algoritmo **SHA-256** (tramite la libreria `sha2`). I record sono salvati nel file locale `users.txt` come hash esadecimale.
- **Macchina a Stati del Veicolo**:
  - **Sconnesso**: Il client non è loggato.
  - **Fermo (Stationary)**: Il client è loggato ma non cambia posizione. Stato iniziale al login.
  - **In Movimento (Moving)**: Transita non appena viene rilevata una variazione di coordinate.
  - **Transizione da Moving a Stationary**: Avviene automaticamente se la posizione del veicolo non subisce modifiche per almeno 3 minuti.
- **Coerenza Spazio-Temporale**: Nel calcolo dei percorsi, la distanza e i tempi del movimento/sosta vengono calcolati solo quando il delta temporale tra due pacchetti è inferiore a 60 secondi, evitando che disconnessioni prolungate falsino il calcolo della velocità media.
- **Persistenza**:
  - La cronologia dei percorsi di ciascun utente viene salvata in modalità _append-only_ in file JSONL sotto la cartella `trajectories/{username}.jsonl`.

---

## Benchmark

Nel file `Cargo.toml` principale è stato configurato un profilo di compilazione release ottimizzato per ridurre l'occupazione di memoria e la dimensione degli eseguibili:

```toml
[profile.release]
opt-level = "z"      # Ottimizza per dimensione
lto = true           # Link-Time Optimization globale
codegen-units = 1    # Generazione codice a singola unità per massimizzare le ottimizzazioni
panic = "abort"      # Rimuove le tabelle per l'unwinding dello stack
strip = true         # Rimuove tutti i simboli di debug dagli eseguibili finali
```

### Dimensioni dei Binari Compilati (su Windows 11 x64):

- **`client.exe`**: **218 KB** (223.744 byte)
- **`server.exe`**: **346 KB** (354.304 byte)

### Consumo Risorse:

Il server genera automaticamente un file `server_cpu.log` riportando ogni 2 minuti le metriche di consumo di tempo CPU del processo ricavate tramite la libreria cross-platform `cpu-time`:

```
[2026-07-16T16:35:20.667501500+00:00] Log Interval: 2 min | Wall Time Delta: 120.04s | CPU Time Delta: 0.2656s | Avg CPU Load: 0.22%
```

_Il carico CPU medio in stato di riposo con un client attivo è dello **0.22%**._

---

## Istruzioni per l'Uso

### Prerequisiti

- Toolchain Rust (installabile tramite [rustup](https://rustup.rs/)).

### Compilazione

Per compilare il progetto in modalità release ottimizzata, posizionarsi nella cartella principale ed eseguire:

```bash
cargo build --release
```

I file eseguibili saranno generati nella directory `target/release/`.

Per avviare gli unit test:

```bash
cargo test --workspace
```

---

## Utilizzo del Server

Avviare il server digitando:

```bash
cargo run --bin server
```

Il server si metterà in ascolto sulla porta locale `127.0.0.1:8080`.

### CLI Server

Una volta avviato, è possibile digitare i seguenti comandi direttamente nel terminale del server:

- `list`: Mostra l'elenco degli utenti registrati, il loro stato corrente e l'ultima coordinata ricevuta.
- `send <username> <messaggio>`: Invia un messaggio di testo privato (DM) a un veicolo specifico.
- `broadcast <messaggio>`: Invia un messaggio di testo a tutti i veicoli attualmente online.
- `analyze <username> <intervallo>`: Esegue l'analisi del percorso del veicolo specificato per l'intervallo richiesto.
  - Gli intervalli supportati sono: `day` (giorno corrente), `week` (settimana corrente), `month` (mese corrente), `all` (intera cronologia).
  - L'analisi calcola: punti totali del tragitto, distanza coperta (km), velocità media (km/h), tempo totale in movimento e tempo trascorso in pausa.
- `exit`: Arresta in modo pulito il server interrompendo il monitoraggio e chiudendo le connessioni attive.

---

## Utilizzo del Client

Per avviare un'istanza del client basta digitare:

```bash
cargo run --bin client
```

### Flusso di Esecuzione

1. **Registrazione / Login**: Al primo avvio viene richiesto se registrarsi (opzione 1) o loggarsi (opzione 2). Digitare username e password.
2. **Selezione Strategia di Movimento**:
   - **Opzione 1 (File-based)**: Riproduce una sequenza di coordinate da un file di testo. Digitare il percorso del file. Ciascuna coordinata verrà trasmessa ogni 30 secondi.
   - **Opzione 2 (Random Walk)**: Genera automaticamente spostamenti casuali partendo da Torino.
   - **Opzione 3 (Manuale)**: Chiede all'utente di digitare manualmente nel terminale la coordinata corrente ad ogni intervallo.
3. **Comunicazione**: Una volta online, è possibile scrivere messaggi di testo nel terminale del client e premere invio per trasmetterli al server. Digitare `exit` o `quit` per disconnettersi.

### Formato dei File di Coordinate (per Opzione 1)

Il client supporta la lettura di file di coordinate strutturati in righe. Il parser tollera l'uso sia della virgola che del punto come separatore decimale:

```text
00:00 45,0618513 7,6606506
00:30 45,0575226 7,6618322
01:00 45.0531939 7.6630137
```

_(Nota: la prima colonna con il timestamp temporale viene ignorata per consentire una trasmissione costante ogni 30 secondi)._
