//! Bounded retention of host TCP terminal events when the app stops polling.
//! A terminal occupies the same admission budget as its former live peer.
//! Accept refuses new sockets while the combined budget is exhausted, so
//! repeated reconnects cannot grow an unbounded disconnected-peer backlog.
use std::collections::VecDeque;
use std::sync::Mutex;

use crate::transport::TransportEvent;

#[derive(Default)]
pub(in crate::transport::socket) struct TerminalEventMailbox {
    pending: Mutex<VecDeque<TransportEvent>>,
}

impl TerminalEventMailbox {
    pub(in crate::transport::socket) fn pending(&self) -> usize {
        self.pending
            .lock()
            .expect("terminal mailbox lock poisoned")
            .len()
    }

    pub(in crate::transport::socket) fn push(
        &self,
        event: TransportEvent,
        capacity: usize,
    ) -> bool {
        let mut pending = self.pending.lock().expect("terminal mailbox lock poisoned");
        if pending.len() >= capacity {
            return false;
        }
        pending.push_back(event);
        true
    }

    pub(in crate::transport::socket) fn pop(&self) -> Option<TransportEvent> {
        self.pending
            .lock()
            .expect("terminal mailbox lock poisoned")
            .pop_front()
    }
}
