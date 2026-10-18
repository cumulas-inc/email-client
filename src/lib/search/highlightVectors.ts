// What a search marks in a text, for highlight.test.ts and the message
// bridge's tests (messageBridge.test.ts): findHits(text, the field's
// patterns of parseSearch(query)) finds exactly `hits`, the substrings of
// `text` (or, for `ranges`, their UTF-16 offsets).

export interface HighlightVector {
  text: string;
  query: string;
  field: "subject" | "sender" | "body";
  hits: string[];
  ranges?: [number, number][];
}

export const HIGHLIGHT_VECTORS: readonly HighlightVector[] = [
  { text: "Invoice #2023-118 from Northwind", query: "inv", field: "subject", hits: ["Inv"] },
  {
    text: "Résumé for Lucía",
    query: "resume lucia",
    field: "subject",
    hits: ["Résumé", "Lucía"],
  },
  { text: "Résumé attached", query: "résumé", field: "body", hits: ["Résumé"] },
  {
    text: "Quarterly report — Q3",
    query: '"quarterly report"',
    field: "body",
    hits: ["Quarterly report"],
  },
  { text: "quarterly numbers, report", query: '"quarterly report"', field: "body", hits: [] },
  {
    text: "Ada <megan@contoso.com>",
    query: "from:megan@contoso",
    field: "sender",
    hits: ["megan@contoso"],
  },
  // Inside words: subjects and senders only.
  { text: "Your invoice", query: "voic", field: "subject", hits: ["voic"] },
  { text: "Your invoice", query: "voic", field: "body", hits: [] },
  {
    text: "the theme of the day",
    query: "the",
    field: "body",
    hits: ["the", "the", "the"],
    ranges: [
      [0, 3],
      [4, 7],
      [13, 16],
    ],
  },
  { text: "😀 invoice", query: "invoice", field: "body", hits: ["invoice"], ranges: [[3, 10]] },
  // Marks stay with their letter: the whole word, as the snippets show it.
  { text: "हिन्दी पाठ्यक्रम", query: "हिन्दी", field: "body", hits: ["हिन्दी"] },
  // To and Cc aren't shown, so not marked.
  { text: "Lunch?", query: "to:lunch", field: "body", hits: [] },
];
