// The notes saves that are not complete, so that a leave of a note waits
// for them (R18). The removal of an empty note reads the notes in the
// store, and a save that comes after the removal finds no note.
//
// An editor waits a short time before it sends a change. Each open editor
// gives its send function here, and `notesSaved` sends each change that
// waits before it waits for the saves.

const sends = new Set<() => void>();
const running = new Map<string, Set<Promise<unknown>>>();

/// An open editor sends its waiting change with `send`. The function that
/// this returns removes it when the editor closes.
export function addNotesSend(send: () => void): () => void {
  sends.add(send);
  return () => {
    sends.delete(send);
  };
}

/// A save of the notes of the note `id` started. `save` settles when the
/// save, and each save that it starts after it, is complete.
export function trackNotesSave(id: string, save: Promise<unknown>) {
  let saves = running.get(id);
  if (!saves) running.set(id, (saves = new Set()));
  saves.add(save);
  const done = saves;
  const end = () => {
    done.delete(save);
    if (done.size === 0 && running.get(id) === done) running.delete(id);
  };
  save.then(end, end);
}

/// Send each waiting change, then wait until no save of the notes of `id`
/// runs. A save that fails counts as complete.
export async function notesSaved(id: string): Promise<void> {
  for (const send of sends) send();
  for (let saves = running.get(id); saves; saves = running.get(id)) {
    await Promise.allSettled([...saves]);
  }
}
