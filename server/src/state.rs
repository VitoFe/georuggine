use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Write, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use chrono::{DateTime, Utc};
use common::{Coordinate, UserStatus, ServerMessage};

#[derive(Debug)]
pub struct UserSession {
    pub status: UserStatus,
    pub last_coord: Option<Coordinate>,
    pub last_changed: Option<DateTime<Utc>>,
    pub connection_sender: Option<Sender<ServerMessage>>,
}

pub struct ServerState {
    pub users: HashMap<String, String>, // username -> password (simple plain text storage for educational clarity)
    pub sessions: HashMap<String, UserSession>,
    pub trajectories: HashMap<String, Vec<(DateTime<Utc>, Coordinate, UserStatus)>>,
    users_filepath: String,
    trajectories_dir: String,
}

impl ServerState {
    pub fn new(users_filepath: &str, trajectories_dir: &str) -> Self {
        let mut state = Self {
            users: HashMap::new(),
            sessions: HashMap::new(),
            trajectories: HashMap::new(),
            users_filepath: users_filepath.to_string(),
            trajectories_dir: trajectories_dir.to_string(),
        };
        state.load_users();
        state.load_trajectories();
        state
    }

    /// Register a new user. Returns true if successful, false if user already exists.
    pub fn register(&mut self, username: String, password: String) -> bool {
        if self.users.contains_key(&username) {
            return false;
        }
        let hashed = hash_password(&password);
        self.users.insert(username.clone(), hashed);
        self.save_users();
        
        // Initialize user session state as Disconnected
        self.sessions.insert(username, UserSession {
            status: UserStatus::Disconnected,
            last_coord: None,
            last_changed: None,
            connection_sender: None,
        });
        true
    }

    /// Authenticate a user. Returns true if successful.
    pub fn login(&mut self, username: &str, password: &str, sender: Sender<ServerMessage>) -> bool {
        if let Some(saved_pwd) = self.users.get(username) {
            let hashed_input = hash_password(password);
            if saved_pwd == &hashed_input {
                // If there's an existing connection, it will be replaced
                self.sessions.insert(username.to_string(), UserSession {
                    status: UserStatus::Stationary, // Initial state upon login is Stationary
                    last_coord: None,
                    last_changed: None,
                    connection_sender: Some(sender),
                });
                println!("[ServerState] User '{}' logged in. State set to Stationary.", username);
                return true;
            }
        }
        false
    }

    /// Disconnect a user
    pub fn disconnect(&mut self, username: &str) {
        if let Some(session) = self.sessions.get_mut(username) {
            session.status = UserStatus::Disconnected;
            session.connection_sender = None;
            println!("[ServerState] User '{}' disconnected.", username);
        }
    }

    /// Updates the position of a user and processes state transitions
    /// - Stationary -> Moving: coordinate change
    /// - Moving -> Stationary: no change for >= 3 minutes
    pub fn update_position(&mut self, username: &str, coord: Coordinate, time: DateTime<Utc>) -> UserStatus {
        let session = match self.sessions.get_mut(username) {
            Some(s) => s,
            None => return UserStatus::Disconnected,
        };

        if session.status == UserStatus::Disconnected {
            return UserStatus::Disconnected;
        }

        let old_status = session.status;
        let mut new_status = old_status;

        match session.last_coord {
            None => {
                // First coordinate report
                session.last_coord = Some(coord);
                session.last_changed = Some(time);
                new_status = UserStatus::Stationary;
            }
            Some(prev_coord) => {
                // Check if coordinate changed (epsilon check for floating-point coords)
                let coord_changed = (coord.lat - prev_coord.lat).abs() > 1e-7 
                                 || (coord.lon - prev_coord.lon).abs() > 1e-7;

                if coord_changed {
                    session.last_coord = Some(coord);
                    session.last_changed = Some(time);
                    new_status = UserStatus::Moving;
                } else {
                    // Coordinate is the same. Check duration of inactivity if currently Moving.
                    if old_status == UserStatus::Moving {
                        if let Some(last_chg) = session.last_changed {
                            let duration = time.signed_duration_since(last_chg);
                            if duration.num_seconds() >= 180 { // 3 minutes = 180 seconds
                                new_status = UserStatus::Stationary;
                                session.last_changed = Some(time); // Reset change tracker to current pause
                            }
                        }
                    }
                }
            }
        }

        session.status = new_status;
        
        // Log to in-memory trajectory
        self.trajectories.entry(username.to_string())
            .or_default()
            .push((time, coord, new_status));

        // Persist report to file
        self.save_trajectory_point(username, time, coord, new_status);

        if old_status != new_status {
            println!("[ServerState] State Transition for '{}': {} -> {}", username, old_status, new_status);
        }

        new_status
    }

