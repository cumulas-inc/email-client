import { KeyRound, ShieldCheck } from "lucide-react";

import { useAppInfo } from "../../../api/queries";
import { PaneSpinner } from "../../../components/Spinner";
import { Callout } from "../Fields";

export function AboutSection() {
  const { data: info } = useAppInfo();
  if (!info) return <PaneSpinner />;
  return (
    <>
      <div className="about-hero">
        <img src="/icon.svg" alt="" width={56} height={56} />
        <div>
          <p className="about-hero__name">Cumulas Mail</p>
          <p className="about-hero__version selectable">Version {info.version}</p>
          <p className="about-hero__tagline">Mail, calendar, contacts and tasks for Linux.</p>
        </div>
      </div>

      {!info.persistentSecrets && (
        <Callout tone="warning" role="status" title="Your sign-ins can't be saved">
          <p>
            No keyring service was found; you&apos;ll need to sign in again each time the app
            starts. Install GNOME Keyring or KWallet.
          </p>
        </Callout>
      )}

      <div className="about-cards">
        <div className="about-card">
          <ShieldCheck size={20} aria-hidden="true" className="about-card__icon" />
          <div>
            <p className="about-card__title">Your data stays on this computer</p>
            <p>
              Mail, events, contacts and tasks are stored on this computer and sent only to your
              mail providers — Microsoft, or the servers you set up. There&apos;s no telemetry, no
              analytics and no Cumulas account.
            </p>
          </div>
        </div>
        {info.persistentSecrets && (
          <div className="about-card">
            <KeyRound size={20} aria-hidden="true" className="about-card__icon" />
            <div>
              <p className="about-card__title">Passwords are kept in your keyring</p>
              <p>
                Passwords and sign-in tokens are stored in your desktop&apos;s keyring, never in
                Cumulas Mail&apos;s own files.
              </p>
            </div>
          </div>
        )}
      </div>
    </>
  );
}
