import { useState } from 'react';
import type { PlanVersion } from '../api';

/** The model's `plan.md` as it wrote it, newest first, with every earlier version a step away. */
export function PlanPanel({ plan }: { plan: PlanVersion[] }) {
  // How many versions back from the newest; 0 follows the newest as new ones arrive.
  const [back, setBack] = useState(0);
  // Nothing published yet, a policy with no plan, or one never written.
  if (!plan.some((version) => version.text.trim() !== '')) return null;

  const index = Math.max(0, plan.length - 1 - Math.min(back, plan.length - 1));
  const shown = plan[index];
  const when = shown.at ? new Date(shown.at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : null;

  return (
    <div className="plan">
      <div className="plan-head">
        <span className="plan-title">Plan</span>
        <span className="dim">
          {index === plan.length - 1 ? 'current' : `version ${index + 1} of ${plan.length}`}
          {when ? ` · ${when}` : ''}
        </span>
        {plan.length > 1 && (
          <span className="plan-steps">
            <button disabled={index === 0} onClick={() => setBack(plan.length - index)} title="an earlier version">
              ‹
            </button>
            <button disabled={index === plan.length - 1} onClick={() => setBack(plan.length - 2 - index)} title="a later version">
              ›
            </button>
          </span>
        )}
      </div>
      {shown.text.trim() === '' ? <p className="plan-text dim">(empty)</p> : <pre className="plan-text">{shown.text}</pre>}
    </div>
  );
}
