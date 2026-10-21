//! One bounded mailbox and one worker per gesture. UI reads never wait for I/O.
use parking_lot::Mutex;
use std::{
    io,
    sync::{mpsc, Arc},
    thread,
    time::Duration,
};

pub(super) struct Poller<T> {
    latest: Arc<Mutex<Option<io::Result<T>>>>,
    stop: mpsc::Sender<()>,
}

impl<T: Send + 'static> Poller<T> {
    pub(super) fn spawn(
        mut sample: impl FnMut() -> io::Result<(T, bool)> + Send + 'static,
    ) -> io::Result<Self> {
        let latest = Arc::new(Mutex::new(None));
        let output = Arc::clone(&latest);
        let (stop, stopped) = mpsc::channel();
        thread::Builder::new()
            .name("tab-drag-pointer".into())
            .spawn(move || {
                loop {
                    if stopped.try_recv().is_ok() {
                        break;
                    }
                    let result = sample();
                    let finished =
                        result.as_ref().map_or(true, |(_, finished)| *finished);
                    // Replace a stale sample rather than queueing work on every mouse event.
                    *output.lock() = Some(result.map(|(value, _)| value));
                    if finished
                        || stopped.recv_timeout(Duration::from_millis(16))
                            != Err(mpsc::RecvTimeoutError::Timeout)
                    {
                        break;
                    }
                }
            })?;
        Ok(Self { latest, stop })
    }

    pub(super) fn take(&self) -> Option<io::Result<T>> {
        self.latest.lock().take()
    }
}

impl<T> Drop for Poller<T> {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        // Never join from the UI. The native owner cancels outstanding socket I/O.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slow_server_does_not_block_or_queue_ui_samples() {
        let (started, ready) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let poller = Poller::spawn(move || {
            started.send(()).unwrap();
            blocked.recv().unwrap();
            Ok((42, true))
        })
        .unwrap();
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        for _ in 0..1000 {
            assert!(poller.take().is_none());
        }
        assert!(
            ready.try_recv().is_err(),
            "UI reads must not schedule more requests"
        );
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(result) = poller.take() {
                assert_eq!(result.unwrap(), 42);
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            thread::yield_now();
        }
    }

    #[test]
    fn worker_errors_reach_the_ui() {
        let poller =
            Poller::<()>::spawn(|| Err(io::Error::other("lost X server"))).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(result) = poller.take() {
                assert_eq!(result.unwrap_err().to_string(), "lost X server");
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            thread::yield_now();
        }
    }
}
