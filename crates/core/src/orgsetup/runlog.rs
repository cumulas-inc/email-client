//! The run log: every sign-in attempt and Microsoft call organization setup
//! makes, in memory only, for "Copy setup details".

/// What organization setup did, kept after its session ends (so "Copy
/// setup details" works in every state) until the next sign-in, and never
/// written anywhere. Nothing is logged yet.
#[derive(Debug, Default)]
pub struct RunLog {
    _private: (),
}

/// The "Copy setup details" text; empty while nothing was logged.
pub fn details_text(_log: &RunLog) -> String {
    String::new()
}
