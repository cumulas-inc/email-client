import { Dialog } from "../../components/Dialog";
import { useAppStore } from "../../app/store";

export function AddAccountDialog() {
  const { addAccountOpen, setAddAccountOpen } = useAppStore();
  return (
    <Dialog open={addAccountOpen} onClose={() => setAddAccountOpen(false)} title="Add account">
      <p>Coming soon.</p>
    </Dialog>
  );
}
