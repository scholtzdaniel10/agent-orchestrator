//! Buffers terminal output until a pane is listening. Port of `terminal-bus.ts`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const DEFAULT_MAX_BUFFERED: usize = 1_000_000;

type Sink = Rc<RefCell<Option<Box<dyn FnMut(String)>>>>;

struct Slot {
    generation: u64,
    sink: Sink,
}

struct Inner {
    buffers: HashMap<String, String>,
    sinks: HashMap<String, Slot>,
    next_generation: u64,
    disposed: bool,
    max_buffered: usize,
    unsubscribe: Option<Box<dyn FnOnce()>>,
}

pub struct TerminalBus {
    inner: Rc<RefCell<Inner>>,
}

impl Clone for TerminalBus {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
    }
}

impl PartialEq for TerminalBus {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
}

pub struct Detach {
    inner: Rc<RefCell<Inner>>,
    id: String,
    generation: u64,
}

impl Detach {
    pub fn call(self) {
        let mut inner = self.inner.borrow_mut();
        let still = inner
            .sinks
            .get(&self.id)
            .is_some_and(|slot| slot.generation == self.generation);
        if still && let Some(slot) = inner.sinks.remove(&self.id) {
            *slot.sink.borrow_mut() = None;
        }
    }
}

pub fn create_terminal_bus(
    subscribe: impl FnOnce(Box<dyn FnMut(String, String)>) -> Box<dyn FnOnce()>,
    max_buffered: Option<usize>,
) -> TerminalBus {
    let inner = Rc::new(RefCell::new(Inner {
        buffers: HashMap::new(),
        sinks: HashMap::new(),
        next_generation: 1,
        disposed: false,
        max_buffered: max_buffered.unwrap_or(DEFAULT_MAX_BUFFERED),
        unsubscribe: None,
    }));
    let hook = Rc::clone(&inner);
    let unsubscribe = subscribe(Box::new(move |id, data| {
        deliver(&hook, id, data);
    }));
    inner.borrow_mut().unsubscribe = Some(unsubscribe);
    TerminalBus { inner }
}

fn deliver(inner: &Rc<RefCell<Inner>>, id: String, data: String) {
    let sink = {
        let borrowed = inner.borrow();
        if borrowed.disposed {
            return;
        }
        borrowed.sinks.get(&id).map(|slot| Rc::clone(&slot.sink))
    };
    if let Some(sink) = sink {
        if let Some(callback) = sink.borrow_mut().as_mut() {
            callback(data);
        }
        return;
    }
    let mut borrowed = inner.borrow_mut();
    if borrowed.disposed {
        return;
    }
    let max = borrowed.max_buffered;
    let next = format!(
        "{}{data}",
        borrowed.buffers.get(&id).map(String::as_str).unwrap_or("")
    );
    let stored = if next.chars().count() > max {
        tail(&next, max)
    } else {
        next
    };
    borrowed.buffers.insert(id, stored);
}

fn tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    text.chars().skip(count.saturating_sub(max)).collect()
}

impl TerminalBus {
    pub fn attach(&self, id: &str, sink: impl FnMut(String) + 'static) -> Detach {
        let mut inner = self.inner.borrow_mut();
        if inner.disposed {
            return Detach {
                inner: Rc::clone(&self.inner),
                id: id.to_string(),
                generation: 0,
            };
        }
        let generation = inner.next_generation;
        inner.next_generation += 1;
        let boxed: Sink = Rc::new(RefCell::new(Some(Box::new(sink))));
        inner.sinks.insert(
            id.to_string(),
            Slot {
                generation,
                sink: Rc::clone(&boxed),
            },
        );
        let buffered = inner.buffers.remove(id).filter(|text| !text.is_empty());
        drop(inner);
        if let Some(buffered) = buffered
            && let Some(callback) = boxed.borrow_mut().as_mut()
        {
            callback(buffered);
        }
        Detach {
            inner: Rc::clone(&self.inner),
            id: id.to_string(),
            generation,
        }
    }

    pub fn forget(&self, id: &str) {
        self.inner.borrow_mut().buffers.remove(id);
    }

