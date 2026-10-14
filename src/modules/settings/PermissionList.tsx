import { Check } from "lucide-react";

import { MICROSOFT_PERMISSIONS } from "./microsoft";

/** The Microsoft permissions Cumulas Mail requests, in plain language. */
export function PermissionList({ showScopes = false }: { showScopes?: boolean }) {
  return (
    <ul className="permission-list">
      {MICROSOFT_PERMISSIONS.map((p) => (
        <li key={p.scopes[0]}>
          <Check size={14} aria-hidden="true" className="permission-list__check" />
          <span>
            {p.label}
            {showScopes && <span className="permission-list__scopes"> {p.scopes.join(", ")}</span>}
          </span>
        </li>
      ))}
    </ul>
  );
}
