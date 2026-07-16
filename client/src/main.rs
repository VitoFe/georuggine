use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use chrono::Utc;
use common::{ClientMessage, Coordinate, ServerMessage};

mod emulator;

use emulator::{MovementStrategy, FileStrategy, RandomWalkStrategy, ManualStrategy};

fn main() {
    println!("=== GeoRuggine Client Starting ===");

    let mut stream = match TcpStream::connect("127.0.0.1:8080") {
        Ok(s) => {
            println!("[Client] Connected to server at 127.0.0.1:8080");
            s
        }
        Err(e) => {
            eprintln!("[Client] Could not connect to server: {:?}", e);
            return;
        }
    };

    if let Err(e) = stream.set_read_timeout(Some(Duration::from_secs(60))) {
        eprintln!("[Client] Failed to set read timeout: {:?}", e);
    }

    // Buffer reader for receiving messages
    let mut reader = BufReader::new(stream.try_clone().expect("Failed to clone socket"));

    // auth flow
    let (username, logged_in) = run_auth_flow(&mut stream, &mut reader);
    if !logged_in {
        println!("[Client] Exiting due to authentication failure or cancellation.");
        return;
    }

    let mut strategy = select_movement_strategy();

    // background thread to listen for server messages
    let running = Arc::new(AtomicBool::new(true));
    let running_recv = running.clone();
    let username_clone = username.clone();
    
    // server reader thread
    thread::spawn(move || {
        let mut line = String::new();
        while running_recv.load(Ordering::Relaxed) {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    println!("\n[System] Server closed connection.");
                    running_recv.store(false, Ordering::Relaxed);
                    break;
                }
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Ok(server_msg) = serde_json::from_str::<ServerMessage>(trimmed) {
                        match server_msg {
                            ServerMessage::TextMessage { sender, text } => {
                                println!("\n[Msg from {}]: {}", sender, text);
                                print!("{}> ", username_clone);
                                let _ = std::io::stdout().flush();
                            }
                            ServerMessage::CommandResponse { .. } => {
                                // we print responses silently or in logs
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    if e.kind() != std::io::ErrorKind::WouldBlock && e.kind() != std::io::ErrorKind::TimedOut {
                        println!("\n[System] Connection lost: {:?}", e);
                        running_recv.store(false, Ordering::Relaxed);
                        break;
                    }
                }
            }
        }
    });

    // background coord sender thread (every 30 sec)
    let running_send = running.clone();
    let mut stream_send = stream.try_clone().expect("Failed to clone socket for sending");
    
    // interval thread
    thread::spawn(move || {
        println!("[Emulator] Movement simulation active. Reporting coordinates every 30s.");
        while running_send.load(Ordering::Relaxed) {
            if let Some(coord) = strategy.next_coordinate() {
                let report = ClientMessage::PositionReport {
                    coordinate: coord,
                    timestamp: Utc::now(),
                };
                
                if send_message(&mut stream_send, &report).is_err() {
                    println!("\n[Emulator] Failed to transmit position report to server.");
                    running_send.store(false, Ordering::Relaxed);
                    break;
                }
                // visual indicator of coordinate report
                // println!("[Report Sent] ({:.6}, {:.6})", coord.lat, coord.lon);
            } else {
                println!("[Emulator] Strategy returned no more coordinates. Ending tracking.");
                break;
            }

            // sleep 30 seconds, checking running flag
            for _ in 0..30 {
                if !running_send.load(Ordering::Relaxed) {
                    break;
                }
                thread::sleep(Duration::from_secs(1));
            }
        }
        println!("[Emulator] Movement simulator thread stopping.");
    });

    // text chat message interface on main thread
    let stdin = std::io::stdin();
    let mut input_reader = stdin.lock();
    let mut line = String::new();

    println!("\nYou are online! Type text messages and press ENTER to send them to the server.");
    println!("Type 'quit' or 'exit' to log out.\n");

    while running.load(Ordering::Relaxed) {
        print!("{}> ", username);
        let _ = std::io::stdout().flush();
        line.clear();
        if input_reader.read_line(&mut line).is_err() {
            break;
        }

        let msg_text = line.trim();
        if msg_text.is_empty() {
            continue;
        }

        if msg_text == "quit" || msg_text == "exit" {
            println!("[Client] Logging out...");
            running.store(false, Ordering::Relaxed);
            break;
        }

        let msg = ClientMessage::TextMessage { text: msg_text.to_string() };
        if send_message(&mut stream, &msg).is_err() {
            println!("[System] Failed to send message. Connection closed.");
            running.store(false, Ordering::Relaxed);
            break;
        }
    }

    println!("=== GeoRuggine Client Stopped ===");
}

