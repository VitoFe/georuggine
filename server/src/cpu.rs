use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use chrono::Utc;
use cpu_time::ProcessTime;

pub fn start_cpu_monitoring(running: Arc<AtomicBool>, log_filepath: &'static str) {
    thread::spawn(move || {
        println!("[CPU Monitor] Thread started. Logging to '{}' every 2 minutes.", log_filepath);
        
        let mut last_cpu = match ProcessTime::try_now() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[CPU Monitor] Failed to initialize CPU monitoring: {:?}", e);
                return;
            }
        };
        let mut last_instant = Instant::now();

        while running.load(Ordering::Relaxed) {
            // Sleep in small increments to be responsive to shutdown signals
            for _ in 0..120 { // 2 minutes
                if !running.load(Ordering::Relaxed) {
                    break;
                }
                thread::sleep(Duration::from_secs(1));
            }

            if !running.load(Ordering::Relaxed) {
                break;
            }

            let current_cpu = match ProcessTime::try_now() {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("[CPU Monitor] Failed to read CPU time: {:?}", e);
                    continue;
                }
            };
            let current_instant = Instant::now();

            let cpu_delta = current_cpu.duration_since(last_cpu);
            let wall_delta = current_instant.duration_since(last_instant);

            let cpu_delta_secs = cpu_delta.as_secs_f64();
            let wall_delta_secs = wall_delta.as_secs_f64();

            let cpu_usage_percent = if wall_delta_secs > 0.0 {
                (cpu_delta_secs / wall_delta_secs) * 100.0
            } else {
                0.0
            };


            let log_line = format!(
                "[{}] Log Interval: 2 min | Wall Time Delta: {:.2}s | CPU Time Delta: {:.4}s | Avg CPU Load: {:.2}%\n",
                Utc::now().to_rfc3339(),
                wall_delta_secs,
                cpu_delta_secs,
                cpu_usage_percent
            );

            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_filepath)
            {
                let _ = file.write_all(log_line.as_bytes());
            }

            last_cpu = current_cpu;
            last_instant = current_instant;
        }
        
        println!("[CPU Monitor] Thread stopped.");
    });
}
