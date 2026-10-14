import { useAppStore } from "../../app/store";
import { EmptyState } from "../../components/EmptyState";

export function Welcome() {
  const setAddAccountOpen = useAppStore((s) => s.setAddAccountOpen);
  return (
    <EmptyState
      title="Welcome to Cumulas Mail"
      description="Add an account to get started."
      action={
        <button type="button" className="btn btn--primary" onClick={() => setAddAccountOpen(true)}>
          Add account
        </button>
      }
    />
  );
}
