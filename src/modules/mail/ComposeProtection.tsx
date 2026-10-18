import { ChevronDown, Lock, Tag } from "lucide-react";
import clsx from "clsx";
import { type Ref, useImperativeHandle, useRef } from "react";

import { useBackend } from "../../api/context";
import type { MessageLabel, ProtectAction, ProtectionSetup } from "../../api/types";
import { Menu, type MenuHandle, type MenuItem } from "../../components/Menu";
import { toast } from "../../components/toast";
import {
  ACTION_NAMES,
  canChooseLabel,
  labelRequired,
  labelTitle,
  offeredActions,
  offeredLabels,
  unofferedChoice,
} from "./protection";

/** Lets the composer point at the encryption or label choice when Send needs one. */
export interface ComposeProtectionHandle {
  focusEncryption: () => void;
  focusLabels: () => void;
  openLabels: () => void;
}

interface ComposeProtectionProps {
  ref?: Ref<ComposeProtectionHandle>;
  setup: ProtectionSetup | undefined;
  protect: ProtectAction | null;
  labelId: string | null;
  /** The label of the message replied to or forwarded. */
  originalLabel: MessageLabel | null;
  disabled?: boolean;
  onProtect: (action: ProtectAction | null) => void;
  onLabel: (labelId: string | null) => void;
}

/**
 * The composer's Encrypt and Sensitivity buttons: only what a check in
 * Settings found the organization's mail server applies, and nothing at
 * all until then. A choice that stopped being offered (its check expired,
 * or didn't pass again) stays in view, marked so, until it's cleared.
 */
export function ComposeProtection({
  ref,
  setup,
  protect,
  labelId,
  originalLabel,
  disabled = false,
  onProtect,
  onLabel,
}: ComposeProtectionProps) {
  const backend = useBackend();
  const encryptButton = useRef<HTMLButtonElement | null>(null);
  const labelButton = useRef<HTMLButtonElement | null>(null);
  const labelMenu = useRef<MenuHandle>(null);
  useImperativeHandle(
    ref,
    () => ({
      focusEncryption: () => encryptButton.current?.focus(),
      focusLabels: () => labelButton.current?.focus(),
      openLabels: () => labelMenu.current?.open(),
    }),
    [],
  );

  const actions = offeredActions(setup);
  const labels = offeredLabels(setup);
  const unoffered = unofferedChoice({ protect, labelId }, setup);
  const showActions = actions.length > 0 || unoffered.action !== null;
  const showLabels = labels.length > 0 || unoffered.label !== null;
  if (!setup || (!showActions && !showLabels)) return null;

  const selected = labels.find((l) => l.id === labelId) ?? null;
  const stale = unoffered.label;
  const staleTitle = stale ? ("name" in stale ? labelTitle(stale) : "Label you chose") : null;
  const selectedTitle = selected ? labelTitle(selected) : staleTitle;
  // A label that encrypts brings its own encryption.
  const labelEncrypts = selected?.hasProtection ?? false;
  const chosenAction = protect && !labelEncrypts ? protect : null;

  const actionItems: MenuItem[] = [
    ...actions.map((action) => ({
      label: ACTION_NAMES[action],
      checked: chosenAction === action,
      onSelect: () => onProtect(action),
    })),
    // Chosen, but no longer offered: shown, so it can be cleared.
    ...(unoffered.action
      ? [
          {
            label: `${ACTION_NAMES[unoffered.action]} (not available now)`,
            checked: true,
            disabled: true,
            onSelect: () => undefined,
          },
        ]
      : []),
    {
      label: "Don't encrypt",
      checked: chosenAction === null,
      separator: true,
      onSelect: () => onProtect(null),
    },
  ];

  const labelItems: MenuItem[] = labels.map((l) => ({
    label: labelTitle(l),
    icon: l.color ? (
      <span className="mail-label-dot" style={{ background: l.color }} aria-hidden="true" />
    ) : undefined,
    checked: labelId === l.id,
    disabled: !canChooseLabel(setup, originalLabel, l),
    onSelect: () => onLabel(l.id),
  }));
  if (staleTitle) {
    labelItems.push({
      label: `${staleTitle} (not available now)`,
      checked: true,
      disabled: true,
      onSelect: () => undefined,
    });
  }
  // A choice that isn't offered can always be cleared.
  if (!labelRequired(setup) || stale) {
    labelItems.push({
      label: "No label",
      checked: labelId === null,
      separator: true,
      disabled: !stale && !canChooseLabel(setup, originalLabel, null),
      onSelect: () => onLabel(null),
    });
  }
  const moreInfo = setup.policy.moreInfoUrl;
  if (moreInfo) {
    labelItems.push({
      label: "Learn about labels",
      separator: true,
      onSelect: () => {
        backend.openLink(moreInfo).catch((e: unknown) => toast.error(e));
      },
    });
  }

  return (
    <>
      {showActions && (
        <Menu
          label="Encryption"
          items={actionItems}
          trigger={({ ref: triggerRef, ...props }) => (
            <button
              type="button"
              ref={(el) => {
                triggerRef.current = el;
                encryptButton.current = el;
              }}
              className={clsx(
                "btn btn--subtle mail-tool mail-compose__protect",
                chosenAction && "is-on",
              )}
              aria-label={
                chosenAction
                  ? `Encryption: ${ACTION_NAMES[chosenAction]}${unoffered.action ? " (not available now)" : ""}`
                  : "Encryption"
              }
              title={
                labelEncrypts && selectedTitle
                  ? `The label ${selectedTitle} encrypts this message.`
                  : "Encrypt this message"
              }
              disabled={disabled || labelEncrypts}
              {...props}
            >
              <Lock size={17} strokeWidth={1.75} aria-hidden="true" />
              <span className="mail-tool__label">
                {chosenAction ? ACTION_NAMES[chosenAction] : "Encrypt"}
              </span>
              <ChevronDown size={14} className="mail-tool__caret" aria-hidden="true" />
            </button>
          )}
        />
      )}
      {showLabels && (
        <Menu
          ref={labelMenu}
          label="Sensitivity label"
          items={labelItems}
          trigger={({ ref: triggerRef, ...props }) => (
            <button
              type="button"
              ref={(el) => {
                triggerRef.current = el;
                labelButton.current = el;
              }}
              className={clsx(
                "btn btn--subtle mail-tool mail-compose__sensitivity",
                selectedTitle && "is-on",
              )}
              aria-label={
                selectedTitle
                  ? `Sensitivity label: ${selectedTitle}${stale ? " (not available now)" : ""}`
                  : "Sensitivity label"
              }
              title="Sensitivity label"
              disabled={disabled}
              {...props}
            >
              {selected?.color ? (
                <span
                  className="mail-label-dot"
                  style={{ background: selected.color }}
                  aria-hidden="true"
                />
              ) : (
                <Tag size={17} strokeWidth={1.75} aria-hidden="true" />
              )}
              <span className="mail-tool__label">{selectedTitle ?? "Sensitivity"}</span>
              <ChevronDown size={14} className="mail-tool__caret" aria-hidden="true" />
            </button>
          )}
        />
      )}
    </>
  );
}
