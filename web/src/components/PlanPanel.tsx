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
 * ⚠️ **In the model's own order, finished items in place.** This used to render open items first
 * and finished ones after, which reordered the list the moment anything was ticked off: an item
 * completed in the middle jumped to the bottom and the numbering stopped matching what the model
 * had written. A plan is a sequence, and re-sorting someone else's sequence for them is a way to
 * make it say something they did not. Done items are greyed rather than moved, and the list is
 * capped at `todo::MAX_ITEMS` including them, so there is no tail to push out of the way.
 */
export function PlanPanel({ plan }: { plan: TodoView[] }) {
  // Nothing published yet: a fresh run, or a policy that is not an LLM, which has no plan and
  // never will. Rendering an empty box in either case would be a panel that is permanently blank
  // on half the deployments.
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
