//! Noticing that the computer woke up from suspend.
//!
//! Tokio's timers run on the monotonic clock, which stands still while the
//! computer sleeps: a timer set before suspend fires late after resume, and
//! connections that died in the meantime go unnoticed for minutes. The wall
//! clock keeps counting, so a check that finds it far ahead of the monotonic
//! time slept means the computer was asleep.

use std::time::{Duration, SystemTime};

use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// How often the clocks are compared.
pub(crate) const CHECK_EVERY: Duration = Duration::from_secs(30);
/// How far the wall clock may run ahead of the monotonic clock before it
/// counts as a resume (also absorbs clock adjustments).
pub(crate) const JUMP: Duration = Duration::from_secs(90);

/// Calls `on_resume` with the time spent asleep whenever the wall clock
/// (`clock`) jumps ahead, until `cancel` fires.
pub(crate) async fn watch_for_resume(
    clock: impl Fn() -> SystemTime,
    cancel: CancellationToken,
    on_resume: impl Fn(Duration),
) {
    let mut wall = clock();
    let mut monotonic = Instant::now();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(CHECK_EVERY) => {}
        }
        let (now_wall, now_monotonic) = (clock(), Instant::now());
        // A wall clock set backwards gives zero.
        let wall_elapsed = now_wall.duration_since(wall).unwrap_or_default();
        let slept = now_monotonic - monotonic;
        if let Some(asleep) = wall_elapsed.checked_sub(slept)
            && asleep > JUMP
        {
            on_resume(asleep);
        }
        (wall, monotonic) = (now_wall, now_monotonic);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn reports_a_jump_of_the_wall_clock() {
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        // The wall clock follows the paused monotonic clock plus an offset
        // the test moves forward to simulate sleep.
        let offset = Arc::new(Mutex::new(Duration::ZERO));
        let origin = Instant::now();
        let clock = {
            let offset = Arc::clone(&offset);
            move || start + (Instant::now() - origin) + *offset.lock().unwrap()
        };
        let resumes = Arc::new(Mutex::new(Vec::new()));
        let cancel = CancellationToken::new();
        let task = tokio::spawn({
            let resumes = Arc::clone(&resumes);
            let cancel = cancel.clone();
            async move {
                watch_for_resume(clock, cancel, |asleep| {
                    resumes.lock().unwrap().push(asleep.as_secs())
                })
                .await
            }
        });

        // Ordinary ticks, and a small adjustment, are not a resume.
        tokio::time::sleep(CHECK_EVERY * 3 + Duration::from_secs(1)).await;
        *offset.lock().unwrap() += Duration::from_secs(60);
        tokio::time::sleep(CHECK_EVERY).await;
        assert!(resumes.lock().unwrap().is_empty());

        // An hour asleep is reported once.
        *offset.lock().unwrap() += Duration::from_secs(3600);
        tokio::time::sleep(CHECK_EVERY * 2).await;
        assert_eq!(*resumes.lock().unwrap(), [3600]);

        // A wall clock set backwards is ignored.
        *offset.lock().unwrap() = Duration::ZERO;
        tokio::time::sleep(CHECK_EVERY * 2).await;
        assert_eq!(resumes.lock().unwrap().len(), 1);

        cancel.cancel();
        task.await.unwrap();
    }
}
