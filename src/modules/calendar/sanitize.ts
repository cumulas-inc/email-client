import DOMPurify from "dompurify";

/**
 * Event descriptions come from other people's invitations: keep basic
 * formatting and links only — no scripts, event handlers, styles, forms,
 * frames or (tracking) images.
 */
export function sanitizeDescription(html: string): string {
  return DOMPurify.sanitize(html, {
    USE_PROFILES: { html: true },
    FORBID_TAGS: [
      "style",
      "img",
      "picture",
      "form",
      "input",
      "button",
      "textarea",
      "select",
      "video",
      "audio",
      "iframe",
    ],
    FORBID_ATTR: ["style", "class", "id", "srcset", "target"],
  });
}
