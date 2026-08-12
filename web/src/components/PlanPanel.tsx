import type { TodoView } from '../api';

/**
 * **W6b / §10** — the model's own plan.
 *
 * The one panel here that is about the *player* rather than the game. The screen says where the run
 * is and the conversation says what it is thinking about right now; neither says what it is trying
 * to do, and over a playthrough of thousands of turns that is the thread a viewer is actually
 * following. It is the same list the model is sent every turn, which is what makes it honest —
 * nothing here is a rendering of something else.
 *
 * ⚠️ **Finished items stay, and they stay at the bottom.** The model's own copy shows only the last
 * few (a done item is answered noise in a context window); a viewer reads them as progress, so the
 * page keeps all of them and greys them out.
 */
export function PlanPanel({ plan }: { plan: TodoView[] }) {
  // Nothing published yet: a fresh run, or `--policy random`, which has no plan and never will.
  // Rendering an empty box in either case would be a panel that is permanently blank on half the
  // deployments.
  if (plan.length === 0) return null;

  const open = plan.filter((item) => !item.done);
  const done = plan.filter((item) => item.done);

  return (
    <div className="plan">
      <div className="plan-head">
        <span className="plan-title">Plan</span>
        <span className="dim">
          {open.length} to do{done.length > 0 ? ` · ${done.length} done` : ''}
        </span>
      </div>
      <ol className="plan-items">
        {[...open, ...done].map((item) => (
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
