use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use chrono::Utc;
use common::{ClientMessage, ServerMessage};

mod state;
mod cpu;
mod analysis;

use state::ServerState;
use analysis::{TimeInterval, analyze_movement};

fn main() {
    println!("=== GeoRuggine Server Starting ===");

    let users_file = "users.txt";
    let trajectories_dir = "trajectories";
    let cpu_log_file = "server_cpu.log";

    // shared state
    let state = Arc::new(Mutex::new(ServerState::new(users_file, trajectories_dir)));
    
    // global shutdown signal
    let running = Arc::new(AtomicBool::new(true));

    // CPU monitor thread (logs every 2 minutes)
    cpu::start_cpu_monitoring(running.clone(), cpu_log_file);

    let listener = match TcpListener::bind("127.0.0.1:8080") {
        Ok(l) => {
            println!("[Server] Listening on 127.0.0.1:8080");
            l
        }
        Err(e) => {
            eprintln!("[Server] Failed to bind TCP listener on 8080: {:?}", e);
            running.store(false, Ordering::Relaxed);
            return;
        }
    };

    // listener to non-blocking so we can check the running flag and shut down gracefully
    if let Err(e) = listener.set_nonblocking(true) {
        eprintln!("[Server] Failed to set TCP listener non-blocking: {:?}", e);
    }

    // TCP connection accepting thread
    let running_accept = running.clone();
    let state_accept = state.clone();
    let accept_thread = thread::spawn(move || {
        while running_accept.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, addr)) => {
                    println!("[Server] New connection from {}", addr);
                    let state_clone = state_accept.clone();
                    let running_clone = running_accept.clone();
                    thread::spawn(move || {
                        handle_client(stream, state_clone, running_clone);
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No incoming connection, sleep briefly
                    thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    eprintln!("[Server] Accept error: {:?}", e);
                    thread::sleep(Duration::from_millis(500));
                }
            }
        }
        println!("[Server] TCP listener thread stopping.");
    });

    // CLI is on the main thread
    run_server_cli(state.clone(), running.clone());

    // wait for shutdown
    running.store(false, Ordering::Relaxed);
    let _ = accept_thread.join();
    println!("=== GeoRuggine Server Stopped ===");
}

