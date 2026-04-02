use std::fmt;

use crossbeam_channel::Sender;
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

/// Tracing layer that sends log messages to a crossbeam channel for the Slint UI.
pub struct UiLogLayer {
    tx: Sender<(String, i32)>,
}

impl UiLogLayer {
    pub fn new(tx: Sender<(String, i32)>) -> Self {
        Self { tx }
    }
}

impl<S: Subscriber> Layer<S> for UiLogLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let level = match *event.metadata().level() {
            tracing::Level::TRACE => 0,
            tracing::Level::DEBUG => 1,
            tracing::Level::INFO => 2,
            tracing::Level::WARN => 3,
            tracing::Level::ERROR => 4,
        };

        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let prefix = match level {
            4 => "ERROR",
            3 => "WARN ",
            1 => "DEBUG",
            0 => "TRACE",
            _ => "INFO ",
        };

        let msg = visitor.message.unwrap_or_default();
        let line = format!("[{prefix}] {msg}");

        let _ = self.tx.try_send((line, level));
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        }
    }
}
