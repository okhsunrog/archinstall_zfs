//! Structured metrics collection for install pipeline profiling.
//!
//! Implements a [`tracing_subscriber::Layer`] that intercepts events with
//! `target = "metrics"` and writes them as newline-delimited JSON to a file.
//! Regular log events (info/warn/debug/…) are ignored — only structured
//! metrics events pass through.
//!
//! ## Event format
//!
//! Each line in the output file is a JSON object:
//! ```json
//! {"ts_ms":1712345678123,"event":"phase_start","num":4,"name":"Installing base system"}
//! {"ts_ms":1712345712456,"event":"pkg_download","filename":"glibc-2.39.tar.zst","bytes":9852304,"duration_ms":234,"mirror":"https://mirror.example.org","speed_bps":42101299}
//! {"ts_ms":1712345715789,"event":"batch_install","count":178,"duration_ms":92340}
//! ```
//!
//! ## Usage
//!
//! ```rust,no_run
//! use archinstall_zfs_core::metrics::MetricsLayer;
//! use tracing_subscriber::prelude::*;
//!
//! let layer = MetricsLayer::open("/tmp/archinstall-metrics.jsonl")
//!     .expect("failed to open metrics file");
//!
//! tracing_subscriber::registry()
//!     .with(layer)
//!     .init();
//! ```
//!
//! Emit a metrics event from anywhere in the install pipeline:
//! ```rust
//! tracing::info!(
//!     target: "metrics",
//!     event = "phase_start",
//!     num = 4u32,
//!     name = "Installing base system",
//! );
//! ```

use std::fmt;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

// ── MetricsLayer ──────────────────────────────────────

/// A tracing [`Layer`] that writes structured metrics events to a
/// newline-delimited JSON (JSONL) file.
///
/// Only events with `target == "metrics"` are captured; all others are
/// passed through unchanged.
pub struct MetricsLayer {
    file: Mutex<File>,
}

impl MetricsLayer {
    /// Open `path` for writing and return a new [`MetricsLayer`]. The file
    /// is created or truncated.
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let file = File::create(path)?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }
}

impl<S: Subscriber> Layer<S> for MetricsLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        // Only process metrics-targeted events.
        if event.metadata().target() != "metrics" {
            return;
        }

        let ts_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let mut visitor = JsonVisitor::default();
        event.record(&mut visitor);

        // Build a simple JSON object from the collected fields.
        // We don't pull in serde for this — just hand-craft the JSON string.
        let mut json = format!("{{\"ts_ms\":{ts_ms}");
        for (k, v) in &visitor.fields {
            json.push_str(&format!(",{k}:{v}"));
        }
        json.push_str("}\n");

        if let Ok(mut f) = self.file.lock() {
            let _ = f.write_all(json.as_bytes());
        }
    }
}

// ── JsonVisitor ───────────────────────────────────────

/// A tracing [`Visit`] implementation that collects all field name/value
/// pairs as pre-serialised JSON fragments.
#[derive(Default)]
struct JsonVisitor {
    /// `(json_key_string, json_value_string)` pairs.
    fields: Vec<(String, String)>,
}

impl Visit for JsonVisitor {
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields.push((quote(field.name()), value.to_string()));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields.push((quote(field.name()), value.to_string()));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.fields.push((quote(field.name()), value.to_string()));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields.push((quote(field.name()), value.to_string()));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields.push((quote(field.name()), json_string(value)));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.fields
            .push((quote(field.name()), json_string(&format!("{value:?}"))));
    }
}

/// Wrap a string in JSON double-quotes, escaping special characters.
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Wrap a key in JSON double-quotes.
fn quote(s: &str) -> String {
    format!("\"{}\"", s)
}
