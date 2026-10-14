import type { ColorOption } from "../../components/ColorPicker";

/** Account colours: readable with white text and on light and dark backgrounds. */
export const ACCOUNT_COLORS: ColorOption[] = [
  { value: "#0f6cbd", name: "Blue" },
  { value: "#038387", name: "Teal" },
  { value: "#107c10", name: "Green" },
  { value: "#8764b8", name: "Purple" },
  { value: "#c239b3", name: "Berry" },
  { value: "#d13438", name: "Red" },
  { value: "#ca5010", name: "Orange" },
  { value: "#986f0b", name: "Gold" },
];

/** The Accounts section's "Add account" button, focused after a removal. */
export const ADD_ACCOUNT_BUTTON_ID = "settings-add-account";