fn handle_client(stream: TcpStream, state: Arc<Mutex<ServerState>>, running: Arc<AtomicBool>) {
    let peer_addr = match stream.peer_addr() {
        Ok(addr) => addr,
        Err(_) => return,
    };

    // read and write timeouts to prevent slow or inactive client hangs
    if let Err(e) = stream.set_read_timeout(Some(Duration::from_secs(45))) {
        eprintln!("[ClientHandler] Failed to set read timeout for {}: {:?}", peer_addr, e);
    }
    if let Err(e) = stream.set_write_timeout(Some(Duration::from_secs(10))) {
        eprintln!("[ClientHandler] Failed to set write timeout for {}: {:?}", peer_addr, e);
    }

    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });

    let mut logged_in_user: Option<String> = None;
    let mut writer_channel: Option<std::sync::mpsc::Sender<ServerMessage>> = None;
    let mut line = String::new();

    while running.load(Ordering::Relaxed) {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {
                // client disconnected
                break;
            }
            Ok(_) => {
                let msg_str = line.trim();
                if msg_str.is_empty() {
                    continue;
                }

                let client_msg: ClientMessage = match serde_json::from_str(msg_str) {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!("[ClientHandler] Failed to parse message from {}: {:?}. Raw: {}", peer_addr, e, msg_str);
                        continue;
                    }
                };

                let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
                match client_msg {
                    ClientMessage::Register { username, password } => {
                        let success = s.register(username.clone(), password);
                        let response = ServerMessage::AuthResponse {
                            success,
                            message: if success {
                                "Registration successful.".to_string()
                            } else {
                                "Registration failed: User already exists.".to_string()
                            },
                        };
                        drop(s); // release lock
                        send_response(&stream, &response);
                    }
                    ClientMessage::Login { username, password } => {
                        let (tx, rx) = std::sync::mpsc::channel::<ServerMessage>();
                        let success = s.login(&username, &password, tx.clone());
                        let response = ServerMessage::AuthResponse {
                            success,
                            message: if success {
                                "Login successful.".to_string()
                            } else {
                                "Login failed: Invalid credentials.".to_string()
                            },
                        };
                        drop(s);

                        if success {
                            logged_in_user = Some(username.clone());
                            writer_channel = Some(tx.clone());

                            let mut write_stream = match stream.try_clone() {
                                Ok(c) => c,
                                Err(e) => {
                                    eprintln!("[ClientHandler] Failed to clone stream for writer thread: {:?}", e);
                                    break;
                                }
                            };
                            let running_writer = running.clone();
                            let username_writer = username.clone();
                            let state_writer = state.clone();

                            // client writer thread
                            thread::spawn(move || {
                                while running_writer.load(Ordering::Relaxed) {
                                    match rx.recv() {
                                        Ok(msg) => {
                                            if let Ok(serialized) = serde_json::to_string(&msg) {
                                                let mut payload = serialized;
                                                payload.push('\n');
                                                if write_stream.write_all(payload.as_bytes()).is_err() {
                                                    eprintln!("[WriterThread] Write error to client {}", username_writer);
                                                    break;
                                                }
                                                let _ = write_stream.flush();
                                            }
                                        }
                                        Err(_) => break, // sender dropped (logout or connection overwrite)
                                    }
                                }
                                let mut s = state_writer.lock().unwrap_or_else(|e| e.into_inner());
                                s.disconnect(&username_writer);
                                println!("[WriterThread] Stopped for user {}", username_writer);
                            });

                            let _ = tx.send(response);
                        } else {
                            send_response(&stream, &response);
                        }
                    }
                    ClientMessage::PositionReport { coordinate, timestamp } => {
                        if let Some(ref username) = logged_in_user {
                            let new_status = s.update_position(username, coordinate, timestamp);
                            let response = ServerMessage::CommandResponse {
                                success: true,
                                message: format!("Position registered. Status: {}", new_status),
                            };
                            drop(s);
                            if let Some(ref tx) = writer_channel {
                                let _ = tx.send(response);
                            }
                        } else {
                            drop(s);
                            let response = ServerMessage::CommandResponse {
                                success: false,
                                message: "Unauthorized. Please login first.".to_string(),
                            };
                            send_response(&stream, &response);
                        }
                    }
                    ClientMessage::TextMessage { text } => {
                        if let Some(ref username) = logged_in_user {
                            println!("\n[Chat from {}]: {}", username, text);
                            print!("server> ");
                            let _ = std::io::stdout().flush();

                            let response = ServerMessage::CommandResponse {
                                success: true,
                                message: "Message received by server.".to_string(),
                            };
                            drop(s);
                            if let Some(ref tx) = writer_channel {
                                let _ = tx.send(response);
                            }
                        } else {
                            drop(s);
                            let response = ServerMessage::CommandResponse {
                                success: false,
                                message: "Unauthorized. Please login first.".to_string(),
                            };
                            send_response(&stream, &response);
                        }
                    }
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                // Read timed out/would block. sleep and loop
                thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                eprintln!("[ClientHandler] Read error for connection {}: {:?}", peer_addr, e);
                break;
            }
        }
    }

    // Cleanup
    if let Some(ref username) = logged_in_user {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.disconnect(username);
    }
    println!("[Server] Connection closed with {}", peer_addr);
}

fn send_response(mut stream: &TcpStream, msg: &ServerMessage) {
    if let Ok(serialized) = serde_json::to_string(msg) {
        let mut payload = serialized;
        payload.push('\n');
        let _ = stream.write_all(payload.as_bytes());
    }
}

