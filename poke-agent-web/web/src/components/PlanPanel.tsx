import { useState } from 'react';
import { shownPlanIndex, type PlanVersion } from '../api';
import { Markdown } from './Markdown';

/** The model's `plan.md` as it wrote it, newest first, with every earlier version a step away. */
export function PlanPanel({ plan, onOlder }: { plan: PlanVersion[]; onOlder: () => void }) {
  // The version stepped to, by number, so a new one arriving leaves it in place; `null` follows the newest.
  const [viewing, setViewing] = useState<number | null>(null);
  // Nothing published yet, a policy with no plan, or one never written.
  if (!plan.some((version) => version.text.trim() !== '')) return null;

  const newest = plan[plan.length - 1];
  // Past the newest only when a new run started its history over.
  const following = viewing === null || viewing > newest.version;
  const index = shownPlanIndex(plan, following ? null : viewing);
  const shown = plan[index];
  const older = index > 0 || plan[0].version > 1;
  const when = shown.at ? new Date(shown.at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : null;

  const earlier = () => {
    if (index > 0) {
      setViewing(plan[index - 1].version);
      return;
    }
    // The page before this one: shown once it arrives, the oldest loaded until then.
    setViewing(plan[0].version - 1);
    onOlder();
  };
  const later = () => setViewing(index + 1 >= plan.length - 1 ? null : plan[index + 1].version);

  return (
    <div className="plan">
      <div className="plan-head">
        <span className="plan-title">Plan</span>
        <span className="dim">
          {following ? 'current' : `version ${shown.version} of ${newest.version}`}
          {when ? ` · ${when}` : ''}
        </span>
        {newest.version > 1 && (
          <span className="plan-steps">
            <button disabled={!older} onClick={earlier} title="an earlier version">
              ‹
            </button>
            <button disabled={following} onClick={later} title="a later version">
              ›
            </button>
          </span>
        )}
      </div>
      {shown.text.trim() === '' ? (
        <p className="plan-text dim">(empty)</p>
      ) : (
        <div className="plan-text">
          <Markdown source={shown.text} />
        </div>
      )}
    </div>
  );
}
