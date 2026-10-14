import { CalendarDays, CircleCheckBig, LockKeyhole, Mail, Users } from "lucide-react";

import { AccountKindOptions } from "../settings/AccountKindOptions";
import { openAddAccount } from "../settings/addAccountStore";

import "./onboarding.css";

const FEATURES = [
  { label: "Mail", icon: <Mail size={16} /> },
  { label: "Calendar", icon: <CalendarDays size={16} /> },
  { label: "Contacts", icon: <Users size={16} /> },
  { label: "Tasks", icon: <CircleCheckBig size={16} /> },
];

/** First-run screen, shown while there are no accounts. */
export function Welcome() {
  return (
    <div className="welcome">
      <div className="welcome__inner">
        <img className="welcome__logo" src="/icon.svg" alt="" width={72} height={72} />
        <h1 className="welcome__title">Welcome to Cumulas Mail</h1>
        <p className="welcome__lead">
          Your mail, calendar, contacts and tasks in one place — fast, private and made for Linux.
        </p>
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
          Your data stays on this computer and goes only to your mail provider. No tracking, no
          telemetry.
        </p>
      </div>
    </div>
  );
}
