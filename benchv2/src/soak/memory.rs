//! RSS samples for the database process while the soak is running.

use std::{
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use serde::Serialize;
use sysinfo::{Pid, ProcessesToUpdate, System};

#[derive(Debug, Clone, Serialize)]
pub struct MemorySample {
    pub t_ms:        u64,
    pub server_rss:  u64,
    pub server_pids: usize,
    pub system_used: u64,
}

#[derive(Debug, Clone)]
pub struct MemoryReading {
    pub server_rss:  u64,
    pub server_pids: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryTrack {
    pub peak_rss: u64,
    pub samples:  Vec<MemorySample>,
}

pub struct MemorySampler {
    stop:   Arc<Mutex<bool>>,
    latest: Arc<Mutex<MemoryReading>>,
    track:  Arc<Mutex<MemoryTrack>>,
    handle: Option<JoinHandle<()>>,
}

impl MemorySampler {
    pub fn start(server_pid: u32) -> Self {
        let stop = Arc::new(Mutex::new(false));
        let latest = Arc::new(Mutex::new(MemoryReading {
            server_rss:  0,
            server_pids: 0,
        }));
        let track = Arc::new(Mutex::new(MemoryTrack {
            peak_rss: 0,
            samples:  Vec::new(),
        }));
        let stop_flag = Arc::clone(&stop);
        let latest_slot = Arc::clone(&latest);
        let track_slot = Arc::clone(&track);
        let handle = thread::spawn(move || {
            let started = Instant::now();
            let mut system = System::new();
            loop {
                if stop_flag.lock().map(|flag| *flag).unwrap_or(true) {
                    break;
                }
                let reading = sample_server(&mut system, server_pid);
                if let Ok(mut slot) = latest_slot.lock() {
                    *slot = reading.clone();
                }
                if let Ok(mut track) = track_slot.lock() {
                    track.peak_rss = track.peak_rss.max(reading.server_rss);
                    track.samples.push(MemorySample {
                        t_ms:        started.elapsed().as_millis() as u64,
                        server_rss:  reading.server_rss,
                        server_pids: reading.server_pids,
                        system_used: system.used_memory(),
                    });
                }
                thread::sleep(Duration::from_secs(5));
            }
        });
        let sampler = Self {
            stop,
            latest,
            track,
            handle: Some(handle),
        };
        thread::sleep(Duration::from_millis(200));
        sampler
    }

    pub fn latest_handle(&self) -> Arc<Mutex<MemoryReading>> {
        Arc::clone(&self.latest)
    }

    pub fn latest(&self) -> MemoryReading {
        self.latest.lock().map(|reading| reading.clone()).unwrap_or(MemoryReading {
            server_rss:  0,
            server_pids: 0,
        })
    }

    pub fn stop(mut self) -> MemoryTrack {
        if let Ok(mut flag) = self.stop.lock() {
            *flag = true;
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        self.track.lock().map(|track| track.clone()).unwrap_or(MemoryTrack {
            peak_rss: 0,
            samples:  Vec::new(),
        })
    }
}

fn sample_server(system: &mut System, server_pid: u32) -> MemoryReading {
    system.refresh_memory();
    if server_pid > 0 {
        let pid = Pid::from_u32(server_pid);
        system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
        let rss = system.process(pid).map(|process| process.memory()).unwrap_or(0);
        return MemoryReading {
            server_rss:  rss,
            server_pids: usize::from(rss > 0),
        };
    }

    system.refresh_processes(ProcessesToUpdate::All, true);
    let mut rss = 0u64;
    let mut pids = 0usize;
    for process in system.processes().values() {
        if is_server_process(process) {
            rss = rss.saturating_add(process.memory());
            pids += 1;
        }
    }
    MemoryReading {
        server_rss:  rss,
        server_pids: pids,
    }
}

fn is_server_process(process: &sysinfo::Process) -> bool {
    let name = process.name().to_string_lossy();
    if name.contains("kalamdb-server") {
        return true;
    }
    process.cmd().iter().any(|arg| arg.to_string_lossy().contains("kalamdb-server"))
}

/// Memory came back down when the process is still close to its baseline,
/// or at least half of the rise above baseline has been released.
pub fn memory_released(baseline: u64, peak: u64, after: u64) -> bool {
    if baseline == 0 || after == 0 {
        return false;
    }
    let allowance = baseline.saturating_add(64 * 1024 * 1024);
    if after <= allowance.max(baseline.saturating_mul(130) / 100) {
        return true;
    }
    let rise = peak.saturating_sub(baseline);
    if rise == 0 {
        return after <= allowance;
    }
    let dropped = peak.saturating_sub(after);
    dropped.saturating_mul(100) / rise >= 50
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn released_when_rss_returns_near_baseline() {
        let baseline = 200 * 1024 * 1024;
        let peak = 700 * 1024 * 1024;
        let after = 240 * 1024 * 1024;
        assert!(memory_released(baseline, peak, after));
    }

    #[test]
    fn not_released_when_peak_sticks() {
        let baseline = 200 * 1024 * 1024;
        let peak = 900 * 1024 * 1024;
        let after = 860 * 1024 * 1024;
        assert!(!memory_released(baseline, peak, after));
    }
}
