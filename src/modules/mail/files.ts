// File type detection for attachment icons.

export type FileKind =
  "pdf" | "word" | "sheet" | "slides" | "image" | "archive" | "audio" | "video" | "code" | "other";

export function fileKind(name: string, contentType: string): FileKind {
  const ext = name.toLowerCase().split(".").pop() ?? "";
  const type = contentType.toLowerCase();
  if (ext === "pdf" || type === "application/pdf") return "pdf";
  if (["doc", "docx", "odt", "rtf"].includes(ext) || type.includes("wordprocessing")) return "word";
  if (["xls", "xlsx", "ods", "csv"].includes(ext) || type.includes("spreadsheet")) return "sheet";
  if (["ppt", "pptx", "odp", "key"].includes(ext) || type.includes("presentation")) return "slides";
  if (type.startsWith("image/")) return "image";
  if (type.startsWith("audio/")) return "audio";
  if (type.startsWith("video/")) return "video";
  if (["zip", "gz", "tgz", "7z", "rar", "tar", "xz", "bz2"].includes(ext) || type.includes("zip")) {
    return "archive";
  }
  if (["js", "ts", "json", "html", "xml", "py", "rs", "sh", "css"].includes(ext)) return "code";
  if (type.startsWith("text/")) return "word";
  return "other";
}
