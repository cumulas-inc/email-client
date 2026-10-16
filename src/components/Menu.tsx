import { Check } from "lucide-react";
import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
} from "react";

export interface MenuItem {
  label: string;
  icon?: ReactNode;
  onSelect: () => void;
  disabled?: boolean;
  danger?: boolean;
  checked?: boolean;
  /** Draw a separator above this item. */
  separator?: boolean;
  shortcut?: string;
}

interface TriggerProps {
  "aria-haspopup": "menu";
  "aria-expanded": boolean;
  "aria-controls": string;
  onClick: () => void;
  ref: RefObject<HTMLButtonElement | null>;
}

interface MenuProps {
  trigger: (props: TriggerProps) => ReactNode;
  items: MenuItem[];
  align?: "start" | "end";
  label: string;
  /**
   * Place the menu over the window rather than inside its anchor, so a
   * scrolling container around the trigger doesn't cut it off. It opens
   * above the trigger when there's no room below, and closes when the page
   * under it scrolls.
   */
  fixed?: boolean;
}

/** Space kept between a fixed menu and the window's edges. */
const EDGE = 8;

/** A rendered element's CSS zoom (the browser demo zooms with CSS; the desktop app doesn't). */
function cssZoom(el: Element): number {
  const zoom = (el as Element & { currentCSSZoom?: number }).currentCSSZoom;
  return typeof zoom === "number" && zoom > 0 ? zoom : 1;
}

/** Dropdown menu with keyboard navigation (arrows, Home/End, Escape). */
export function Menu({ trigger, items, align = "start", label, fixed = false }: MenuProps) {
  const [open, setOpen] = useState(false);
  const id = useId();
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  const close = useCallback((focusTrigger: boolean) => {
    setOpen(false);
    if (focusTrigger) triggerRef.current?.focus();
  }, []);

  useEffect(() => {
    if (!open) return;
    const first = menuRef.current?.querySelector<HTMLButtonElement>("button:not(:disabled)");
    first?.focus();
    const onPointer = (e: MouseEvent) => {
      if (
        !menuRef.current?.contains(e.target as Node) &&
        !triggerRef.current?.contains(e.target as Node)
      ) {
        close(false);
      }
    };
    document.addEventListener("mousedown", onPointer);
    return () => document.removeEventListener("mousedown", onPointer);
  }, [open, close]);

  // A fixed menu goes under the trigger (or over it), inside the window.
  useLayoutEffect(() => {
    const menu = menuRef.current;
    const button = triggerRef.current;
    if (!open || !fixed || !menu || !button) return;
    const r = button.getBoundingClientRect();
    const zoom = cssZoom(menu);
    const width = menu.offsetWidth * zoom;
    const height = menu.offsetHeight * zoom;
    const left = Math.min(
      Math.max(align === "end" ? r.right - width : r.left, EDGE),
      window.innerWidth - EDGE - width,
    );
    const below = r.bottom + 4;
    const top =
      below + height <= window.innerHeight - EDGE ? below : Math.max(EDGE, r.top - 4 - height);
    menu.style.left = `${left / zoom}px`;
    menu.style.top = `${top / zoom}px`;
    menu.style.right = "auto";
    const onScroll = (e: Event) => {
      if (!menu.contains(e.target as Node)) close(false);
    };
    window.addEventListener("scroll", onScroll, true);
    return () => window.removeEventListener("scroll", onScroll, true);
  }, [open, fixed, align, close]);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const buttons = Array.from(
      menuRef.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [],
    );
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const focus = (i: number) => buttons[(i + buttons.length) % buttons.length]?.focus();
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        focus(index + 1);
        break;
      case "ArrowUp":
        e.preventDefault();
        focus(index - 1);
        break;
      case "Home":
        e.preventDefault();
        focus(0);
        break;
      case "End":
        e.preventDefault();
        focus(buttons.length - 1);
        break;
      case "Escape":
      case "Tab":
        e.preventDefault();
        close(true);
        break;
    }
  };

  return (
    <div className="menu-anchor">
      {trigger({
        "aria-haspopup": "menu",
        "aria-expanded": open,
        "aria-controls": id,
        onClick: () => setOpen((o) => !o),
        ref: triggerRef,
      })}
      {open && (
        <div
          id={id}
          ref={menuRef}
          role="menu"
          aria-label={label}
          className={`menu menu--${align}${fixed ? " menu--fixed" : ""}`}
          onKeyDown={onKeyDown}
        >
          {items.map((item) => (
            <div key={item.label}>
              {item.separator && <div className="menu__separator" role="separator" />}
              <button
                type="button"
                role={item.checked === undefined ? "menuitem" : "menuitemcheckbox"}
                aria-checked={item.checked}
                className={`menu__item${item.danger ? " menu__item--danger" : ""}`}
                disabled={item.disabled}
                onClick={() => {
                  close(true);
                  item.onSelect();
                }}
              >
                <span className="menu__icon">{item.checked ? <Check size={16} /> : item.icon}</span>
                <span className="menu__label">{item.label}</span>
                {item.shortcut && <span className="menu__shortcut">{item.shortcut}</span>}
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
