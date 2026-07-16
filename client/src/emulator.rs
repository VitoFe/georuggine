use std::fs::File;
use std::io::{BufRead, BufReader};
use common::{Coordinate, parse_coordinate_value};

pub trait MovementStrategy {
    fn next_coordinate(&mut self) -> Option<Coordinate>;
}

// File-based Movement Strategy
pub struct FileStrategy {
    reader: BufReader<File>,
    last_coordinate: Option<Coordinate>,
}

impl FileStrategy {
    pub fn new(filepath: &str) -> std::io::Result<Self> {
        let file = File::open(filepath)?;
        Ok(Self {
            reader: BufReader::new(file),
            last_coordinate: None,
        })
    }
}

impl MovementStrategy for FileStrategy {
    fn next_coordinate(&mut self) -> Option<Coordinate> {
        let mut line = String::new();
        let mut seeked = false;
        loop {
            line.clear();
            match self.reader.read_line(&mut line) {
                Ok(0) => {
                    if seeked {
                        return self.last_coordinate;
                    }
                    // Loop back to the start if we reach EOF
                    use std::io::{Seek, SeekFrom};
                    if self.reader.seek(SeekFrom::Start(0)).is_err() {
                        return self.last_coordinate;
                    }
                    seeked = true;
                }
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('-') {
                        continue;
                    }
                    
                    // line like: "00:30 45,0575226 7,6618322" or "45.0575226 7.6618322"
                    let parts: Vec<&str> = trimmed.split_whitespace().collect();
                    if parts.len() >= 2 {
                        // If three parts, the first is timestamp, second is lat, third is lon
                        let (lat_str, lon_str) = if parts.len() == 3 {
                            (parts[1], parts[2])
                        } else {
                            (parts[0], parts[1])
                        };

                        if let (Ok(lat), Ok(lon)) = (
                            parse_coordinate_value(lat_str),
                            parse_coordinate_value(lon_str),
                        ) {
                            let coord = Coordinate { lat, lon };
                            self.last_coordinate = Some(coord);
                            return Some(coord);
                        }
                    }
                }
                Err(_) => return self.last_coordinate,
            }
        }
    }
}

// Pseudo-Random Walk Strategy
// using Linear Congruential Generator (LCG)
pub struct RandomWalkStrategy {
    state: u64,
    current_lat: f64,
    current_lon: f64,
}

impl RandomWalkStrategy {
    pub fn new(start: Coordinate) -> Self {
        Self {
            state: 123456789, // Seed value
            current_lat: start.lat,
            current_lon: start.lon,
        }
    }

    // Simple LCG PRNG yielding a float in [-1.0, 1.0)
    fn next_rand(&mut self) -> f64 {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let val = (self.state >> 32) as u32;
        (val as f64 / u32::MAX as f64) * 2.0 - 1.0
    }
}

impl MovementStrategy for RandomWalkStrategy {
    fn next_coordinate(&mut self) -> Option<Coordinate> {
        // Step size of approx 0.0001 (approx 10-15 meters)
        let lat_step = self.next_rand() * 0.0001;
        let lon_step = self.next_rand() * 0.0001;

        self.current_lat += lat_step;
        self.current_lon += lon_step;

        Some(Coordinate {
            lat: self.current_lat,
            lon: self.current_lon,
        })
    }
}

// Interactive Manual Strategy
pub struct ManualStrategy;

impl MovementStrategy for ManualStrategy {
    fn next_coordinate(&mut self) -> Option<Coordinate> {
        let stdin = std::io::stdin();
        let mut line = String::new();
        loop {
            println!("\n[Manual Input] Enter coordinate (format: 'lat lon' or 'lat,lon'):");
            print!("coords> ");
            use std::io::Write;
            let _ = std::io::stdout().flush();

            line.clear();
            if stdin.read_line(&mut line).is_err() || line.trim().is_empty() {
                continue;
            }

            let cleaned = line.replace(',', " ");
            let parts: Vec<&str> = cleaned.split_whitespace().collect();
            if parts.len() == 2 {
                if let (Ok(lat), Ok(lon)) = (
                    parse_coordinate_value(parts[0]),
                    parse_coordinate_value(parts[1]),
                ) {
                    return Some(Coordinate { lat, lon });
                }
            }
            println!("Invalid format. Try again.");
        }
    }
}
