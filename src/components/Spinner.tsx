import { LoaderCircle } from "lucide-react";

export function Spinner({ size = 20, label = "Loading" }: { size?: number; label?: string }) {
  return (
    <span className="spinner" role="status" aria-label={label}>
      <LoaderCircle size={size} className="spin" />
    </span>
  );
}

/** Centered spinner filling its container. */
export function PaneSpinner() {
  return (
    <div className="pane-spinner">
      <Spinner size={28} />
    </div>
  );
}
