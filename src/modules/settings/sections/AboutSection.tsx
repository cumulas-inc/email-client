import { ExternalLink, KeyRound, Scale, ShieldCheck } from "lucide-react";
import { useId } from "react";

import { useBackend } from "../../../api/context";
import { useAppInfo } from "../../../api/queries";
import { LogoMark } from "../../../app/LogoMark";
import { PaneSpinner } from "../../../components/Spinner";
import { Callout } from "../Fields";

const LICENSE_URL = "https://www.gnu.org/licenses/gpl-3.0.html";

export function AboutSection() {
  const backend = useBackend();
  const { data: info } = useAppInfo();
  const id = useId();
  if (!info) return <PaneSpinner />;
  return (
    <>
      <div className="about-hero">
        <span className="about-hero__mark">
          <LogoMark size={56} />
        </span>
        <p className="about-hero__name">Cumulas Mail</p>
        <p className="about-hero__version selectable">Version {info.version}</p>
        <p className="about-hero__tagline">Mail, calendar, contacts and tasks for Linux.</p>
      </div>

      {!info.persistentSecrets && (
        <Callout tone="warning" role="status" title="Your sign-ins can't be saved">
          <p>
            No keyring service was found; you&apos;ll need to sign in again each time the app
            starts. Install GNOME Keyring or KWallet.
          </p>
        </Callout>
      )}

      <section className="settings-group" aria-labelledby={`${id}-privacy`}>
        <h4 id={`${id}-privacy`} className="settings-group__title">
          Privacy
        </h4>
        <div className="settings-card">
          <div className="about-row">
            <ShieldCheck size={18} aria-hidden="true" className="about-row__icon" />
            <div className="about-row__text">
              <p className="about-row__title">Your data stays on this computer</p>
              <p>
                Mail, events, contacts and tasks are stored on this computer and sent only to your
                mail providers — Microsoft, or the servers you set up. There&apos;s no telemetry, no
                analytics and no Cumulas account.
              </p>
            </div>
          </div>
          {info.persistentSecrets && (
            <div className="about-row">
              <KeyRound size={18} aria-hidden="true" className="about-row__icon" />
              <div className="about-row__text">
                <p className="about-row__title">Passwords are kept in your keyring</p>
                <p>
                  Passwords and sign-in tokens are stored in your desktop&apos;s keyring, never in
                  Cumulas Mail&apos;s own files.
                </p>
              </div>
            </div>
          )}
        </div>
      </section>

      <section className="settings-group" aria-labelledby={`${id}-license`}>
        <h4 id={`${id}-license`} className="settings-group__title">
          License
        </h4>
        <div className="settings-card">
          <div className="about-row">
            <Scale size={18} aria-hidden="true" className="about-row__icon" />
            <div className="about-row__text">
              <p className="about-row__title">
                Free software
                <span className="about-license-id">GPL-3.0-or-later</span>
              </p>
              <p>
                You can use, study, share and change Cumulas Mail under the terms of the GNU General
                Public License, version 3 or later. It comes with no warranty.
              </p>
            </div>
            <button
              type="button"
              className="btn btn--sm about-row__action"
              onClick={() => void backend.openLink(LICENSE_URL)}
            >
              Read the license
              <ExternalLink size={14} aria-hidden="true" />
            </button>
          </div>
        </div>
      </section>

      <p className="about-trademarks">
        Microsoft 365 and Outlook.com are trademarks of Microsoft Corporation. Cumulas Mail is an
        independent project, not affiliated with or endorsed by Microsoft.
      </p>
    </>
  );
}