    /// Send a message to a specific logged-in user
    pub fn send_message(&mut self, username: &str, msg: &ServerMessage) -> bool {
        if let Some(session) = self.sessions.get_mut(username) {
            if let Some(ref tx) = session.connection_sender {
                if tx.send(msg.clone()).is_ok() {
                    return true;
                }
                // Send failed, mark connection as broken
                session.connection_sender = None;
                session.status = UserStatus::Disconnected;
            }
        }
        false
    }

    /// Send a message to all active users
    pub fn broadcast_message(&mut self, msg: &ServerMessage) {
        let active_users: Vec<String> = self.sessions.iter()
            .filter(|(_, session)| session.connection_sender.is_some())
            .map(|(username, _)| username.clone())
            .collect();

        for username in active_users {
            self.send_message(&username, msg);
        }
    }

    // --- Persistence ---

    fn load_users(&mut self) {
        if !Path::new(&self.users_filepath).exists() {
            return;
        }
        if let Ok(file) = fs::File::open(&self.users_filepath) {
            let reader = BufReader::new(file);
            for line in reader.lines().map_while(Result::ok) {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() == 2 {
                    let username = parts[0].to_string();
                    let password = parts[1].to_string();
                    self.users.insert(username.clone(), password);
                    // Initialize empty session
                    self.sessions.insert(username, UserSession {
                        status: UserStatus::Disconnected,
                        last_coord: None,
                        last_changed: None,
                        connection_sender: None,
                    });
                }
            }
        }
    }

    fn save_users(&self) {
        if let Ok(mut file) = fs::File::create(&self.users_filepath) {
            for (user, pwd) in &self.users {
                let _ = writeln!(file, "{}:{}", user, pwd);
            }
        }
    }

    fn load_trajectories(&mut self) {
        let _ = fs::create_dir_all(&self.trajectories_dir);
        if let Ok(entries) = fs::read_dir(&self.trajectories_dir) {
            for entry in entries.map_while(Result::ok) {
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                    if let Some(username) = path.file_stem().and_then(|s| s.to_str()) {
                        let mut points = Vec::new();
                        if let Ok(file) = fs::File::open(&path) {
                            let reader = BufReader::new(file);
                            for line in reader.lines().map_while(Result::ok) {
                                // Expected format: timestamp_rfc3339 lat lon status
                                let parts: Vec<&str> = line.split_whitespace().collect();
                                if parts.len() == 4 {
                                    if let (Ok(time), Ok(lat), Ok(lon)) = (
                                        DateTime::parse_from_rfc3339(parts[0]),
                                        parts[1].parse::<f64>(),
                                        parts[2].parse::<f64>(),
                                    ) {
                                        let status = match parts[3] {
                                            "Moving" => UserStatus::Moving,
                                            _ => UserStatus::Stationary,
                                        };
                                        points.push((
                                            time.with_timezone(&Utc),
                                            Coordinate { lat, lon },
                                            status,
                                        ));
                                    }
                                }
                            }
                        }
                        self.trajectories.insert(username.to_string(), points);
                    }
                }
            }
        }
    }

    fn save_trajectory_point(&self, username: &str, time: DateTime<Utc>, coord: Coordinate, status: UserStatus) {
        let filepath = PathBuf::from(&self.trajectories_dir).join(format!("{}.jsonl", username));
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(filepath)
        {
            let status_str = match status {
                UserStatus::Moving => "Moving",
                _ => "Stationary",
            };
            let _ = writeln!(
                file,
                "{} {} {} {}",
                time.to_rfc3339(),
                coord.lat,
                coord.lon,
                status_str
            );
        }
    }
}

fn hash_password(password: &str) -> String {
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    let result = hasher.finalize();
    result.iter().map(|b| format!("{:02x}", b)).collect::<String>()
}
