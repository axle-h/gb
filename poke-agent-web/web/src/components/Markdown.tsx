import { Fragment, type ReactNode } from 'react';

/** The model's own markdown, drawn as React elements and never as HTML, so nothing it writes can run.
    It covers what a plan is written in: headings, lists and checklists, quotes, rules, tables, fenced
    code and the inline marks. A link is drawn as its text, since its target is a file in the model's
    own filesystem rather than anywhere a browser can go. */
export function Markdown({ source }: { source: string }) {
  return <div className="markdown">{blocks(source.replace(/\r\n?/g, '\n').split('\n'))}</div>;
}

const FENCE = /^\s*(```|~~~)/;
const HEADING = /^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/;
const RULE = /^\s{0,3}([-*_])(\s*\1){2,}\s*$/;
const QUOTE = /^\s{0,3}>\s?(.*)$/;
const ITEM = /^(\s*)([-*+]|\d+[.)])\s+(.*)$/;
const TABLE_RULE = /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/;

function blocks(lines: string[]): ReactNode[] {
  const out: ReactNode[] = [];
  let at = 0;
  while (at < lines.length) {
    const line = lines[at];
    const key = out.length;
    if (line.trim() === '') {
      at += 1;
      continue;
    }

    const fence = FENCE.exec(line);
    if (fence) {
      const body: string[] = [];
      at += 1;
      while (at < lines.length && !lines[at].trimStart().startsWith(fence[1])) body.push(lines[at++]);
      at += 1;
      out.push(<pre key={key}>{body.join('\n')}</pre>);
      continue;
    }

    const heading = HEADING.exec(line);
    if (heading) {
      const Tag = `h${Math.min(heading[1].length + 2, 6)}` as 'h3';
      out.push(<Tag key={key}>{inline(heading[2])}</Tag>);
      at += 1;
      continue;
    }

    // Before lists, since `- - -` and `* * *` are rules and not items.
    if (RULE.test(line)) {
      out.push(<hr key={key} />);
      at += 1;
      continue;
    }

    if (QUOTE.test(line)) {
      const body: string[] = [];
      while (at < lines.length && QUOTE.test(lines[at])) body.push(QUOTE.exec(lines[at++])![1]);
      out.push(<blockquote key={key}>{blocks(body)}</blockquote>);
      continue;
    }

    if (ITEM.test(line)) {
      const start = at;
      // An item runs on through indented lines, and through a blank line when one of those follows.
      while (
        at < lines.length &&
        (ITEM.test(lines[at]) ||
          (lines[at].trim() !== '' && /^\s/.test(lines[at])) ||
          (lines[at].trim() === '' && at + 1 < lines.length && /^\s+\S/.test(lines[at + 1])))
      ) {
        at += 1;
      }
      out.push(<Fragment key={key}>{list(lines.slice(start, at))}</Fragment>);
      continue;
    }

    if (line.includes('|') && at + 1 < lines.length && TABLE_RULE.test(lines[at + 1])) {
      const head = cells(line);
      at += 2;
      const rows: string[][] = [];
      while (at < lines.length && lines[at].includes('|') && lines[at].trim() !== '') rows.push(cells(lines[at++]));
      out.push(
        <div key={key} className="table">
          <table>
            <thead>
              <tr>
                {head.map((cell, index) => (
                  <th key={index}>{inline(cell)}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((row, index) => (
                <tr key={index}>
                  {head.map((_, column) => (
                    <td key={column}>{inline(row[column] ?? '')}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>,
      );
      continue;
    }

    // A paragraph keeps the model's line breaks: a plan is written a line at a time.
    const body: string[] = [];
    while (at < lines.length && lines[at].trim() !== '' && !startsBlock(lines, at)) body.push(lines[at++].trim());
    out.push(
      <p key={key}>
        {body.map((text, index) => (
          <Fragment key={index}>
            {index > 0 && <br />}
            {inline(text)}
          </Fragment>
        ))}
      </p>,
    );
  }
  return out;
}

function startsBlock(lines: string[], at: number): boolean {
  const line = lines[at];
  return (
    FENCE.test(line) ||
    HEADING.test(line) ||
    RULE.test(line) ||
    QUOTE.test(line) ||
    ITEM.test(line) ||
    (line.includes('|') && at + 1 < lines.length && TABLE_RULE.test(lines[at + 1]))
  );
}

interface Item {
  indent: number;
  ordered: boolean;
  start: number;
  text: string[];
  children: string[];
}

/** One list from its lines; anything indented past an item's own marker is that item's nested content. */
function list(lines: string[]): ReactNode {
  const items: Item[] = [];
  for (const line of lines) {
    const item = ITEM.exec(line);
    const top = items.length === 0 ? null : items[0].indent;
    if (item && (top === null || item[1].length <= top)) {
      items.push({
        indent: item[1].length,
        ordered: /\d/.test(item[2]),
        start: parseInt(item[2], 10),
        text: [item[3]],
        children: [],
      });
    } else if (items.length > 0) {
      const last = items[items.length - 1];
      // Text straight under an item continues it; anything after a nested block belongs to that block.
      if (item || last.children.length > 0 || line.trim() === '') last.children.push(line);
      else last.text.push(line.trim());
    }
  }
  if (items.length === 0) return null;

  const ordered = items[0].ordered;
  const rendered = items.map((item, index) => {
    const check = /^\[([ xX])\]\s+/.exec(item.text[0]);
    const text = check ? [item.text[0].slice(check[0].length), ...item.text.slice(1)] : item.text;
    const done = check !== null && check[1] !== ' ';
    const nested = dedent(item.children);
    return (
      <li key={index} className={check ? (done ? 'task done' : 'task') : undefined}>
        {check && (
          <span className="tick" aria-label={done ? 'done' : 'to do'}>
            {done ? '✓' : ''}
          </span>
        )}
        {text.map((line, at) => (
          <Fragment key={at}>
            {at > 0 && <br />}
            {inline(line)}
          </Fragment>
        ))}
        {nested.length > 0 && blocks(nested)}
      </li>
    );
  });
  return ordered ? <ol start={items[0].start}>{rendered}</ol> : <ul>{rendered}</ul>;
}

function dedent(lines: string[]): string[] {
  const indents = lines.filter((line) => line.trim() !== '').map((line) => /^\s*/.exec(line)![0].length);
  const by = indents.length === 0 ? 0 : Math.min(...indents);
  return lines.map((line) => line.slice(Math.min(by, /^\s*/.exec(line)![0].length)));
}

function cells(line: string): string[] {
  return line
    .trim()
    .replace(/^\|/, '')
    .replace(/\|$/, '')
    .split('|')
    .map((cell) => cell.trim());
}

// Whichever mark opens first wins; code spans first, so nothing inside one is read as a mark.
const INLINE =
  /`([^`]+)`|\*\*(.+?)\*\*|__(.+?)__|~~(.+?)~~|\*(?!\s)(.+?)\*|(?<![\w])_(?!\s)(.+?)_(?![\w])|\[([^\]]+)\]\(([^)\s]*)\)/g;

function inline(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  let at = 0;
  for (const match of text.matchAll(INLINE)) {
    if (match.index > at) out.push(text.slice(at, match.index));
    const key = out.length;
    const [, code, strong, strong2, strike, em, em2, label, href] = match;
    if (code !== undefined) out.push(<code key={key}>{code}</code>);
    else if (strong !== undefined || strong2 !== undefined) out.push(<strong key={key}>{inline(strong ?? strong2)}</strong>);
    else if (strike !== undefined) out.push(<s key={key}>{inline(strike)}</s>);
    else if (em !== undefined || em2 !== undefined) out.push(<em key={key}>{inline(em ?? em2)}</em>);
    else
      out.push(
        <span key={key} className="link" title={href}>
          {inline(label)}
        </span>,
      );
    at = match.index + match[0].length;
  }
  if (at < text.length) out.push(text.slice(at));
  return out;
}
