//! The run-once record, shared by every thread a `parallel` starts.
//!
//! "A task runs once per invocation, keyed on name and arguments" is a
//! promise about the *run*, not about one interpreter, so the record cannot
//! live inside one. `parallel build test`, where both call `deps`, has to run
//! `deps` once: giving each thread its own map would run it twice and nothing
//! would notice, which is the one outcome this module exists to prevent.
//!
//! So the map is behind a mutex and an entry is claimed *before* the body
//! runs, not after it finishes. A key is either
//! [`Running`](Slot::Running), meaning some context owns it and is running
//! the body now, or [`Done`](Slot::Done), with the captured stdout if
//! something asked for a value. Two siblings that reach the same key at the
//! instant therefore do not both run it: the first inserts `Running`, the
//! second finds it and blocks on the condvar until the owner resolves the
//! slot, then reads the answer. The lock is held only while a slot is
//! inspected or changed, never while a body runs, so tasks that share nothing
//! never serialize.
//!
//! # Waiting without deadlocking
//!
//! Blocking on another context introduces cycles a single-threaded run could
//! not have: `parallel a b` where `a` calls `b` and `b` calls `a` would have
//! each thread holding the key the other wants. The waits are therefore
//! recorded as a graph and consulted before blocking: a context waits on
//! whoever owns the key it wants, and a `parallel` waits on the children it
//! spawned. If the owner can reach us through that graph, blocking would
//! deadlock, and the call falls back to the answer a single-threaded run
//! gives for a key that is already on the stack: skip it, or rerun it when a
//! value is wanted. A cycle then costs an extra run at worst, where the
//! honest-looking alternative costs the whole run.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

/// A task name and the arguments it ran with, defaults filled in.
pub(super) type Key = (String, Vec<String>);

/// Identifies one interpreter. The run starts with one; every `parallel`
/// child gets a fresh one, and the wait graph is drawn over them.
pub(super) type CtxId = u64;

enum Slot {
    /// This context is running the body right now.
    Running(CtxId),
    /// The body has run. The value is the stdout something captured, or
    /// `None` when the task only ever streamed and there is nothing to
    /// replay.
    Done(Option<Vec<u8>>),
}

/// What a slot held, lifted out of the map so the borrow ends before the
/// claim decides what to do about it.
enum Peek {
    Vacant,
    Done(Option<Vec<u8>>),
    Running(CtxId),
}

struct State {
    slots: HashMap<Key, Slot>,
    /// Who each context is blocked on. One entry per context, because a
    /// context is either running a task (and may be waiting on one key) or
    /// joining the children a `parallel` spawned.
    waits: HashMap<CtxId, Vec<CtxId>>,
    next: CtxId,
}

impl State {
    /// Can `from` reach `target` by following what contexts are waiting on?
    /// True means blocking on `from` would close a cycle.
    fn reaches(&self, from: CtxId, target: CtxId) -> bool {
        let mut seen = HashSet::new();
        let mut stack = vec![from];
        while let Some(at) = stack.pop() {
            if at == target {
                return true;
            }
            if !seen.insert(at) {
                continue;
            }
            if let Some(on) = self.waits.get(&at) {
                stack.extend(on.iter().copied());
            }
        }
        false
    }
}

/// The run's shared record. Cloned as an [`Arc`] into every parallel child.
pub(super) struct Memo {
    state: Mutex<State>,
    /// Signalled whenever a slot changes, so a waiter re-reads it.
    moved: Condvar,
}

/// What a caller should do about a key it asked for.
pub(super) enum Claimed {
    /// Nobody has run it: run the body, and hold the claim until it is over.
    Run(Claim),
    /// It ran and something captured what it printed; that is the answer.
    Replay(Vec<u8>),
    /// It ran and no value is wanted: the work is done.
    Skip,
    /// It ran, but only ever streamed, so there is no value to replay and
    /// running it again is the only honest way to answer.
    Rerun,
}

