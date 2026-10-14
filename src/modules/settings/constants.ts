import type { ColorOption } from "../../components/ColorPicker";

/** Account colors: readable with white text and on light and dark backgrounds. */
export const ACCOUNT_COLORS: ColorOption[] = [
  { value: "#0b7a75", name: "Teal" },
  { value: "#2b7aa8", name: "Sky" },
  { value: "#b35f26", name: "Apricot" },
  { value: "#8a4f7d", name: "Plum" },
  { value: "#4d7a2e", name: "Moss" },
  { value: "#8c6a10", name: "Gold" },
  { value: "#b8434f", name: "Rose" },
  { value: "#5e6b7a", name: "Slate" },
];

/** The Accounts section's "Add account" button, focused after a removal. */
export const ADD_ACCOUNT_BUTTON_ID = "settings-add-account";
