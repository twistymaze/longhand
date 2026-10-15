# Lens: error-recovery

Find failure paths that make a bad result worse: errors that are hidden, cleanup that does not run, and retries that cause new failures.

## Look for

- Errors that the code ignores or changes into a default value, so a later step acts on wrong data (`let _ =`, `.ok()`, `unwrap_or_default()`, an empty `catch`, a promise without a rejection handler).
- `unwrap`, `expect`, indexes, or `panic!` on a path where the input comes from disk, the network, a device, or the user.
- Cleanup that runs only on the success path, so a failure leaves a lock, a temporary file, a marker, or a state flag behind.
- A state that stays "in progress" forever after a failure, so the user cannot try again.
- Retries without a limit, without a delay, or that repeat a step that is not safe to repeat.
- An error that the user must know about but that the code only logs, or a message that does not say what the user can do.
- External failures: permission denied, disk full, a device that disconnects, a model file that is missing or damaged, a network that is not available.

## Leave to other lenses

Data loss as the result (`data-loss`) when the main defect is the write itself.
