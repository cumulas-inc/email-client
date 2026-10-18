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
} from "./protection";

/** Lets the composer point at the label choice when Send needs one. */
export interface ComposeProtectionHandle {
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
 * all until then.
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
  const labelButton = useRef<HTMLButtonElement | null>(null);
  const labelMenu = useRef<MenuHandle>(null);
  useImperativeHandle(
    ref,
    () => ({
      focusLabels: () => labelButton.current?.focus(),
      openLabels: () => labelMenu.current?.open(),
    }),
    [],
  );

  const actions = offeredActions(setup);
  const labels = offeredLabels(setup);
  if (!setup || (actions.length === 0 && labels.length === 0)) return null;

  const selected = labels.find((l) => l.id === labelId) ?? null;
  const selectedTitle = selected ? labelTitle(selected) : null;
  // A label that encrypts brings its own encryption.
  const labelEncrypts = selected?.hasProtection ?? false;
  const chosenAction = protect && !labelEncrypts ? protect : null;

  const actionItems: MenuItem[] = [
    ...actions.map((action) => ({
      label: ACTION_NAMES[action],
      checked: chosenAction === action,
      onSelect: () => onProtect(action),
    })),
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
  if (!labelRequired(setup)) {
    labelItems.push({
      label: "No label",
      checked: selected === null,
      separator: true,
      disabled: !canChooseLabel(setup, originalLabel, null),
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
      {actions.length > 0 && (
        <Menu
          label="Encryption"
          items={actionItems}
          trigger={(props) => (
            <button
              type="button"
              className={clsx(
                "btn btn--subtle mail-tool mail-compose__protect",
                chosenAction && "is-on",
              )}
              aria-label={chosenAction ? `Encryption: ${ACTION_NAMES[chosenAction]}` : "Encryption"}
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
      {labels.length > 0 && (
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
                selected && "is-on",
              )}
              aria-label={
                selectedTitle ? `Sensitivity label: ${selectedTitle}` : "Sensitivity label"
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
