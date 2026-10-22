//! Prometheus metrics, served as `GET /metrics` on the RPC listener.
//!
//! Uses the `metrics` facade with a Prometheus exporter, like Summit, so the
//! node's Prometheus Agent can scrape this endpoint alongside Reth's and
//! Summit's. Metric names:
//!
//! | Metric                                       | Type    | Meaning                                                   |
//! | -------------------------------------------- | ------- | --------------------------------------------------------- |
//! | `checkpointer_last_checkpoint_epoch`         | gauge   | epoch of the newest completed checkpoint                  |
//! | `checkpointer_last_checkpoint_block`         | gauge   | Reth block of the newest completed checkpoint             |
//! | `checkpointer_last_checkpoint_timestamp_seconds` | gauge | Unix time the newest checkpoint completed              |
//! | `checkpointer_checkpoints_total{result}`     | counter | checkpoint attempts, `result="ok"` or `"error"`           |
//! | `checkpointer_checkpoint_duration_seconds`   | histogram | wall time of successful checkpoint creation             |
//! | `checkpointer_monitor_errors_total{kind}`    | counter | monitor-loop failures by kind (`rpc`, `http`, `other`)    |
//! | `checkpointer_snapshot_requests_total{kind,status}` | counter | `/snapshots` serving by `manifest`/`archive` and HTTP status |
//! | `checkpointer_build_info{version}`           | gauge   | always 1; carries the version label                       |
//!
//! Alerting on `time() - checkpointer_last_checkpoint_timestamp_seconds`
//! growing beyond the expected epoch length catches a stalled checkpointer
//! whether or not it is logging errors.

use metrics::{counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::{
    sync::OnceLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Install the global recorder. Idempotent: later calls return the existing
/// handle, so tests and library users can call it freely.
pub fn init() -> PrometheusHandle {
    HANDLE
        .get_or_init(|| {
            let handle =
                PrometheusBuilder::new().install_recorder().expect("install Prometheus recorder");
            describe_gauge!(
                "checkpointer_last_checkpoint_epoch",
                "Epoch of the newest completed checkpoint"
            );
            describe_gauge!(
                "checkpointer_last_checkpoint_block",
                "Reth block number of the newest completed checkpoint"
            );
            describe_gauge!(
                "checkpointer_last_checkpoint_timestamp_seconds",
                "Unix time at which the newest checkpoint completed"
            );
            describe_counter!(
                "checkpointer_checkpoints_total",
                "Checkpoint attempts by result (ok, error)"
            );
            describe_histogram!(
                "checkpointer_checkpoint_duration_seconds",
                "Wall-clock duration of successful checkpoint creation"
            );
            describe_counter!(
                "checkpointer_monitor_errors_total",
                "Monitor loop failures by kind (rpc, http, other)"
            );
            describe_counter!(
                "checkpointer_snapshot_requests_total",
                "Snapshot HTTP requests by kind (manifest, archive) and status"
            );
            describe_gauge!("checkpointer_build_info", "Always 1; carries the version label");
            gauge!("checkpointer_build_info", "version" => env!("CARGO_PKG_VERSION")).set(1.0);
            handle
        })
        .clone()
}

/// Render the current metrics in Prometheus text format.
pub fn render() -> String {
    init().render()
}

/// Seed the "last checkpoint" gauges from persisted state at startup, so a
/// restart does not report zero until the next checkpoint.
pub fn record_existing_checkpoint(epoch: Option<u64>, block: Option<u64>) {
    if let Some(epoch) = epoch {
        gauge!("checkpointer_last_checkpoint_epoch").set(epoch as f64);
    }
    if let Some(block) = block {
        gauge!("checkpointer_last_checkpoint_block").set(block as f64);
    }
}

pub fn record_checkpoint_ok(epoch: u64, block: u64, duration: Duration) {
    gauge!("checkpointer_last_checkpoint_epoch").set(epoch as f64);
    gauge!("checkpointer_last_checkpoint_block").set(block as f64);
    gauge!("checkpointer_last_checkpoint_timestamp_seconds").set(unix_now());
    counter!("checkpointer_checkpoints_total", "result" => "ok").increment(1);
    histogram!("checkpointer_checkpoint_duration_seconds").record(duration.as_secs_f64());
}

pub fn record_checkpoint_error() {
    counter!("checkpointer_checkpoints_total", "result" => "error").increment(1);
}

pub fn record_monitor_error(kind: &'static str) {
    counter!("checkpointer_monitor_errors_total", "kind" => kind).increment(1);
}

pub fn record_snapshot_request(kind: &'static str, status: u16) {
    counter!(
        "checkpointer_snapshot_requests_total",
        "kind" => kind,
        "status" => status.to_string()
    )
    .increment(1);
}

fn unix_now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_recorded_values() {
        init();
        record_existing_checkpoint(Some(41), Some(409_999));
        record_checkpoint_ok(42, 420_000, Duration::from_secs(3));
        record_checkpoint_error();
        record_monitor_error("rpc");
        record_snapshot_request("manifest", 200);
        record_snapshot_request("archive", 404);
        let out = render();
        assert!(out.contains("checkpointer_last_checkpoint_epoch 42"), "{out}");
        assert!(out.contains("checkpointer_last_checkpoint_block 420000"), "{out}");
        assert!(out.contains("checkpointer_checkpoints_total{result=\"ok\"} 1"), "{out}");
        assert!(out.contains("checkpointer_checkpoints_total{result=\"error\"} 1"), "{out}");
        assert!(out.contains("checkpointer_monitor_errors_total{kind=\"rpc\"} 1"), "{out}");
        assert!(
            out.contains(
                "checkpointer_snapshot_requests_total{kind=\"manifest\",status=\"200\"} 1"
            ) || out.contains(
                "checkpointer_snapshot_requests_total{status=\"200\",kind=\"manifest\"} 1"
            ),
            "{out}"
        );
        assert!(out.contains("checkpointer_checkpoint_duration_seconds_count 1"), "{out}");
        assert!(out.contains("checkpointer_build_info{version=\""), "{out}");
        assert!(out.contains("# TYPE checkpointer_last_checkpoint_epoch gauge"), "{out}");
    }
}
