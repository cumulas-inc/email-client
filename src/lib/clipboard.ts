/**
 * Copies a text the backend is still putting together, such as a details
 * text for support. The write starts here, in the click: WebKit refuses one
 * that starts only after the core answered, which can take a while. The text
 * follows as a promise.
 *
 * Rejects with the core's own error when the text failed, which says more
 * than the clipboard's.
 */
export function copyLater(text: Promise<string>): Promise<void> {
  const copied =
    typeof ClipboardItem !== "undefined" && navigator.clipboard?.write
      ? navigator.clipboard.write([
          new ClipboardItem({
            "text/plain": text.then((t) => new Blob([t], { type: "text/plain" })),
          }),
        ])
      : text.then((t) => {
          // Never "Copied" when nothing was.
          if (!navigator.clipboard) throw new Error("Copying isn't available here.");
          return navigator.clipboard.writeText(t);
        });
  return copied.catch((e: unknown) =>
    text.then(
      () => Promise.reject(e),
      (reason: unknown) => Promise.reject(reason),
    ),
  );
}
