# Lens: concurrency

Find defects that occur when two things happen at the same time, or in an order that the code does not expect.

## Look for

- Shared state that two threads or async tasks read and write without a lock, or with a check and an act that are not atomic.
- Locks held across `.await`, across a blocking call, or in an order that can deadlock with another lock.
- A blocking call (file I/O, a model run, a long loop) on an async runtime thread or the UI thread.
- Events or messages that can arrive before setup completes, after teardown, or two times.
- Tasks that are started and never joined or cancelled, so they act after their owner is gone.
- Channels that can fill and block the producer, or that drop messages without a sign.
- A second start of the same operation while the first is still running (two clicks, two windows, a retry).
- Two processes or two computers that write the same file.
- In UI code: a response that arrives after the user has moved to another item, and is applied to the wrong item.

## For each finding

Describe the order of events that causes the failure, step by step, with the line of each step. A race without a specific order is not a finding.

## Leave to other lenses

A single-threaded logic error (`correctness`). Lost data with no concurrency (`data-loss`).
