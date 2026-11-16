import { fireEvent } from "@testing-library/react";

/// Sends the keydown that WebKit sends when a key confirms or cancels an
/// input method composition (Japanese, Chinese or Korean input): the
/// composition has ended, so isComposing is false, but keyCode is 229.
export function imeKeyDown(target: Element, key = "Enter") {
  fireEvent.keyDown(target, { key, keyCode: 229 });
}
