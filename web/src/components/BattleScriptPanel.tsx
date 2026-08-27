import { useMemo, useState } from 'react';
import type { BattleScriptView } from '../api';

/**
 * The program deciding the run's battle turns — the model's `set_battle_script`, on the page.
 *
 * The one panel here that shows something the model *wrote* rather than something it did. A scripted
 * battle is otherwise completely invisible from outside: it costs no request, produces no turn and
 * publishes no decision, so a viewer watching a run tear through Route 3 has no way of knowing
 * whether it is playing well or whether a program is playing for it. This is that program.
 *
 * ⚠️ **A disclosure rather than a panel, and the default is closed.** It is up to six kilobytes of
 * source that changes a handful of times in a playthrough, sitting under a plan and a party that
 * change every few turns — open by default it would push both off a laptop screen to show something
 * that is the same as it was an hour ago. The head is the part worth a permanent line, because
 * `armed` is a live fact: it is what says whether the battles going past are being decided here or
 * one paid request at a time.
 *
 * ⚠️ **`armed` and "there is a source" are separate questions and both are shown.** A script that
 * fails is **kept and disarmed** — the source stays because it is the thing the model has to edit —
 * so a panel that took a source as proof of a running script would say the opposite of what is
 * happening for the rest of the run. When it is disarmed the reason is the first thing in the box,
 * above the code, since that is what a reader is there to find out.
 */
export function BattleScriptPanel({
  script,
  alwaysOpen = false,
}: {
  script: BattleScriptView | null;
  /**
   * The phone's Script tab, where this panel *is* the pane. A disclosure that has to be opened after
   * choosing the tab that shows it is a click that asks the same question twice, so the tab answers
   * it and the chevron is not drawn.
   */
  alwaysOpen?: boolean;
}) {
  const [open, setOpen] = useState(false);
  // Hooks before the early return: a run that never sets a script would otherwise unmount them, and
  // a run that sets one mid-playthrough would change the hook count between two renders.
  const lines = useMemo(() => highlight(script?.source ?? ''), [script?.source]);

  // Nothing to show: a fresh run, a model that has not written one, or any policy that is not an LLM
  // and never will. Same rule as `PlanPanel` — an empty box on half the deployments is worse than no
  // box at all.
  if (!script?.source) return null;

  const showing = alwaysOpen || open;
  const count = lines.length;
  const state = script.armed ? 'armed' : script.is_default ? 'default' : 'disarmed';

  return (
    <div className={`battle-script${showing ? ' open' : ''}`}>
      <div className="script-head">
        <span className="script-title">Battle script</span>
        {/* Three states, not two. `default` is the run that has not written a script yet: a real
            source, not armed, nothing wrong with it — calling that "disarmed" would report a fault
            where there is none, and calling it "armed" would say the battles going past are free
            when the run is paying a full request for every one. */}
        <span className={`script-state ${state}`}>{state}</span>
        <span className="dim script-size">
          {count} line{count === 1 ? '' : 's'}
        </span>
        {!alwaysOpen && (
          <button
            className="script-toggle"
            onClick={() => setOpen((was) => !was)}
            aria-expanded={showing}
            title={showing ? 'hide the script' : 'show the script'}
          >
            {showing ? '▾' : '▸'}
          </button>
        )}
      </div>
      {showing && (
        <div className="script-body">
          {/* Above the code rather than below it: a disarmed script is still the thing being read,
              and the question a reader opens this with is why it stopped. */}
          {state === 'disarmed' && script.last_failure && (
            <p className="script-failure">
              <span className="mark" aria-hidden="true">
                ✗
              </span>{' '}
              {script.last_failure}
            </p>
          )}
          {state === 'default' && (
            <p className="script-failure quiet">
              The default script, which every run starts on. It decides nothing and hands every battle turn back to the
              model, so each one costs a request.
            </p>
          )}
          {state === 'disarmed' && !script.last_failure && (
            <p className="script-failure quiet">Written but not armed. Battle turns are being decided one at a time.</p>
          )}
          <ol className="code" aria-label="battle script source">
            {lines.map((tokens, index) => (
              // The index is the key because the line number *is* the identity here: this is a
              // rendering of a string, not a list of things that can be reordered.
              <li key={index}>
                {tokens.map((token, at) => (
                  <span key={at} className={token.cls}>
                    {token.text}
                  </span>
                ))}
              </li>
            ))}
          </ol>
        </div>
      )}
    </div>
  );
}

/** One coloured run of source. `cls` is `''` for the ordinary text between the interesting parts. */
interface Token {
  cls: string;
  text: string;
}

