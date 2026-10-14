import { KeyRound, Scale, ShieldCheck } from "lucide-react";

import { useBackend } from "../../../api/context";
import { useAppInfo } from "../../../api/queries";
import { PaneSpinner } from "../../../components/Spinner";
import { Callout } from "../Fields";

const LICENSE_URL = "https://www.gnu.org/licenses/gpl-3.0.html";

export function AboutSection() {
  const backend = useBackend();
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
        <div className="about-card">
          <Scale size={20} aria-hidden="true" className="about-card__icon" />
          <div>
            <p className="about-card__title">Free software</p>
            <p>
              You can use, study, share and change Cumulas Mail under the terms of the GNU General
              Public License, version 3 or later. It comes with no warranty.
            </p>
            <button
              type="button"
              className="btn btn--subtle btn--sm about-card__link"
              onClick={() => void backend.openLink(LICENSE_URL)}
            >
              Read the license
            </button>
          </div>
        </div>
      </div>

      <p className="about-trademarks">
        Microsoft 365 and Outlook.com are trademarks of Microsoft Corporation. Cumulas Mail is an
        independent project, not affiliated with or endorsed by Microsoft.
      </p>
    </>
  );
}
