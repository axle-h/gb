import type { TodoView } from '../api';

/** The model's plan in its own order; done items are greyed in place, never moved. */
export function PlanPanel({ plan }: { plan: TodoView[] }) {
  // Nothing published yet, or a policy with no plan.
  if (plan.length === 0) return null;

  const open = plan.filter((item) => !item.done).length;
  const done = plan.length - open;

  return (
    <div className="plan">
      <div className="plan-head">
        <span className="plan-title">Plan</span>
        <span className="dim">
          {open} to do{done > 0 ? ` · ${done} done` : ''}
        </span>
      </div>
      <ol className="plan-items">
        {plan.map((item) => (
          <li key={item.id} className={item.done ? 'done' : undefined}>
            <span className="tick" aria-hidden="true">
              {item.done ? '✓' : '·'}
            </span>
            <span className="what">{item.text}</span>
          </li>
        ))}
      </ol>
    </div>
  );
}
