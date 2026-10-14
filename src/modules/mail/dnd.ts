// Drag-and-drop of messages onto folders. dataTransfer payloads can't be read
// during dragover, so the dragged messages are kept here while dragging.

export const MAIL_DRAG_TYPE = "application/x-cumulas-messages";

export interface MailDrag {
  ids: string[];
  accountId: string;
  folderId: string;
}

let current: MailDrag | null = null;

export function startMailDrag(drag: MailDrag): void {
  current = drag;
}

export function currentMailDrag(): MailDrag | null {
  return current;
}

export function endMailDrag(): void {
  current = null;
}