fn run_auth_flow(stream: &mut TcpStream, reader: &mut BufReader<TcpStream>) -> (String, bool) {
    let stdin = std::io::stdin();
    let mut input_reader = stdin.lock();
    let mut line = String::new();

    loop {
        println!("\nWelcome to GeoRuggine. Select option:");
        println!("  1. Register new user");
        println!("  2. Login");
        println!("  3. Exit");
        print!("select> ");
        let _ = std::io::stdout().flush();

        line.clear();
        if input_reader.read_line(&mut line).is_err() {
            return (String::new(), false);
        }

        let choice = line.trim();
        if choice == "3" {
            return (String::new(), false);
        }

        if choice != "1" && choice != "2" {
            println!("Invalid choice.");
            continue;
        }

        print!("Username: ");
        let _ = std::io::stdout().flush();
        let mut username = String::new();
        if input_reader.read_line(&mut username).is_err() {
            return (String::new(), false);
        }
        let username = username.trim().to_string();

        print!("Password: ");
        let _ = std::io::stdout().flush();
        let mut password = String::new();
        if input_reader.read_line(&mut password).is_err() {
            return (String::new(), false);
        }
        let password = password.trim().to_string();

        let msg = if choice == "1" {
            ClientMessage::Register { username: username.clone(), password }
        } else {
            ClientMessage::Login { username: username.clone(), password }
        };

        if send_message(stream, &msg).is_err() {
            println!("Network error sending authentication request.");
            return (String::new(), false);
        }

        // Wait for response
        let mut response_line = String::new();
        if reader.read_line(&mut response_line).is_ok() {
            if let Ok(ServerMessage::AuthResponse { success, message }) = serde_json::from_str(&response_line) {
                println!("[Auth Server]: {}", message);
                if success && choice == "2" {
                    return (username, true);
                }
            } else {
                println!("Unexpected response format from server.");
            }
        } else {
            println!("Disconnected from authentication server.");
            return (String::new(), false);
        }
    }
}

fn select_movement_strategy() -> Box<dyn MovementStrategy + Send> {
    let stdin = std::io::stdin();
    let mut input_reader = stdin.lock();
    let mut line = String::new();

    loop {
        println!("\nSelect Movement Emulation Strategy:");
        println!("  1. File-based path playback");
        println!("  2. Pseudo-random walk (starting from Torino)");
        println!("  3. Interactive manual coordinates entry");
        print!("select> ");
        let _ = std::io::stdout().flush();

        line.clear();
        if input_reader.read_line(&mut line).is_err() {
            println!("Error reading input, defaulting to Random Walk.");
            return Box::new(RandomWalkStrategy::new(Coordinate { lat: 45.0618513, lon: 7.6606506 }));
        }

        match line.trim() {
            "1" => {
                print!("Enter path to coordinate file: ");
                let _ = std::io::stdout().flush();
                let mut path = String::new();
                if input_reader.read_line(&mut path).is_ok() {
                    let cleaned_path = path.trim();
                    match FileStrategy::new(cleaned_path) {
                        Ok(strat) => return Box::new(strat),
                        Err(e) => {
                            println!("Failed to open file: {:?}. Retrying strategy selection.", e);
                        }
                    }
                }
            }
            "2" => {
                // Torino coordinates (Torino => Asti example starting point)
                let start = Coordinate { lat: 45.0618513, lon: 7.6606506 };
                return Box::new(RandomWalkStrategy::new(start));
            }
            "3" => {
                return Box::new(ManualStrategy);
            }
            _ => {
                println!("Invalid selection. Enter 1, 2, or 3.");
            }
        }
    }
}

fn send_message(stream: &mut TcpStream, msg: &ClientMessage) -> std::io::Result<()> {
    let serialized = serde_json::to_string(msg)?;
    let mut payload = serialized;
    payload.push('\n');
    stream.write_all(payload.as_bytes())?;
    stream.flush()
}
