use std::fmt;
use std::sync::mpsc::Sender;

use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing::span;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

/// A tracing layer that sends formatted log messages to an mpsc channel.
/// The install progress screen reads from the receiver end.
pub struct ChannelLayer {
    tx: Sender<String>,
}

impl ChannelLayer {
    pub fn new(tx: Sender<String>) -> Self {
        Self { tx }
    }
}

impl<S: Subscriber> Layer<S> for ChannelLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let level = metadata.level();

        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let msg = if let Some(message) = visitor.message {
            message
        } else {
            format!("{}: {}", metadata.target(), visitor.fields.join(", "))
        };

        let formatted = match *level {
            tracing::Level::ERROR => format!("[ERROR] {msg}"),
            tracing::Level::WARN => format!("[WARN] {msg}"),
            _ => msg,
        };

        let _ = self.tx.send(formatted);
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
