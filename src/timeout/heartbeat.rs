//! Heartbeat monitor for stall detection.
//!
//! This module provides a heartbeat monitoring system that detects stalled
//! agents by tracking time between heartbeat pulses. When pulses stop arriving,
//! warnings and stall detection events are sent through a channel.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use super::TimeoutConfig;

/// Events emitted by the heartbeat monitor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeartbeatEvent {
    /// Warning: heartbeats are being missed but threshold not yet reached.
    Warning {
        /// Number of missed heartbeats.
        missed: u32,
        /// Elapsed seconds since last heartbeat.
        elapsed_secs: u64,
        /// Seconds until stall detection triggers.
        remaining_secs: u64,
    },
    /// Stall detected: missed heartbeats threshold has been reached.
    StallDetected {
        /// Number of missed heartbeats.
        missed: u32,
        /// Elapsed seconds since last heartbeat.
        elapsed_secs: u64,
        /// Threshold in seconds that was exceeded.
        threshold_secs: u64,
    },
}

/// Heartbeat monitor for detecting stalled agent execution.
///
/// The monitor tracks the time since the last heartbeat pulse and sends
/// events when heartbeats are missed. It runs a background task that
/// periodically checks elapsed time since the last heartbeat.
///
/// # Example
///
/// ```ignore
/// use std::time::Duration;
/// use ralphmacchio::timeout::{HeartbeatMonitor, TimeoutConfig};
///
/// let config = TimeoutConfig::new()
///     .with_heartbeat_interval(Duration::from_secs(1))
///     .with_missed_heartbeats_threshold(3);
///
/// let (monitor, mut receiver) = HeartbeatMonitor::new(config);
/// monitor.start_monitoring();
///
/// // Send heartbeats to indicate progress
/// monitor.pulse();
///
/// // Check for events
/// if let Some(event) = receiver.try_recv().ok() {
///     match event {
///         HeartbeatEvent::Warning { missed, elapsed_secs, remaining_secs } => {
///             println!("Warning: {} missed, {}s elapsed, {}s until stall", missed, elapsed_secs, remaining_secs);
///         }
///         HeartbeatEvent::StallDetected { missed, elapsed_secs, threshold_secs } => {
///             println!("Stall: {} missed, {}s elapsed (threshold: {}s)", missed, elapsed_secs, threshold_secs);
///         }
///     }
/// }
///
/// // Stop monitoring when done
/// monitor.stop();
/// ```
pub struct HeartbeatMonitor {
    /// Configuration for timeout behavior.
    config: TimeoutConfig,
    /// Timestamp of the last heartbeat pulse.
    last_heartbeat: Arc<Mutex<Instant>>,
    /// Channel sender for heartbeat events.
    sender: mpsc::Sender<HeartbeatEvent>,
    /// Flag to signal the background task to stop.
    stop_flag: Arc<AtomicBool>,
    /// Handle to the background monitoring task.
    task_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl HeartbeatMonitor {
    /// Creates a new heartbeat monitor with the given configuration.
    ///
    /// Returns a tuple of the monitor and a receiver for heartbeat events.
    /// The receiver will receive `HeartbeatEvent::Warning` when heartbeats
    /// start being missed, and `HeartbeatEvent::StallDetected` when the
    /// threshold is reached.
    ///
    /// # Arguments
    ///
    /// * `config` - Timeout configuration including heartbeat interval and threshold
    ///
    /// # Returns
    ///
    /// A tuple of (`HeartbeatMonitor`, `mpsc::Receiver<HeartbeatEvent>`)
    pub fn new(config: TimeoutConfig) -> (Self, mpsc::Receiver<HeartbeatEvent>) {
        let (sender, receiver) = mpsc::channel(16);

        let monitor = Self {
            config,
            last_heartbeat: Arc::new(Mutex::new(Instant::now())),
            sender,
            stop_flag: Arc::new(AtomicBool::new(false)),
            task_handle: Arc::new(Mutex::new(None)),
        };

        (monitor, receiver)
    }

    /// Records a heartbeat pulse, updating the last heartbeat timestamp.
    ///
    /// Call this method periodically to indicate that the agent is still
    /// making progress. If pulses stop arriving, the monitor will detect
    /// the stall and send appropriate events.
    pub async fn pulse(&self) {
        let mut last = self.last_heartbeat.lock().await;
        *last = Instant::now();
    }

    /// Starts the background monitoring task.
    ///
    /// The task waits for an initial grace period (to allow agent startup),
    /// then periodically checks the elapsed time since the last heartbeat
    /// and sends events when heartbeats are missed:
    ///
    /// - `HeartbeatEvent::Warning` is sent after `missed_heartbeats_threshold - 1`
    ///   consecutive missed heartbeats.
    /// - `HeartbeatEvent::StallDetected` is sent after `missed_heartbeats_threshold`
    ///   consecutive missed heartbeats.
    ///
    /// The task continues running until `stop()` is called.
    pub async fn start_monitoring(&self) {
        // Reset state
        self.stop_flag.store(false, Ordering::SeqCst);
        {
            let mut last = self.last_heartbeat.lock().await;
            *last = Instant::now();
        }

        let config = self.config.clone();
        let last_heartbeat = Arc::clone(&self.last_heartbeat);
        let sender = self.sender.clone();
        let stop_flag = Arc::clone(&self.stop_flag);

        let handle = tokio::spawn(async move {
            let interval = config.heartbeat_interval;
            let threshold = config.missed_heartbeats_threshold;
            let grace_period = config.startup_grace_period;
            let mut last_warning_sent: Option<u32> = None;

            // Wait for the initial grace period before starting monitoring.
            // This allows time for agent startup, MCP server initialization,
            // and the first API call to complete.
            if !grace_period.is_zero() {
                tokio::time::sleep(grace_period).await;

                // Check if we were stopped during the grace period
                if stop_flag.load(Ordering::SeqCst) {
                    return;
                }

                // Reset the heartbeat timestamp after grace period
                // so that the first check starts fresh
                {
                    let mut last = last_heartbeat.lock().await;
                    *last = Instant::now();
                }
            }

            loop {
                if stop_flag.load(Ordering::SeqCst) {
                    break;
                }

                tokio::time::sleep(interval).await;

                if stop_flag.load(Ordering::SeqCst) {
                    break;
                }

                let elapsed = {
                    let last = last_heartbeat.lock().await;
                    last.elapsed()
                };

                // Calculate number of missed heartbeats
                let missed = (elapsed.as_secs_f64() / interval.as_secs_f64()).floor() as u32;
                let elapsed_secs = elapsed.as_secs();
                let threshold_secs = interval.as_secs() * threshold as u64;

                if missed >= threshold {
                    // Stall detected
                    let _ = sender
                        .send(HeartbeatEvent::StallDetected {
                            missed,
                            elapsed_secs,
                            threshold_secs,
                        })
                        .await;
                    // Reset warning tracking after stall
                    last_warning_sent = None;
                } else if missed >= threshold.saturating_sub(1) && missed > 0 {
                    // Warning threshold reached (threshold - 1 missed beats)
                    // Only send warning if we haven't sent one for this level
                    if last_warning_sent != Some(missed) {
                        let remaining_secs = threshold_secs.saturating_sub(elapsed_secs);
                        let _ = sender
                            .send(HeartbeatEvent::Warning {
                                missed,
                                elapsed_secs,
                                remaining_secs,
                            })
                            .await;
                        last_warning_sent = Some(missed);
                    }
                } else if missed == 0 {
                    // Reset warning tracking when heartbeats resume
                    last_warning_sent = None;
                }
            }
        });

        let mut task = self.task_handle.lock().await;
        *task = Some(handle);
    }

    /// Stops the background monitoring task.
    ///
    /// This method signals the background task to stop and waits for it
    /// to complete. After calling this method, `start_monitoring()` can
    /// be called again to restart monitoring.
    pub async fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);

