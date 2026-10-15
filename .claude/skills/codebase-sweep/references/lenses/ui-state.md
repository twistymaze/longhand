# Lens: ui-state

Find defects in interface code where the screen shows a wrong state, loses input, or acts on the wrong item.

## Look for

- Stale closures: a handler or effect that uses an old value of state or props.
- Effects with missing or wrong dependencies, effects that run two times and act two times, and effects without cleanup.
- An async result that arrives after the user changed the selection or closed the view, and that the code then applies.
- State that is copied from props and does not update when the props change.
- Input that is lost: an edit that is not saved before a switch, a debounce that drops the last change, a reload that replaces unsaved text.
- Calls to the backend whose errors the interface ignores, so it shows success after a failure.
- Keyboard and focus handling that leaves a control unusable without a mouse.
- A list key that is not stable, so the wrong item keeps its state.

## Leave to other lenses

Visual design and wording. Backend races (`concurrency`).
