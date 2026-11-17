type KeyState = Pick<KeyboardEvent, "isComposing" | "keyCode">;

/// Whether the input method (Japanese, Chinese or Korean input) uses this
/// key. A keydown handler returns at once for such a key: the Enter that
/// confirms a composition must not save or submit, and Escape and the arrow
/// keys belong to the composition too. WebKit sends the confirming Enter
/// after compositionend, with isComposing false, so keyCode 229 is also
/// necessary. Accepts a DOM event or a React event.
export function isImeComposing(event: KeyState | { nativeEvent: KeyState }): boolean {
  const key = "nativeEvent" in event ? event.nativeEvent : event;
  return key.isComposing || key.keyCode === 229;
}