fn run_server_cli(state: Arc<Mutex<ServerState>>, running: Arc<AtomicBool>) {
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let mut line = String::new();

    println!("Server CLI:");
    println!("  list                         - List all users and current status");
    println!("  send <user> <message>        - Send a direct message to a user");
    println!("  broadcast <message>          - Broadcast a message to all active users");
    println!("  analyze <user> <interval>    - Analyze user's route (interval: day | week | month | all)");
    println!("  exit                         - Shutdown the server");

    while running.load(Ordering::Relaxed) {
        print!("server> ");
        let _ = std::io::stdout().flush();
        line.clear();

        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {
                let cmd = line.trim();
                if cmd.is_empty() {
                    continue;
                }

                let parts: Vec<&str> = cmd.split_whitespace().collect();
                if parts.is_empty() {
                    continue;
                }

                match parts[0] {
                    "list" => {
                        let s = state.lock().unwrap_or_else(|e| e.into_inner());
                        println!("Registered Users & Current Status:");
                        for (username, session) in &s.sessions {
                            let last_coord_str = match session.last_coord {
                                Some(c) => format!("({:.6}, {:.6})", c.lat, c.lon),
                                None => "None".to_string(),
                            };
                            println!("  - {}: Status = {}, Last Coord = {}", username, session.status, last_coord_str);
                        }
                    }
                    "send" => {
                        if parts.len() < 3 {
                            println!("Usage: send <user> <message>");
                            continue;
                        }
                        let target = parts[1];
                        let message = parts[2..].join(" ");
                        
                        let msg = ServerMessage::TextMessage {
                            sender: "Server (DM)".to_string(),
                            text: message.clone(),
                        };

                        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
                        if s.send_message(target, &msg) {
                            println!("Message sent to '{}'.", target);
                        } else {
                            println!("Failed to send: '{}' is not active.", target);
                        }
                    }
                    "broadcast" => {
                        if parts.len() < 2 {
                            println!("Usage: broadcast <message>");
                            continue;
                        }
                        let message = parts[1..].join(" ");
                        let msg = ServerMessage::TextMessage {
                            sender: "Server (Broadcast)".to_string(),
                            text: message.clone(),
                        };

                        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
                        s.broadcast_message(&msg);
                        println!("Broadcast sent.");
                    }
                    "analyze" => {
                        if parts.len() < 3 {
                            println!("Usage: analyze <user> <interval> (interval: day | week | month | all)");
                            continue;
                        }
                        let target = parts[1];
                        let interval_str = parts[2].to_lowercase();
                        let interval = match interval_str.as_str() {
                            "day" => TimeInterval::CurrentDay,
                            "week" => TimeInterval::CurrentWeek,
                            "month" => TimeInterval::CurrentMonth,
                            "all" => TimeInterval::All,
                            _ => {
                                println!("Invalid interval. Choose from: day, week, month, all");
                                continue;
                            }
                        };

                        let s = state.lock().unwrap_or_else(|e| e.into_inner());
                        if let Some(history) = s.trajectories.get(target) {
                            let result = analyze_movement(history, interval, Utc::now());
                            println!("\n=== Movement Analysis for '{}' ({}) ===", target, interval_str);
                            println!("  Route Points: {}", result.route.len());
                            println!("  Total Distance Covered: {:.2} meters ({:.2} km)", result.total_distance, result.total_distance / 1000.0);
                            println!("  Duration in Motion (Moving): {}s", result.movement_duration_secs);
                            println!("  Duration Stationary (Pauses): {}s", result.pause_duration_secs);
                            
                            // Speeds
                            let speed_mps = result.average_speed_mps;
                            let speed_kmh = speed_mps * 3.6;
                            println!("  Average Speed: {:.2} m/s ({:.2} km/h)", speed_mps, speed_kmh);
                            println!("===================================================\n");
                        } else {
                            println!("No trajectory data found for '{}'.", target);
                        }
                    }
                    "exit" => {
                        println!("Initiating server shutdown...");
                        running.store(false, Ordering::Relaxed);
                        break;
                    }
                    _ => {
                        println!("Unknown command. Type 'list', 'send', 'broadcast', 'analyze', or 'exit'.");
                    }
                }
            }
            Err(_) => break,
        }
    }
}
