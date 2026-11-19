//! The mark of a tenant that is busy with a request.

/// Holds the mark of a busy tenant for the duration of a request.
///
/// The mark is given back when the guard is dropped, also when the future
/// that owns it is cancelled. A client that hangs up in the middle of a
/// request must not leave the tenant busy forever: a busy tenant is never
/// taken away by the scheduler, so a single cancelled request would keep the
/// memory of the model booked and no bigger model would fit any more.
pub(crate) struct ActiveRequest {
    release: Option<Box<dyn FnOnce() + Send>>,
}

impl ActiveRequest {
    /// Creates a guard that calls `release` when it is dropped.
    pub(crate) fn new(release: impl FnOnce() + Send + 'static) -> Self {
        Self {
            release: Some(Box::new(release)),
        }
    }
}

impl Drop for ActiveRequest {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// The mark is given back when the guard is dropped.
    #[test]
    fn the_mark_is_given_back_on_drop() {
        let released = Arc::new(AtomicUsize::new(0));
        let counter = released.clone();
        {
            let _guard = ActiveRequest::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            });
            assert_eq!(released.load(Ordering::SeqCst), 0, "the guard is alive");
        }
        assert_eq!(released.load(Ordering::SeqCst), 1, "the mark is given back");
    }

    /// The mark is given back exactly once.
    #[test]
    fn the_mark_is_given_back_once() {
        let released = Arc::new(AtomicUsize::new(0));
        let counter = released.clone();
        let guard = ActiveRequest::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        drop(guard);
        assert_eq!(released.load(Ordering::SeqCst), 1);
    }
}
