use chrono::{DateTime, Utc, Datelike};
use common::{Coordinate, UserStatus, calculate_distance};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeInterval {
    CurrentDay,
    CurrentWeek,
    CurrentMonth,
    All,
}

#[derive(Debug)]
pub struct AnalysisResult {
    pub route: Vec<Coordinate>,
    pub total_distance: f64,              // in meters
    pub movement_duration_secs: i64,      // in seconds
    pub pause_duration_secs: i64,         // in seconds
    pub average_speed_mps: f64,           // in meters/second
}

/// Helper to get the start time for a given programmable interval
pub fn get_interval_start(interval: TimeInterval, now: DateTime<Utc>) -> DateTime<Utc> {
    match interval {
        TimeInterval::CurrentDay => {
            now.date_naive()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_local_timezone(Utc)
                .unwrap()
        }
        TimeInterval::CurrentWeek => {
            let weekday = now.weekday();
            let days_from_monday = weekday.num_days_from_monday();
            let start_of_day = now.date_naive()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_local_timezone(Utc)
                .unwrap();
            start_of_day - chrono::Duration::days(days_from_monday as i64)
        }
        TimeInterval::CurrentMonth => {
            now.date_naive()
                .with_day(1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_local_timezone(Utc)
                .unwrap()
        }
        TimeInterval::All => {
            DateTime::from_timestamp(0, 0).unwrap()
        }
    }
}

pub fn analyze_movement(
    history: &[(DateTime<Utc>, Coordinate, UserStatus)],
    interval: TimeInterval,
    now: DateTime<Utc>,
) -> AnalysisResult {
    let start_time = get_interval_start(interval, now);

    // Filter points by time interval
    let filtered_points: Vec<&(DateTime<Utc>, Coordinate, UserStatus)> = history
        .iter()
        .filter(|(time, _, _)| *time >= start_time)
        .collect();

    let mut route = Vec::new();
    let mut total_distance = 0.0;
    let mut movement_duration_secs = 0;
    let mut pause_duration_secs = 0;

    if filtered_points.is_empty() {
        return AnalysisResult {
            route,
            total_distance,
            movement_duration_secs,
            pause_duration_secs,
            average_speed_mps: 0.0,
        };
    }

    // Extract coordinate points for the route taken
    for (_, coord, _) in &filtered_points {
        route.push(*coord);
    }

    // Process consecutive points to compute distances and durations
    for i in 0..filtered_points.len() - 1 {
        let (t1, p1, _) = filtered_points[i];
        let (t2, p2, s2) = filtered_points[i + 1];

        let time_delta = t2.signed_duration_since(*t1).num_seconds();

        // only count the duration and distance if the gap is <= 60 seconds.
        if time_delta > 0 && time_delta <= 60 {
            let dist = calculate_distance(*p1, *p2);
            total_distance += dist;

            match s2 {
                UserStatus::Moving => {
                    movement_duration_secs += time_delta;
                }
                UserStatus::Stationary => {
                    pause_duration_secs += time_delta;
                }
                UserStatus::Disconnected => {}
            }
        }
    }

    let average_speed_mps = if movement_duration_secs > 0 {
        total_distance / (movement_duration_secs as f64)
    } else {
        0.0
    };

    AnalysisResult {
        route,
        total_distance,
        movement_duration_secs,
        pause_duration_secs,
        average_speed_mps,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_analyze_movement() {
        let start = Utc.with_ymd_and_hms(2026, 7, 16, 12, 0, 0).unwrap();
        
        let p1 = Coordinate { lat: 45.0, lon: 7.0 };
        let p2 = Coordinate { lat: 45.001, lon: 7.001 }; // Moved
        let p3 = Coordinate { lat: 45.001, lon: 7.001 }; // Stationary

        // coord updates every 30s
        let history = vec![
            (start, p1, UserStatus::Stationary),
            (start + chrono::Duration::seconds(30), p2, UserStatus::Moving),
            (start + chrono::Duration::seconds(60), p3, UserStatus::Stationary),
        ];

        let result = analyze_movement(&history, TimeInterval::All, start);
        assert_eq!(result.route.len(), 3);
        assert!(result.total_distance > 0.0);
        assert_eq!(result.movement_duration_secs, 30);
        assert_eq!(result.pause_duration_secs, 30);
        assert!(result.average_speed_mps > 0.0);
    }
}
