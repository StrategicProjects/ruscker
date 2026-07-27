//! In-memory ring buffer of recent Ruscker log lines (#100).
//!
//! A `tracing` layer (wired in `ruscker-cli`) pushes each formatted log
//! line here; the admin "Logs" tab reads a snapshot and polls for new
//! lines. It's a bounded *recent tail* — lost on restart;
//! journald / `docker logs` / `container-log-path` remain the durable
//! source. No `tracing` dependency lives here on purpose: the writer
//! that feeds it is owned by the CLI.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Monotonic, gap-tolerant line sequence — lets a polling client ask
/// "anything after seq N?" even though old lines are evicted.
struct Inner {
    lines: VecDeque<(u64, String)>,
    next_seq: u64,
    cap: usize,
}

/// A cheap, clonable handle to the shared ring buffer.
#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<Mutex<Inner>>,
}

impl LogBuffer {
    /// New buffer retaining at most `cap` lines.
    pub fn new(cap: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                lines: VecDeque::with_capacity(cap.min(1024)),
                next_seq: 0,
                cap: cap.max(1),
            })),
        }
    }

    /// Acquire the buffer even if an earlier writer panicked while holding
    /// the mutex. This is a best-effort, in-memory log tail: losing the
    /// dashboard and potentially cascading the panic through the process is
    /// worse than continuing from the still structurally valid ring buffer.
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Append one line, evicting the oldest if at capacity.
    pub fn push_line(&self, line: impl Into<String>) {
        // Run caller-provided conversion code before taking the mutex so a
        // custom `Into<String>` panic cannot poison the shared buffer.
        let line = line.into();
        let mut g = self.lock();
        let seq = g.next_seq;
        g.next_seq += 1;
        g.lines.push_back((seq, line));
        while g.lines.len() > g.cap {
            g.lines.pop_front();
        }
    }

    /// Current snapshot, oldest-first — full buffer (e.g. the download
    /// endpoint).
    pub fn snapshot(&self) -> Vec<String> {
        let g = self.lock();
        g.lines.iter().map(|(_, l)| l.clone()).collect()
    }

    /// The last `n` lines (oldest-first within the tail) plus the total
    /// number of buffered lines. Caps the initial `/admin/logs` render so
    /// a long-lived buffer doesn't ship the whole ring on every load
    /// (#200); the polling endpoint then appends what's new.
    pub fn tail(&self, n: usize) -> (Vec<String>, usize) {
        let (lines, total, _) = self.tail_with_cursor(n);
        (lines, total)
    }

    /// The last `n` lines, total buffered lines, and the cursor immediately
    /// after that same snapshot. Taking all three under one lock prevents a
    /// line written during the initial page render from falling between the
    /// rendered tail and the first poll (#1039).
    pub fn tail_with_cursor(&self, n: usize) -> (Vec<String>, usize, u64) {
        let g = self.lock();
        let total = g.lines.len();
        let start = total.saturating_sub(n);
        let lines = g.lines.iter().skip(start).map(|(_, l)| l.clone()).collect();
        (lines, total, g.next_seq)
    }

    /// The next sequence number that will be assigned — a polling client
    /// records this at connect and asks [`Self::since`] for anything past
    /// it.
    pub fn cursor(&self) -> u64 {
        self.lock().next_seq
    }

    /// Lines with `seq >= after`, plus the new cursor. Lines evicted
    /// since `after` are simply absent (a tail, not a guaranteed log).
    pub fn since(&self, after: u64) -> (Vec<String>, u64) {
        let g = self.lock();
        let new: Vec<String> = g
            .lines
            .iter()
            .filter(|(seq, _)| *seq >= after)
            .map(|(_, l)| l.clone())
            .collect();
        (new, g.next_seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_and_evicts_oldest() {
        let b = LogBuffer::new(3);
        for i in 0..5 {
            b.push_line(format!("line {i}"));
        }
        let snap = b.snapshot();
        assert_eq!(snap, vec!["line 2", "line 3", "line 4"], "kept the last 3");
    }

    #[test]
    fn tail_returns_recent_slice_and_total() {
        let b = LogBuffer::new(100);
        for i in 0..10 {
            b.push_line(format!("line {i}"));
        }
        // Fewer than asked → all, total reported.
        let (lines, total) = b.tail(3);
        assert_eq!(lines, vec!["line 7", "line 8", "line 9"]);
        assert_eq!(total, 10);
        // n >= len → everything, no panic.
        let (all, total) = b.tail(100);
        assert_eq!(all.len(), 10);
        assert_eq!(total, 10);
        // Empty buffer.
        let (none, total) = LogBuffer::new(10).tail(5);
        assert!(none.is_empty());
        assert_eq!(total, 0);
    }

    #[test]
    fn tail_cursor_hands_off_to_incremental_poll_without_a_gap() {
        let b = LogBuffer::new(100);
        b.push_line("a");
        b.push_line("b");

        let (tail, total, cursor) = b.tail_with_cursor(1);
        assert_eq!(tail, vec!["b"]);
        assert_eq!(total, 2);

        b.push_line("c");
        let (new, next) = b.since(cursor);
        assert_eq!(new, vec!["c"]);
        assert_eq!(next, cursor + 1);
    }

    #[test]
    fn since_returns_only_new_lines() {
        let b = LogBuffer::new(100);
        b.push_line("a");
        b.push_line("b");
        let cursor = b.cursor(); // after a, b
        b.push_line("c");
        let (new, next) = b.since(cursor);
        assert_eq!(new, vec!["c"], "only the line after the cursor");
        assert!(next > cursor);
        // Nothing new past the latest cursor.
        let (empty, _) = b.since(next);
        assert!(empty.is_empty());
    }

    #[test]
    fn recovers_after_mutex_poisoning() {
        let b = LogBuffer::new(4);
        b.push_line("before panic");

        let poison_target = b.clone();
        let panicked = std::panic::catch_unwind(move || {
            let _guard = poison_target.inner.lock().expect("mutex starts healthy");
            panic!("simulate a writer panic while holding the log buffer");
        });
        assert!(panicked.is_err(), "the mutex must have been poisoned");
        assert!(b.inner.is_poisoned());

        // Every public lock-taking operation must keep working. Because the
        // poison flag remains set, this also proves recovery is repeatable.
        b.push_line("after panic");
        assert_eq!(b.snapshot(), vec!["before panic", "after panic"]);
        assert_eq!(b.tail(1), (vec!["after panic".to_string()], 2));
        assert_eq!(b.tail_with_cursor(1).0, vec!["after panic"]);
        let cursor = b.cursor();
        b.push_line("new line");
        assert_eq!(b.since(cursor).0, vec!["new line"]);
    }
}
