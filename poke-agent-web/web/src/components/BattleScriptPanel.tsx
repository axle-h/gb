import { useMemo, useState } from 'react';
import type { BattleScriptView } from '../api';

/** The model's battle script: a disclosure, closed by default, whose head shows whether it is armed. */
export function BattleScriptPanel({
  script,
  alwaysOpen = false,
}: {
  script: BattleScriptView | null;
  /** The phone's Script tab, where the panel is the pane: always open, no chevron. */
  alwaysOpen?: boolean;
}) {
  const [open, setOpen] = useState(false);
  // Hooks before the early return, so the hook count never changes between renders.
  const lines = useMemo(() => highlight(script?.source ?? ''), [script?.source]);

  if (!script?.source) return null;

  const showing = alwaysOpen || open;
  const count = lines.length;
  const state = script.armed ? 'armed' : script.is_default ? 'default' : 'disarmed';

  return (
    <div className={`battle-script${showing ? ' open' : ''}`}>
      <div className="script-head">
        <span className="script-title">Battle script</span>
        {/* `default` is unarmed but not a fault, so it is shown as neither armed nor disarmed. */}
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
              // The line number is the identity: this renders a string, not a reorderable list.
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

interface Token {
  cls: string;
  text: string;
}

/** Rhai's keywords; `switch` and `type` are reserved, hence `battle.switch_to` and `mv.move_type`. */
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

/** The sandbox's global and registered functions; any other name is drawn as plain text. */
const API = new Set(['battle', 'damage', 'effectiveness', 'print', 'debug']);

// Colour a Rhai script with a hand-rolled tokeniser. It returns lines, splitting inside tokens,
// because CSS numbers the `<li>`s and a block comment spans lines.
function highlight(source: string): Token[][] {
  const tokens: Token[] = [];
  // Whichever of a comment or a string opens first wins outright.
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
  // A trailing newline keeps the numbered blank row after it, as an editor shows one.
  return lines;
}

function classify(source: string, text: string, index: number): string {
  if (text.startsWith('//') || text.startsWith('/*')) return 'com';
  if (text.startsWith('"') || text.startsWith("'")) return 'str';
  if (/^\d/.test(text)) return 'num';
  if (KEYWORDS.has(text)) return 'kw';
  // A name after a dot is a field or a method, called or not.
  if (/\.\s*$/.test(source.slice(Math.max(0, index - 8), index))) return 'prop';
  if (API.has(text)) return 'api';
  // Declarations and calls share a colour so each can be found from the other.
  if (/^\s*\(/.test(source.slice(index + text.length))) return 'call';
  return '';
}