/**
 * Rhai's keywords.
 *
 * ⚠️ **`switch` and `type` are in here as keywords and are *not* the battle API's names**, which is
 * the whole reason `battle.switch_to` and `mv.move_type` are spelled the way they are: both words are
 * reserved by the parser, `switch` even in method position. Highlighting them as keywords in a script
 * that tried to use them is the honest picture — the script does not misbehave, it does not parse.
 */
const KEYWORDS = new Set([
  'as',
  'break',
  'catch',
  'const',
  'continue',
  'do',
  'else',
  'export',
  'false',
  'fn',
  'for',
  'if',
  'import',
  'in',
  'let',
  'loop',
  'private',
  'return',
  'switch',
  'this',
  'throw',
  'true',
  'try',
  'type',
  'until',
  'while',
]);

/**
 * The names this sandbox puts in front of the model: the one global it is given, and the functions
 * `battle_script::engine` registers on top of rhai's own.
 *
 * Deliberately short. It is not a symbol table and does not have to be: an unknown name is drawn as
 * ordinary text, which is what a reader wants for the model's own variables anyway.
 */
const API = new Set(['battle', 'damage', 'effectiveness', 'print', 'debug']);

/**
 * Colour one Rhai script, line by line.
 *
 * A hand-rolled tokeniser rather than a highlighting library, on the same argument the rest of this
 * page is built on: the whole SPA has two dependencies, and the alternative here is tens of
 * kilobytes of grammar to draw one file that is at most six. It is a tokeniser and not a parser —
 * it knows comments, strings, numbers, keywords and whether a name is being called or read, which is
 * all a reader needs to find their way around thirty lines.
 *
 * ⚠️ **Returns lines rather than a flat token list, because a block comment spans them.** The line
 * numbers are drawn by CSS from the `<li>`s, so a token carrying an embedded newline would number
 * the file wrongly from that point on — the split has to happen here, inside the token, rather than
 * on the source before it.
 */
function highlight(source: string): Token[][] {
  const tokens: Token[] = [];
  // The alternatives are ordered by how greedy they are, longest-lived first: a `//` inside a string
  // literal is not a comment, and a `"` inside a comment is not a string, so whichever opens first
  // has to win outright. One pass, one regex, no lookahead needed.
  const pattern = /\/\/[^\n]*|\/\*[\s\S]*?(?:\*\/|$)|"(?:[^"\\]|\\[\s\S])*"?|'(?:[^'\\]|\\[\s\S])*'?|\b\d[\d_]*(?:\.\d+)?\b|[A-Za-z_][A-Za-z0-9_]*/g;
  let at = 0;
  for (let match = pattern.exec(source); match !== null; match = pattern.exec(source)) {
    if (match.index > at) tokens.push({ cls: '', text: source.slice(at, match.index) });
    tokens.push({ cls: classify(source, match[0], match.index), text: match[0] });
    at = match.index + match[0].length;
  }
  if (at < source.length) tokens.push({ cls: '', text: source.slice(at) });

  const lines: Token[][] = [[]];
  for (const token of tokens) {
    const parts = token.text.split('\n');
    parts.forEach((part, index) => {
      if (index > 0) lines.push([]);
      if (part.length > 0) lines[lines.length - 1].push({ cls: token.cls, text: part });
    });
  }
  // A file that ends in a newline has a trailing empty line, which is a blank row under the last
  // statement with a number beside it. Every editor draws that; a code panel that swallowed it would
  // disagree with what the model wrote by one line.
  if (lines.length > 1 && lines[lines.length - 1].length === 0) lines.pop();
  return lines;
}

/** Which of the five colours a matched run gets. */
function classify(source: string, text: string, index: number): string {
  if (text.startsWith('//') || text.startsWith('/*')) return 'com';
  if (text.startsWith('"') || text.startsWith("'")) return 'str';
  if (/^\d/.test(text)) return 'num';
  if (KEYWORDS.has(text)) return 'kw';
  // A name reached through a dot is a field or a method — `mv.damage`, `battle.fight` — and it is
  // drawn as one whether or not it is called. That is what makes `battle.fight(mv)` read as one
  // thing rather than as an object, a dot and a function that happens to share the page with it.
  if (/\.\s*$/.test(source.slice(Math.max(0, index - 8), index))) return 'prop';
  if (API.has(text)) return 'api';
  // `fn healthiest(party, active)` and every call of it. The declaration and the call want the same
  // colour: the point of the colour is that the reader can find one from the other.
  if (/^\s*\(/.test(source.slice(index + text.length))) return 'call';
  return '';
}
