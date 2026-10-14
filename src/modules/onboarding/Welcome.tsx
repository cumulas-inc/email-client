import { CalendarDays, CircleCheckBig, LockKeyhole, Mail, Users } from "lucide-react";

import { LogoMark } from "../../app/LogoMark";
import { AccountKindOptions } from "../settings/AccountKindOptions";
import { openAddAccount } from "../settings/addAccountStore";

import "./onboarding.css";

const ICON = { size: 16, strokeWidth: 1.75 };

const FEATURES = [
  { label: "Mail", icon: <Mail {...ICON} /> },
  { label: "Calendar", icon: <CalendarDays {...ICON} /> },
  { label: "Contacts", icon: <Users {...ICON} /> },
  { label: "Tasks", icon: <CircleCheckBig {...ICON} /> },
];

/** First-run screen, shown while there are no accounts. */
export function Welcome() {
  return (
    <div className="welcome">
      <div className="welcome__card">
        <div className="welcome__mark">
          <LogoMark size={84} />
        </div>
        <h1 className="welcome__title">Welcome to Cumulas Mail</h1>
        <p className="welcome__lead">One app for your inbox and your week.</p>
        <ul className="welcome__features" aria-label="Included">
          {FEATURES.map((f) => (
            <li key={f.label} className="welcome__feature">
              <span aria-hidden="true">{f.icon}</span>
              {f.label}
            </li>
          ))}
        </ul>

        <section className="welcome__choices" aria-labelledby="welcome-choose">
          <h2 id="welcome-choose" className="welcome__choose">
            Add your first account
          </h2>
          <AccountKindOptions variant="cards" onChoose={(step) => openAddAccount({ step })} />
        </section>

        <p className="welcome__privacy">
          <LockKeyhole size={14} aria-hidden="true" />
          Cumulas Mail connects only to your mail provider. No tracking, no telemetry.
        </p>
      </div>
    </div>
  );
}
