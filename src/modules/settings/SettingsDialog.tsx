import { Dialog } from "../../components/Dialog";
import { useAppStore } from "../../app/store";

export function SettingsDialog() {
  const { settingsOpen, closeSettings } = useAppStore();
  return (
    <Dialog open={settingsOpen !== null} onClose={closeSettings} title="Settings">
      <p>Coming soon.</p>
    </Dialog>
  );
}
