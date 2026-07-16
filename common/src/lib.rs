use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
pub struct Coordinate {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum UserStatus {
    Disconnected,
    Stationary, // "fermo"
    Moving,     // "in movimento"
}

impl std::fmt::Display for UserStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserStatus::Disconnected => write!(f, "Disconnected"),
            UserStatus::Stationary => write!(f, "Stationary"),
            UserStatus::Moving => write!(f, "Moving"),
        }
    }
}

/// Client to Server messages
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum ClientMessage {
    Register { username: String, password: String },
    Login { username: String, password: String },
    PositionReport { coordinate: Coordinate, timestamp: DateTime<Utc> },
    TextMessage { text: String },
}

/// Server to Client messages
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum ServerMessage {
    AuthResponse { success: bool, message: String },
    TextMessage { sender: String, text: String },
    CommandResponse { success: bool, message: String },
}

/// Helper to parse a coordinate string that might use a comma instead of a period as a decimal separator
pub fn parse_coordinate_value(s: &str) -> Result<f64, std::num::ParseFloatError> {
    let normalized = s.replace(',', ".");
    normalized.trim().parse::<f64>()
}

/// Calculates the geodesic distance between two points on Earth using the Haversine formula (in meters)
pub fn calculate_distance(p1: Coordinate, p2: Coordinate) -> f64 {
    let r = 6371000.0; // Earth radius in meters
    let d_lat = (p2.lat - p1.lat).to_radians();
    let d_lon = (p2.lon - p1.lon).to_radians();
    let a = (d_lat / 2.0).sin().powi(2)
        + p1.lat.to_radians().cos() * p2.lat.to_radians().cos() * (d_lon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
    r * c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_coordinate_value() {
        assert_eq!(parse_coordinate_value("45.0618513").unwrap(), 45.0618513);
        assert_eq!(parse_coordinate_value("45,0618513").unwrap(), 45.0618513);
        assert_eq!(parse_coordinate_value(" 7,6606506 ").unwrap(), 7.6606506);
    }

    #[test]
    fn test_calculate_distance() {
        // Torino coordinates (approx)
        let p1 = Coordinate { lat: 45.0618513, lon: 7.6606506 };
        // Asti coordinates (approx)
        let p2 = Coordinate { lat: 44.9011802, lon: 8.2064197 };
        
        let dist = calculate_distance(p1, p2);
        // Distance between Torino and Asti is roughly 45-50 km
        assert!(dist > 40000.0 && dist < 55000.0);
    }
}
