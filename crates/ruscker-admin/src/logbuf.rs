//! In-memory ring buffer of recent Ruscker log lines (#100).
//!
//! A `tracing` layer (wired in `ruscker-cli`) pushes each formatted log
//! line here; the admin "Logs" tab reads a snapshot and follows new
//! lines over SSE. It's a bounded *recent tail* — lost on restart;
//! journald / `docker logs` / `container-log-path` remain the durable
//! source. No `tracing` dependency lives here on purpose: the writer
//! that feeds it is owned by the CLI.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Monotonic, gap-tolerant line sequence — lets the SSE follower ask
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

    /// Append one line, evicting the oldest if at capacity.
    pub fn push_line(&self, line: impl Into<String>) {
        let mut g = self.inner.lock().unwrap();
        let seq = g.next_seq;
        g.next_seq += 1;
        g.lines.push_back((seq, line.into()));
        while g.lines.len() > g.cap {
            g.lines.pop_front();
        }
    }

    /// Current snapshot, oldest-first — full buffer (e.g. the download
    /// endpoint).
    pub fn snapshot(&self) -> Vec<String> {
        let g = self.inner.lock().unwrap();
        g.lines.iter().map(|(_, l)| l.clone()).collect()
    }

    /// The last `n` lines (oldest-first within the tail) plus the total
    /// number of buffered lines. Caps the initial `/admin/logs` render so
    /// a long-lived buffer doesn't ship the whole ring on every load
    /// (#200); the SSE stream then appends what's new.
    pub fn tail(&self, n: usize) -> (Vec<String>, usize) {
        let g = self.inner.lock().unwrap();
        let total = g.lines.len();
        let start = total.saturating_sub(n);
        let lines = g.lines.iter().skip(start).map(|(_, l)| l.clone()).collect();
        (lines, total)
    }

    /// The next sequence number that will be assigned — an SSE follower
    /// records this at connect and asks [`Self::since`] for anything past
    /// it.
    pub fn cursor(&self) -> u64 {
        self.inner.lock().unwrap().next_seq
    }

    /// Lines with `seq >= after`, plus the new cursor. Lines evicted
    /// since `after` are simply absent (a tail, not a guaranteed log).
    pub fn since(&self, after: u64) -> (Vec<String>, u64) {
        let g = self.inner.lock().unwrap();
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
}