    pub fn dispose(&self) {
        let mut inner = self.inner.borrow_mut();
        if inner.disposed {
            return;
        }
        inner.disposed = true;
        if let Some(unsubscribe) = inner.unsubscribe.take() {
            drop(inner);
            unsubscribe();
        } else {
            drop(inner);
        }
        let mut inner = self.inner.borrow_mut();
        inner.buffers.clear();
        inner.sinks.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::create_terminal_bus;
    use std::cell::RefCell;
    use std::rc::Rc;

    type Emit = Box<dyn FnMut(&str, &str)>;
    type Listener = Rc<RefCell<Option<Box<dyn FnMut(String, String)>>>>;

    struct Harness {
        emit: Emit,
        unsubscribes: Rc<RefCell<u32>>,
    }

    fn harness(max: Option<usize>) -> (super::TerminalBus, Harness) {
        let listener: Listener = Rc::new(RefCell::new(None));
        let unsubscribes = Rc::new(RefCell::new(0));
        let listen = Rc::clone(&listener);
        let count = Rc::clone(&unsubscribes);
        let bus = create_terminal_bus(
            move |cb| {
                *listen.borrow_mut() = Some(cb);
                let listen = Rc::clone(&listen);
                let count = Rc::clone(&count);
                Box::new(move || {
                    *count.borrow_mut() += 1;
                    *listen.borrow_mut() = None;
                })
            },
            max,
        );
        let emit = Box::new(move |id: &str, data: &str| {
            let mut slot = listener.borrow_mut();
            let Some(cb) = slot.as_mut() else {
                panic!("not subscribed");
            };
            cb(id.to_string(), data.to_string());
        });
        (bus, Harness { emit, unsubscribes })
    }

    #[test]
    fn buffers_before_attach_and_flushes_in_order() {
        let (bus, mut harness) = harness(None);
        (harness.emit)("a", "one");
        (harness.emit)("a", "two");
        (harness.emit)("b", "other");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let record = Rc::clone(&seen);
        bus.attach("a", move |chunk| record.borrow_mut().push(chunk));
        assert_eq!(seen.borrow().as_slice(), ["onetwo"]);
    }

    #[test]
    fn delivers_live_data_after_attach() {
        let (bus, mut harness) = harness(None);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let record = Rc::clone(&seen);
        bus.attach("a", move |chunk| record.borrow_mut().push(chunk));
        (harness.emit)("a", "live");
        assert_eq!(seen.borrow().as_slice(), ["live"]);
    }

    #[test]
    fn detach_then_reattach_flushes_the_gap() {
        let (bus, mut harness) = harness(None);
        let first = Rc::new(RefCell::new(Vec::new()));
        let record = Rc::clone(&first);
        let detach = bus.attach("a", move |chunk| record.borrow_mut().push(chunk));
        (harness.emit)("a", "during");
        detach.call();
        (harness.emit)("a", "between");
        let second = Rc::new(RefCell::new(Vec::new()));
        let record = Rc::clone(&second);
        bus.attach("a", move |chunk| record.borrow_mut().push(chunk));
        (harness.emit)("a", "after");
        assert_eq!(first.borrow().as_slice(), ["during"]);
        assert_eq!(second.borrow().as_slice(), ["between", "after"]);
    }

    #[test]
    fn attaching_again_replaces_the_sink() {
        let (bus, mut harness) = harness(None);
        let first = Rc::new(RefCell::new(Vec::new()));
        let record = Rc::clone(&first);
        let detach_first = bus.attach("a", move |chunk| record.borrow_mut().push(chunk));
        let second = Rc::new(RefCell::new(Vec::new()));
        let record = Rc::clone(&second);
        bus.attach("a", move |chunk| record.borrow_mut().push(chunk));
        detach_first.call();
        (harness.emit)("a", "only-second");
        assert!(first.borrow().is_empty());
        assert_eq!(second.borrow().as_slice(), ["only-second"]);
    }

    #[test]
    fn the_cap_keeps_the_tail() {
        let (bus, mut harness) = harness(Some(5));
        (harness.emit)("a", "abcdef");
        (harness.emit)("a", "gh");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let record = Rc::clone(&seen);
        bus.attach("a", move |chunk| record.borrow_mut().push(chunk));
        assert_eq!(seen.borrow().as_slice(), ["defgh"]);
    }

    #[test]
    fn forget_drops_the_buffer() {
        let (bus, mut harness) = harness(None);
        (harness.emit)("a", "secret");
        bus.forget("a");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let record = Rc::clone(&seen);
        bus.attach("a", move |chunk| record.borrow_mut().push(chunk));
        (harness.emit)("a", "next");
        assert_eq!(seen.borrow().as_slice(), ["next"]);
    }

    #[test]
    fn dispose_unsubscribes() {
        let (bus, mut harness) = harness(None);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let record = Rc::clone(&seen);
        bus.attach("a", move |chunk| record.borrow_mut().push(chunk));
        bus.dispose();
        assert_eq!(*harness.unsubscribes.borrow(), 1);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (harness.emit)("a", "late");
        }));
        assert!(result.is_err());
        assert!(seen.borrow().is_empty());
    }
}
