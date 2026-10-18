import { Check } from "lucide-react";
import { useId } from "react";

import { MICROSOFT_PERMISSIONS, type Permission } from "./microsoft";

interface PermissionListProps {
  showScopes?: boolean;
  /** Only the permissions mail needs, without headings. */
  only?: "core";
  /** The push service's permission too (the approval link covers it). */
  push?: boolean;
}

/** The push service's permission, as the app describes it. */
const PUSH: Permission = {
  scopes: ["Sync.Receive"],
  label: "Receive notices that your mail or calendar changed (instant updates)",
  request: "",
};

function Items({ permissions, showScopes }: { permissions: Permission[]; showScopes: boolean }) {
  return permissions.map((p) => (
    <li key={p.scopes[0]}>
      <Check size={14} aria-hidden="true" className="permission-list__check" />
      <span>
        {p.label}
        {showScopes && <span className="permission-list__scopes"> {p.scopes.join(", ")}</span>}
      </span>
    </li>
  ));
}

/** The Microsoft permissions Cumulas Mail requests, in plain language. */
export function PermissionList({ showScopes = false, only, push = false }: PermissionListProps) {
  const id = useId();
  const core = [...MICROSOFT_PERMISSIONS.filter((p) => !p.feature), ...(push ? [PUSH] : [])];
  if (only === "core") {
    return (
      <ul className="permission-list">
        <Items permissions={core} showScopes={showScopes} />
      </ul>
    );
  }
  const optional = MICROSOFT_PERMISSIONS.filter((p) => p.feature);
  return (
    <div className="permission-groups">
      <p id={`${id}-core`} className="permission-groups__heading">
        For mail
      </p>
      <ul className="permission-list" aria-labelledby={`${id}-core`}>
        <Items permissions={core} showScopes={showScopes} />
      </ul>
      <p id={`${id}-optional`} className="permission-groups__heading">
        For calendar, contacts, tasks and time zone (optional)
      </p>
      <ul className="permission-list" aria-labelledby={`${id}-optional`}>
        <Items permissions={optional} showScopes={showScopes} />
      </ul>
    </div>
  );
}
