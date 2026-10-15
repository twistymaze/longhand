# Lens: resources

Find defects in the lifetime of resources: things that are not released, released too early, or grow without a limit.

## Look for

- Devices, streams, files, sockets, child processes, and threads that are not closed or stopped on every path, including errors and cancellation.
- A resource used after it is released or after its owner is dropped.
- Memory, queues, caches, or logs that grow with the length of a session and have no limit. Consider a session of several hours.
- Large models or buffers loaded more than one time, or held after they are no longer needed.
- Child processes that stay after the app quits or crashes.
- Event listeners and timers that are added and never removed.
- System state that the app changes and does not restore, for example a sleep block or an audio device setting.

## Leave to other lenses

Performance that the user cannot see.
