// The own saves of one Settings field: the values that this window sent and
// that the saved value did not show yet. A saved value that is one of them
// is an own save. Any other saved value is a change from another computer.
// A value stays in the list only until the saved value shows it, so a later
// change on another computer back to that value shows as a change.

export class OwnSaves<T> {
  // In the order sent.
  private values: T[] = [];
  private inFlight = 0;

  /// Records a value that a save sends now.
  sent(value: T) {
    this.values.push(value);
    this.inFlight += 1;
  }

  /// Records that a save ended, with or without success. A failed save keeps
  /// its value, because the backend can have written it.
  settled() {
    this.inFlight = Math.max(0, this.inFlight - 1);
  }

  /// Whether `value` is an own save that the saved value did not show yet.
  has(value: T): boolean {
    return this.values.includes(value);
  }

  /// Call for each new saved value. Returns true when it is an own save: the
  /// oldest equal entry and the entries before it are removed. Another value
  /// clears the list when no save is in flight, because each entry was then
  /// written over or never shown.
  confirm(saved: T): boolean {
    const index = this.values.indexOf(saved);
    if (index >= 0) {
      this.values.splice(0, index + 1);
      return true;
    }
    if (this.inFlight === 0) this.values = [];
    return false;
  }
}
