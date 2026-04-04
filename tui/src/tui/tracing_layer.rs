use std::fmt;

use tokio::sync::mpsc::UnboundedSender;
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

/// A tracing layer that sends (message, level) pairs to a tokio mpsc channel.
/// Level: 0=trace, 1=debug, 2=info, 3=warn, 4=error
pub struct ChannelLayer {
    tx: UnboundedSender<(String, i32)>,
}

impl ChannelLayer {
    pub fn new(tx: UnboundedSender<(String, i32)>) -> Self {
        Self { tx }
    }
}

impl<S: Subscriber> Layer<S> for ChannelLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let level = match *metadata.level() {
            tracing::Level::TRACE => 0,
            tracing::Level::DEBUG => 1,
            tracing::Level::INFO => 2,
            tracing::Level::WARN => 3,
            tracing::Level::ERROR => 4,
        };

        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let msg = if let Some(message) = visitor.message {
            message
        } else {
            format!("{}: {}", metadata.target(), visitor.fields.join(", "))
        };

        let prefix = match level {
            4 => "ERROR",
            3 => "WARN ",
            1 => "DEBUG",
            0 => "TRACE",
            _ => "INFO ",
        };

        let formatted = format!("[{prefix}] {msg}");
        let _ = self.tx.send((formatted, level));
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
    fields: Vec<String>,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        } else {
            self.fields.push(format!("{}={:?}", field.name(), value));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.push(format!("{}={}", field.name(), value));
        }
    }
}
