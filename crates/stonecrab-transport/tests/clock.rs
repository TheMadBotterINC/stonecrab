//! `SystemClock` against Tokio's clock, with paused time so nothing really sleeps.
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use stonecrab_core::{Clock, Deadline};
    use stonecrab_testkit::poll_once;
    use stonecrab_transport::SystemClock;

    #[tokio::test(start_paused = true)]
    async fn now_follows_paused_tokio_time_so_deadlines_agree() {
        let clock = SystemClock::default();
        let start = clock.now();
        assert_eq!(
            start,
            tokio::time::Instant::now().into_std(),
            "now() is Tokio's instant"
        );
        let deadline = Deadline::after(start, Duration::from_secs(5)).expect("representable");
        assert!(!deadline.is_elapsed(clock.now()), "not elapsed yet");
        tokio::time::advance(Duration::from_secs(5)).await;
        assert_eq!(
            clock.now().duration_since(start),
            Duration::from_secs(5),
            "advance moved the clock"
        );
        assert!(
            deadline.is_elapsed(clock.now()),
            "the deadline agrees with the clock"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn sleep_until_waits_for_tokio_time_and_resolves_immediately_when_past() {
        let clock = SystemClock::default();
        let target = clock.now() + Duration::from_secs(30);
        let mut sleep = clock.sleep_until(target);
        assert!(
            poll_once(&mut sleep).is_pending(),
            "a future deadline pends"
        );
        sleep.await.expect("sleep resolves");
        assert!(clock.now() >= target, "time reached the target");

        clock
            .sleep_until(clock.now())
            .await
            .expect("now resolves at once");
        clock
            .sleep_until(target)
            .await
            .expect("a past deadline resolves at once");
    }

    #[tokio::test(start_paused = true)]
    async fn a_dropped_sleep_has_no_effect() {
        let clock = SystemClock::default();
        let before = clock.now();
        let mut sleep = clock.sleep_until(before + Duration::from_secs(60));
        assert!(poll_once(&mut sleep).is_pending(), "pending");
        drop(sleep);
        assert_eq!(clock.now(), before, "dropping advanced nothing");
    }

    #[test]
    fn wall_now_is_the_system_wall_clock() {
        let clock = SystemClock::default();
        let first = clock.wall_now();
        assert!(first > SystemTime::UNIX_EPOCH, "after the epoch");
        let dynamic: &dyn Clock = &clock;
        assert!(
            dynamic.wall_now() > SystemTime::UNIX_EPOCH,
            "readable through dyn"
        );
    }
}