impl Memo {
    /// A fresh record, and the id of the context that owns the run.
    pub(super) fn new() -> (Arc<Self>, CtxId) {
        let memo = Arc::new(Self {
            state: Mutex::new(State {
                slots: HashMap::new(),
                waits: HashMap::new(),
                next: 1,
            }),
            moved: Condvar::new(),
        });
        (memo, 0)
    }

    /// A poisoned lock means a task panicked while holding it. The map it
    /// left behind is still a map of well-formed entries, and refusing to
    /// look at it would turn one task's panic into a hung run.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// An id for a new parallel child.
    pub(super) fn context(&self) -> CtxId {
        let mut state = self.lock();
        state.next += 1;
        state.next
    }

    /// Record that `ctx` is blocked until `on` finish. A `parallel` calls
    /// this before it spawns, so a child that asks for a key its parent holds
    /// can see that waiting for the parent would be waiting for itself.
    pub(super) fn joining(&self, ctx: CtxId, on: Vec<CtxId>) {
        self.lock().waits.insert(ctx, on);
    }

    pub(super) fn joined(&self, ctx: CtxId) {
        self.lock().waits.remove(&ctx);
    }

    /// Ask for a key, blocking while another context runs it.
    pub(super) fn claim(self: &Arc<Self>, key: &Key, ctx: CtxId, wants_value: bool) -> Claimed {
        let mut state = self.lock();
        loop {
            let peek = match state.slots.get(key) {
                None => Peek::Vacant,
                Some(Slot::Done(value)) => Peek::Done(value.clone()),
                Some(Slot::Running(owner)) => Peek::Running(*owner),
            };
            match peek {
                Peek::Vacant => {
                    state.slots.insert(key.clone(), Slot::Running(ctx));
                    return Claimed::Run(Claim {
                        memo: Arc::clone(self),
                        key: key.clone(),
                        ctx,
                        abandon: false,
                    });
                }
                Peek::Done(Some(value)) if wants_value => return Claimed::Replay(value),
                Peek::Done(_) if !wants_value => return Claimed::Skip,
                Peek::Done(_) => return Claimed::Rerun,
                // Our own claim is a task calling itself, and an owner that
                // can reach us is a cycle between threads. Neither can be
                // waited out, and both get the answer a single-threaded run
                // gives.
                Peek::Running(owner) if owner == ctx || state.reaches(owner, ctx) => {
                    return if wants_value {
                        Claimed::Rerun
                    } else {
                        Claimed::Skip
                    };
                }
                Peek::Running(owner) => {
                    state.waits.insert(ctx, vec![owner]);
                    state = self.moved.wait(state).unwrap_or_else(|e| e.into_inner());
                    state.waits.remove(&ctx);
                }
            }
        }
    }

    /// Remember what a task printed, so the next capture of it gets the value
    /// back instead of the body running again.
    pub(super) fn record(&self, key: &Key, stdout: &[u8]) {
        self.lock()
            .slots
            .insert(key.clone(), Slot::Done(Some(stdout.to_vec())));
        self.moved.notify_all();
    }
}

/// One held claim: while it lives, this context owns the key and everyone
/// else waits. Resolving the slot on drop rather than at the end of a happy
/// path is what keeps a failed, unwound or panicking task from leaving
/// waiters asleep on a key nobody will ever finish.
pub(super) struct Claim {
    memo: Arc<Memo>,
    key: Key,
    ctx: CtxId,
    abandon: bool,
}

impl Claim {
    /// Give the key back unrun. `--fail-fast` stops a sibling between
    /// statements, and a task that stopped halfway has not run: leaving a
    /// `Done` behind would let a later call skip work that never happened.
    pub(super) fn abandon(&mut self) {
        self.abandon = true;
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        let mut state = self.memo.lock();
        // Only our own `Running` is ours to resolve: a capture may already
        // have recorded the value, and that answer is better than ours.
        if matches!(state.slots.get(&self.key), Some(Slot::Running(o)) if *o == self.ctx) {
            if self.abandon {
                state.slots.remove(&self.key);
            } else {
                state.slots.insert(self.key.clone(), Slot::Done(None));
            }
        }
        drop(state);
        self.memo.moved.notify_all();
    }
}
