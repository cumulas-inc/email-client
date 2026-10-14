import { Placeholder } from "@tiptap/extensions";
import { EditorContent, useEditor, useEditorState } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import { Bold, Italic, Link2, List, ListOrdered, RemoveFormatting, Underline } from "lucide-react";
import { type ReactNode, type Ref, useEffect, useImperativeHandle, useRef, useState } from "react";

import { Dialog } from "../../components/Dialog";

export interface EditorHandle {
  focus: () => void;
}

interface RichTextEditorProps {
  initialHtml: string;
  onChange: (html: string) => void;
  autoFocus?: boolean;
  placeholder?: string;
  ref?: Ref<EditorHandle>;
}

function normaliseUrl(raw: string): string | null {
  const value = raw.trim();
  if (!value) return null;
  if (/^(https?:|mailto:)/i.test(value)) return value;
  if (/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(value)) return `mailto:${value}`;
  return `https://${value}`;
}

function FormatButton({
  label,
  shortcut,
  active,
  onClick,
  children,
}: {
  label: string;
  shortcut?: string;
  active?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      className="icon-btn icon-btn--sm mail-format__btn"
      aria-label={label}
      aria-pressed={active}
      title={shortcut ? `${label} (${shortcut})` : label}
      // Keep the editor's selection while clicking.
      onMouseDown={(e) => e.preventDefault()}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

/** Rich-text body editor (TipTap) with a compact formatting toolbar. */
export function RichTextEditor({
  initialHtml,
  onChange,
  autoFocus,
  placeholder = "Write your message",
  ref,
}: RichTextEditorProps) {
  const onChangeRef = useRef(onChange);
  useEffect(() => {
    onChangeRef.current = onChange;
  });
  const [linkOpen, setLinkOpen] = useState(false);
  const [linkUrl, setLinkUrl] = useState("");
  const [linkText, setLinkText] = useState("");

  const editor = useEditor({
    extensions: [
      StarterKit.configure({
        heading: false,
        codeBlock: false,
        horizontalRule: false,
        link: {
          openOnClick: false,
          autolink: true,
          linkOnPaste: true,
          defaultProtocol: "https",
          protocols: ["mailto"],
        },
      }),
      Placeholder.configure({ placeholder }),
    ],
    content: initialHtml,
    autofocus: autoFocus ? "start" : false,
    editorProps: {
      attributes: {
        class: "mail-editor__content",
        role: "textbox",
        "aria-multiline": "true",
        "aria-label": "Message body",
      },
      handleKeyDown: (_view, event) => {
        // Ctrl+Enter sends (handled by the composer), not a line break.
        if ((event.ctrlKey || event.metaKey) && event.key === "Enter") return true;
        // Ctrl+K: insert link. Elsewhere it jumps to search, so keep it here.
        if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
          event.preventDefault();
          event.stopPropagation();
          openLinkDialog();
          return true;
        }
        return false;
      },
    },
    onUpdate: ({ editor: e }) => onChangeRef.current(e.getHTML()),
  });

  const state = useEditorState({
    editor,
    selector: ({ editor: e }) => ({
      bold: e?.isActive("bold") ?? false,
      italic: e?.isActive("italic") ?? false,
      underline: e?.isActive("underline") ?? false,
      bullet: e?.isActive("bulletList") ?? false,
      ordered: e?.isActive("orderedList") ?? false,
      link: e?.isActive("link") ?? false,
    }),
  });

  useImperativeHandle(ref, () => ({
    focus: () => editor?.commands.focus("start"),
  }));

  function openLinkDialog() {
    if (!editor) return;
    const existing = editor.getAttributes("link").href as string | undefined;
    const { from, to } = editor.state.selection;
    setLinkUrl(existing ?? "");
    setLinkText(editor.state.doc.textBetween(from, to, " "));
    setLinkOpen(true);
  }

  const applyLink = () => {
    if (!editor) return;
    const href = normaliseUrl(linkUrl);
    const chain = editor.chain().focus().extendMarkRange("link");
    if (!href) {
      chain.unsetLink().run();
    } else if (editor.state.selection.empty && !editor.isActive("link")) {
      const text = linkText.trim() || linkUrl.trim();
      chain.insertContent({ type: "text", text, marks: [{ type: "link", attrs: { href } }] }).run();
    } else {
      chain.setLink({ href }).run();
    }
    setLinkOpen(false);
  };

  if (!editor) return null;

  return (
    <div className="mail-editor">
      <div className="mail-format" role="toolbar" aria-label="Formatting">
        <FormatButton
          label="Bold"
          shortcut="Ctrl+B"
          active={state?.bold}
          onClick={() => editor.chain().focus().toggleBold().run()}
        >
          <Bold size={15} />
        </FormatButton>
        <FormatButton
          label="Italic"
          shortcut="Ctrl+I"
          active={state?.italic}
          onClick={() => editor.chain().focus().toggleItalic().run()}
        >
          <Italic size={15} />
        </FormatButton>
        <FormatButton
          label="Underline"
          shortcut="Ctrl+U"
          active={state?.underline}
          onClick={() => editor.chain().focus().toggleUnderline().run()}
        >
          <Underline size={15} />
        </FormatButton>
        <span className="mail-format__separator" />
        <FormatButton
          label="Bulleted list"
          active={state?.bullet}
          onClick={() => editor.chain().focus().toggleBulletList().run()}
        >
          <List size={15} />
        </FormatButton>
        <FormatButton
          label="Numbered list"
          active={state?.ordered}
          onClick={() => editor.chain().focus().toggleOrderedList().run()}
        >
          <ListOrdered size={15} />
        </FormatButton>
        <span className="mail-format__separator" />
        <FormatButton
          label="Insert link"
          shortcut="Ctrl+K"
          active={state?.link}
          onClick={openLinkDialog}
        >
          <Link2 size={15} />
        </FormatButton>
        <FormatButton
          label="Clear formatting"
          onClick={() => editor.chain().focus().unsetAllMarks().clearNodes().run()}
        >
          <RemoveFormatting size={15} />
        </FormatButton>
      </div>
      <EditorContent editor={editor} className="mail-editor__scroll selectable" />
      <Dialog
        open={linkOpen}
        onClose={() => {
          setLinkOpen(false);
          editor.commands.focus();
        }}
        title="Insert link"
        width={420}
        footer={
          <>
            <button type="button" className="btn btn--primary" onClick={applyLink}>
              {linkUrl.trim() ? "Insert" : "Remove link"}
            </button>
            <button
              type="button"
              className="btn"
              onClick={() => {
                setLinkOpen(false);
                editor.commands.focus();
              }}
            >
              Cancel
            </button>
          </>
        }
      >
        <form
          className="mail-link-form"
          onSubmit={(e) => {
            e.preventDefault();
            applyLink();
          }}
        >
          <label className="field">
            <span className="field__label">Address</span>
            <input
              className="input"
              value={linkUrl}
              autoFocus
              placeholder="https://"
              onChange={(e) => setLinkUrl(e.target.value)}
            />
          </label>
          {editor.state.selection.empty && !state?.link && (
            <label className="field">
              <span className="field__label">Text to display</span>
              <input
                className="input"
                value={linkText}
                onChange={(e) => setLinkText(e.target.value)}
              />
            </label>
          )}
          <button type="submit" hidden />
        </form>
      </Dialog>
    </div>
  );
}