        let handle = {
            let mut task = self.task_handle.lock().await;
            task.take()
        };

        if let Some(handle) = handle {
            let _ = handle.await;
        }
    }

    /// Returns a reference to the timeout configuration.
    pub fn config(&self) -> &TimeoutConfig {
        &self.config
    }

    /// Returns true if the monitoring task is currently running.
    pub async fn is_running(&self) -> bool {
        let task = self.task_handle.lock().await;
        if let Some(handle) = task.as_ref() {
            !handle.is_finished()
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn test_config() -> TimeoutConfig {
        TimeoutConfig::new()
            .with_heartbeat_interval(Duration::from_millis(50))
            .with_missed_heartbeats_threshold(3)
            .with_startup_grace_period(Duration::ZERO) // No grace period for fast tests
    }

    #[tokio::test]
    async fn test_monitor_creation() {
        let config = test_config();
        let (monitor, _receiver) = HeartbeatMonitor::new(config.clone());

        assert_eq!(
            monitor.config().heartbeat_interval,
            config.heartbeat_interval
        );
        assert_eq!(
            monitor.config().missed_heartbeats_threshold,
            config.missed_heartbeats_threshold
        );
    }

    #[tokio::test]
    async fn test_pulse_updates_timestamp() {
        let config = test_config();
        let (monitor, _receiver) = HeartbeatMonitor::new(config);

        // Record initial timestamp
        let initial = {
            let last = monitor.last_heartbeat.lock().await;
            *last
        };

        // Wait a bit and pulse
        tokio::time::sleep(Duration::from_millis(10)).await;
        monitor.pulse().await;

        // Verify timestamp was updated
        let updated = {
            let last = monitor.last_heartbeat.lock().await;
            *last
        };

        assert!(updated > initial);
    }

    #[tokio::test]
    async fn test_no_events_with_regular_heartbeats() {
        let config = TimeoutConfig::new()
            .with_heartbeat_interval(Duration::from_millis(50))
            .with_missed_heartbeats_threshold(3)
            .with_startup_grace_period(Duration::ZERO);

        let (monitor, mut receiver) = HeartbeatMonitor::new(config);
        monitor.start_monitoring().await;

        // Send heartbeats regularly
        for _ in 0..5 {
            tokio::time::sleep(Duration::from_millis(30)).await;
            monitor.pulse().await;
        }

        // Check no events were sent
        let event = receiver.try_recv();
        assert!(event.is_err());

        monitor.stop().await;
    }

    #[tokio::test]
    async fn test_warning_after_threshold_minus_one_missed() {
        let config = TimeoutConfig::new()
            .with_heartbeat_interval(Duration::from_millis(50))
            .with_missed_heartbeats_threshold(3)
            .with_startup_grace_period(Duration::ZERO);

        let (monitor, mut receiver) = HeartbeatMonitor::new(config);
        monitor.start_monitoring().await;

        // Wait for threshold - 1 intervals (2 missed)
        // Need to wait for the monitor to check, which happens after one interval
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Should have received a warning
        let event = tokio::time::timeout(Duration::from_millis(100), receiver.recv()).await;

        monitor.stop().await;

        assert!(event.is_ok());
        let event = event.unwrap();
        assert!(event.is_some());
        assert!(matches!(event.unwrap(), HeartbeatEvent::Warning { .. }));
    }

    #[tokio::test]
    async fn test_stall_detected_after_threshold_missed() {
        let config = TimeoutConfig::new()
            .with_heartbeat_interval(Duration::from_millis(50))
            .with_missed_heartbeats_threshold(3)
            .with_startup_grace_period(Duration::ZERO);

        let (monitor, mut receiver) = HeartbeatMonitor::new(config);
        monitor.start_monitoring().await;

        // Wait for threshold intervals (3 missed)
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Stop monitoring to stop event generation
        monitor.stop().await;

        // Collect events that were sent
        let mut events = Vec::new();
        while let Ok(event) = tokio::time::timeout(Duration::from_millis(50), receiver.recv()).await
        {
            if let Some(e) = event {
                events.push(e);
            } else {
                break;
            }
        }

        // Should have received a stall detection
        assert!(events
            .iter()
            .any(|e| matches!(e, HeartbeatEvent::StallDetected { .. })));
    }

    #[tokio::test]
    async fn test_stop_terminates_task() {
        let config = test_config();
        let (monitor, _receiver) = HeartbeatMonitor::new(config);

        monitor.start_monitoring().await;
        assert!(monitor.is_running().await);

        monitor.stop().await;

        // Give it a moment to fully stop
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(!monitor.is_running().await);
    }

    #[tokio::test]
    async fn test_restart_monitoring() {
        let config = test_config();
        let (monitor, _receiver) = HeartbeatMonitor::new(config);

        // Start and stop
        monitor.start_monitoring().await;
        assert!(monitor.is_running().await);
        monitor.stop().await;

        // Give it a moment to fully stop
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(!monitor.is_running().await);

        // Start again
        monitor.start_monitoring().await;
        assert!(monitor.is_running().await);
        monitor.stop().await;
    }

    #[tokio::test]
    async fn test_heartbeat_event_equality() {
        let warning1 = HeartbeatEvent::Warning {
            missed: 2,
            elapsed_secs: 100,
            remaining_secs: 50,
        };
        let warning2 = HeartbeatEvent::Warning {
            missed: 2,
            elapsed_secs: 100,
            remaining_secs: 50,
        };
        let warning3 = HeartbeatEvent::Warning {
            missed: 3,
            elapsed_secs: 150,
            remaining_secs: 0,
        };
        let stall = HeartbeatEvent::StallDetected {
            missed: 3,
            elapsed_secs: 150,
            threshold_secs: 150,
        };

        assert_eq!(warning1, warning2);
        assert_ne!(warning1, warning3);
        assert_ne!(warning1, stall);
    }

    #[tokio::test]
    async fn test_heartbeat_event_debug() {
        let warning = HeartbeatEvent::Warning {
            missed: 2,
            elapsed_secs: 100,
            remaining_secs: 50,
        };
        let stall = HeartbeatEvent::StallDetected {
            missed: 3,
            elapsed_secs: 150,
            threshold_secs: 150,
        };

        let warning_debug = format!("{:?}", warning);
        let stall_debug = format!("{:?}", stall);

        assert!(warning_debug.contains("Warning"));
        assert!(warning_debug.contains("missed: 2"));
        assert!(warning_debug.contains("elapsed_secs: 100"));
        assert!(stall_debug.contains("StallDetected"));
        assert!(stall_debug.contains("missed: 3"));
        assert!(stall_debug.contains("threshold_secs: 150"));
    }

    #[tokio::test]
    async fn test_heartbeat_event_clone() {
        let warning = HeartbeatEvent::Warning {
            missed: 2,
            elapsed_secs: 100,
            remaining_secs: 50,
        };
        let cloned = warning.clone();
        assert_eq!(warning, cloned);
    }

    #[tokio::test]
    async fn test_config_accessor() {
        let config = TimeoutConfig::new()
            .with_heartbeat_interval(Duration::from_secs(5))
            .with_missed_heartbeats_threshold(10)
            .with_startup_grace_period(Duration::from_secs(60));

        let (monitor, _receiver) = HeartbeatMonitor::new(config);

        assert_eq!(monitor.config().heartbeat_interval, Duration::from_secs(5));
        assert_eq!(monitor.config().missed_heartbeats_threshold, 10);
        assert_eq!(
            monitor.config().startup_grace_period,
            Duration::from_secs(60)
        );
    }

    #[tokio::test]
    async fn test_pulse_resets_missed_count() {
        let config = TimeoutConfig::new()
            .with_heartbeat_interval(Duration::from_millis(50))
            .with_missed_heartbeats_threshold(3)
            .with_startup_grace_period(Duration::ZERO);

        let (monitor, mut receiver) = HeartbeatMonitor::new(config);
        monitor.start_monitoring().await;

        // Wait almost to warning threshold
        tokio::time::sleep(Duration::from_millis(80)).await;

        // Pulse to reset
        monitor.pulse().await;

        // Wait a bit more (should not reach threshold from reset point)
        tokio::time::sleep(Duration::from_millis(80)).await;

        // Pulse again
        monitor.pulse().await;

        // No events should have been sent
        let event = receiver.try_recv();
        assert!(event.is_err());

        monitor.stop().await;
    }

    #[tokio::test]
    async fn test_warning_before_stall() {
        let config = TimeoutConfig::new()
            .with_heartbeat_interval(Duration::from_millis(30))
            .with_missed_heartbeats_threshold(3)
            .with_startup_grace_period(Duration::ZERO);

        let (monitor, mut receiver) = HeartbeatMonitor::new(config);
        monitor.start_monitoring().await;

        // Wait long enough for both warning and stall
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Stop monitoring to stop event generation
        monitor.stop().await;

        // Collect events that were sent
        let mut events = Vec::new();
        while let Ok(event) = tokio::time::timeout(Duration::from_millis(50), receiver.recv()).await
        {
            if let Some(e) = event {
                events.push(e);
            } else {
                break;
            }
        }

        // Should have warning(s) and stall detection
        let has_warning = events
            .iter()
            .any(|e| matches!(e, HeartbeatEvent::Warning { .. }));
        let has_stall = events
            .iter()
            .any(|e| matches!(e, HeartbeatEvent::StallDetected { .. }));

        assert!(has_warning, "Expected warning event");
        assert!(has_stall, "Expected stall detection event");
    }

    #[tokio::test]
    async fn test_is_running_before_start() {
        let config = test_config();
        let (monitor, _receiver) = HeartbeatMonitor::new(config);

        assert!(!monitor.is_running().await);
    }

    #[tokio::test]
    async fn test_grace_period_delays_monitoring() {
        // Configure a short grace period for testing
        let config = TimeoutConfig::new()
            .with_heartbeat_interval(Duration::from_millis(30))
            .with_missed_heartbeats_threshold(2)
            .with_startup_grace_period(Duration::from_millis(100));

        let (monitor, mut receiver) = HeartbeatMonitor::new(config);
        monitor.start_monitoring().await;

        // Wait for less than the grace period
        // (no heartbeat checks should happen yet)
        tokio::time::sleep(Duration::from_millis(80)).await;

        // No events should be sent during grace period
        let event = receiver.try_recv();
        assert!(
            event.is_err(),
            "Should not receive events during grace period"
        );

        // Wait for grace period to complete plus some heartbeat intervals
        // Grace period: 100ms, then heartbeat checks at 30ms intervals
        // After grace period, need 2 missed heartbeats (60ms) to trigger stall
        tokio::time::sleep(Duration::from_millis(120)).await;

        monitor.stop().await;

        // Now we should have received stall detection
        let mut events = Vec::new();
        while let Ok(event) = tokio::time::timeout(Duration::from_millis(50), receiver.recv()).await
        {
            if let Some(e) = event {
                events.push(e);
            } else {
                break;
            }
        }

        // Should have received a stall detection after grace period
        assert!(
            events
                .iter()
                .any(|e| matches!(e, HeartbeatEvent::StallDetected { .. })),
            "Expected stall detection after grace period"
        );
    }

    #[tokio::test]
    async fn test_stop_during_grace_period() {
        let config = TimeoutConfig::new()
            .with_heartbeat_interval(Duration::from_millis(50))
            .with_missed_heartbeats_threshold(3)
            .with_startup_grace_period(Duration::from_millis(200));

        let (monitor, mut receiver) = HeartbeatMonitor::new(config);
        monitor.start_monitoring().await;

        // Wait a bit then stop during grace period
        tokio::time::sleep(Duration::from_millis(50)).await;
        monitor.stop().await;

        // Give it a moment to fully stop
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(!monitor.is_running().await);

        // No events should have been sent
        let event = receiver.try_recv();
        assert!(
            event.is_err(),
            "Should not receive events when stopped during grace period"
        );
    }
}
