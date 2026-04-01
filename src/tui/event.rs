use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

pub enum AppEvent {
    Key(KeyEvent),
    Quit,
    Resize(u16, u16),
    Tick,
}

pub fn map_event(event: Event) -> Option<AppEvent> {
    match event {
        Event::Key(key) => {
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                Some(AppEvent::Quit)
            } else {
                Some(AppEvent::Key(key))
            }
        }
        Event::Resize(w, h) => Some(AppEvent::Resize(w, h)),
        _ => None,
    }
}
