import type { Entry } from '../api';

export type ToolEntry = Extract<Entry, { type: 'tool' }>;

/** One tool call as a sentence; an unlisted tool falls back to its name and compact arguments. */
export function describeTool(entry: ToolEntry, turnKind: string | undefined): string {
  const args = parseArguments(entry.arguments);
  const text = (key: string): string | undefined => {
    const value = args[key];
    return typeof value === 'string' && value !== '' ? value : undefined;
  };
  switch (entry.name) {
    case 'read_file':
      return describeRead(text('path'));
    case 'write_file': {
      const path = text('path') ?? '';
      const content = typeof args.content === 'string' ? args.content : '';
      if (path.startsWith('issues/')) return content ? `Reported: ${content}` : 'Reported a problem with the agent';
      if (path === 'battle-script.rhai') {
        if (content.trim() === '') return 'Stopped scripting its battles';
        const lines = content.trim().split('\n').length;
        return `Wrote a battle script (${lines} line${lines === 1 ? '' : 's'})`;
      }
      if (path === 'plan.md') return 'Rewrote its plan';
      return `Wrote ${path || 'a file'}`;
    }
    case 'edit_file': {
      const path = text('path');
      if (path === 'plan.md') return 'Edited its plan';
      if (path === 'battle-script.rhai') return 'Edited its battle script';
      return path ? `Edited ${path}` : 'Edited a file';
    }
    case 'choose_action':
      return describeAction(text('id'), turnKind);
    // The tools below are an older conversation's, which `/api/history` still replays.
    case 'read_map':
      return 'Read the map';
    case 'read_party':
      return 'Read the party';
    case 'read_bag':
      return 'Read the bag';
    case 'read_battle':
      return 'Read the battle';
    case 'read_route': {
      const to = text('to');
      return to ? `Asked the way to ${to}` : 'Asked the way';
    }
    case 'screenshot':
      return 'Looked at the screen';
    // `todo_add` still appears in older transcripts.
    case 'todo_add':
    case 'todo_set': {
      const item = text('text');
      if (item) return args.id === undefined ? `Planned: ${item}` : `Revised plan item ${args.id}: ${item}`;
      // No text with an id is the delete overload older transcripts carry.
      return args.id === undefined ? 'Added to the plan' : `Dropped plan item ${args.id}`;
    }
    case 'todo_delete':
      return args.id === undefined ? 'Dropped a plan item' : `Dropped plan item ${args.id}`;
    case 'todo_complete':
      return args.id === undefined ? 'Ticked something off' : `Ticked off plan item ${args.id}`;
    case 'get_battle_script_docs':
      return 'Read how to script a battle';
    case 'read_battle_script':
      return 'Read its battle script';
    // The size, not the script, which would take over the log; the source is one click away.
    case 'set_battle_script': {
      const script = text('script');
      if (!script) return 'Stopped scripting its battles';
      const lines = script.trim().split('\n').length;
      return `Wrote a battle script (${lines} line${lines === 1 ? '' : 's'})`;
    }
    case 'choose_battle_action': {
      const id = text('id');
      return id ? `Chose ${id}` : 'Chose an action';
    }
    case 'use_field_move': {
      const move = text('move');
      return move ? `Used ${move}` : 'Used a field move';
    }
    case 'press_buttons': {
      const buttons = Array.isArray(args.buttons) ? args.buttons.join(', ') : undefined;
      return buttons ? `Pressed ${buttons}` : 'Pressed buttons';
    }
    // Written to be read by a person, so the message is the row.
    case 'report_issue': {
      const message = text('message');
      return message ? `Reported: ${message}` : 'Reported a problem with the agent';
    }
    case 'set_nickname': {
      const name = text('name');
      // Omitting the argument is the ordinary answer here, not a missing one.
      return name ? `Named it ${name}` : 'Kept the default name';
    }
    case 'buy_item': {
      const item = text('item');
      const quantity = typeof args.quantity === 'number' ? ` ×${args.quantity}` : '';
      return item ? `Bought ${item}${quantity}` : 'Bought nothing';
    }
    case 'forget_move':
      return args.slot === undefined ? 'Declined the new move' : `Forgot the move in slot ${args.slot}`;
    case 'mimic_move':
      return args.slot === undefined ? 'Copied an enemy move' : `Copied the enemy's move in row ${args.slot}`;
    case 'wait':
      return args.ticks === undefined ? 'Waited' : `Waited ${args.ticks} ticks`;
    default: {
      const rest = compact(entry.arguments);
      return rest ? `${entry.name} ${rest}` : entry.name;
    }
  }
}

/** A `read_file` path as a sentence. */
function describeRead(path: string | undefined): string {
  if (!path) return 'Read a file';
  const fixed: Record<string, string> = {
    'map.png': 'Read the map',
    'screen.png': 'Looked at the screen',
    'party.md': 'Read the party',
    'bag.md': 'Read the bag',
    'pc.md': 'Read the PC',
    'plan.md': 'Read its plan',
    'battle-script.rhai': 'Read its battle script',
    'battle-script.md': 'Read how to script a battle',
    'guide/current.md': 'Read the walkthrough',
    'routes.md': 'Listed the maps it has walked',
    'issues/': 'Listed the issues it has filed',
  };
  if (fixed[path]) return fixed[path];
  if (path.startsWith('routes/')) return `Asked the way to ${path.slice('routes/'.length).replace(/\.md$/, '')}`;
  if (path.startsWith('guide/')) return `Read the walkthrough's ${path.slice('guide/'.length)}`;
  return `Read ${path}`;
}

/** A `choose_action` id as a sentence: its prefix says what kind of action it is, except on a naming turn. */
export function describeAction(id: string | undefined, turnKind: string | undefined): string {
  if (!id) return 'Chose an action';
  // The naming turn's id is the name itself, and `NONE` declines.
  if (turnKind === 'nickname' && !/^wait(\*|$)/i.test(id)) {
    return id.trim().toUpperCase() === 'NONE' ? 'Kept the species name' : `Named it ${id.trim()}`;
  }
  const [base, count] = id.split('*');
  if (base === 'wait') return count ? `Waited ${count} ticks` : 'Waited';
  if (base === 'leave') return 'Bought nothing';
  if (base === 'keep') return 'Declined the new move';
  if (base === 'take_over') return 'Took this battle over from its script';
  if (base.startsWith('press:')) return `Pressed ${base.slice('press:'.length).split(',').join(', ')}`;
  if (base.startsWith('buy:')) return `Bought ${base.slice('buy:'.length)}${count ? ` ×${count}` : ''}`;
  if (base.startsWith('forget:')) return `Forgot the move in slot ${base.slice('forget:'.length)}`;
  if (base.startsWith('mimic:')) return `Copied the enemy's move in row ${base.slice('mimic:'.length)}`;
  const field = describeFieldRow(base, count);
  return field ?? `Chose ${id}`;
}

/** An overworld row done where the player stands: `fly:`, `item:`, `teach:`, the PC's and the rest. */
function describeFieldRow(base: string, count: string | undefined): string | undefined {
  const [head, ...parts] = base.split(':');
  const rest = parts.join(':');
  const [thing, on] = rest.split('@');
  let slot = '';
  if (on?.endsWith(':stay')) slot = ` on slot ${on.slice(0, -':stay'.length)}, kept from evolving`;
  else if (on !== undefined && /^\d+$/.test(on)) slot = ` on slot ${on}`;
  else if (on !== undefined) slot = ` on ${on.split(':').pop()}`;
  const many = count ? ` ×${count}` : '';
  switch (head) {
    case 'dig': return 'Dug out';
    case 'teleport': return 'Teleported';
    case 'flash': return 'Used Flash';
    case 'fly': return `Flew to ${rest}`;
    case 'lift': return `Rode the lift to ${rest}`;
    case 'prize': return `Bought the ${rest} prize`;
    case 'lead': return `Made slot ${rest} the leader`;
    case 'toss': return `Tossed ${rest}${many}`;
    case 'item': return `Used ${thing}${slot}`;
    case 'teach': return `Taught ${thing}${slot}`;
    case 'evolve': return `Used ${thing}${slot}`;
    case 'daycare': return rest === 'withdraw' ? 'Collected a Pokémon from the Day Care' : `Boarded slot ${on} at the Day Care`;
    case 'pc':
      if (thing === 'box') return `Opened PC box ${count ?? '1'}`;
      if (thing.startsWith('store:')) return `Stored ${thing.slice('store:'.length)}${many} in the PC`;
      if (thing.startsWith('take:')) return `Took ${thing.slice('take:'.length)}${many} from the PC`;
      if (thing === 'deposit') return `Deposited slot ${on} in the PC`;
      if (thing === 'withdraw') return `Withdrew box slot ${on} from the PC`;
      if (thing === 'release') return `Released box slot ${on}`;
      return undefined;
    default: return undefined;
  }
}

/** The model's arguments, or an empty object — a call whose JSON will not parse is one to show raw. */
export function parseArguments(json: string): Record<string, unknown> {
  try {
    const parsed: unknown = JSON.parse(json.trim() || '{}');
    return typeof parsed === 'object' && parsed !== null ? (parsed as Record<string, unknown>) : {};
  } catch {
    return {};
  }
}

export function pretty(json: string): string {
  const trimmed = json.trim();
  try {
    return JSON.stringify(JSON.parse(trimmed), null, 2);
  } catch {
    return trimmed;
  }
}

/** A finished thought collapses to its word count: how long it deliberated. */
export function summarise(text: string): string {
  const trimmed = text.trim();
  if (trimmed === '') return 'thought about it';
  const words = trimmed.split(/\s+/).length;
  return `thought for ${words.toLocaleString()} word${words === 1 ? '' : 's'}`;
}

/** Arguments arrive as the model sent them, which may be pretty-printed across several lines. */
export function compact(json: string): string {
  const trimmed = json.trim();
  if (trimmed === '' || trimmed === '{}') return '';
  try {
    return JSON.stringify(JSON.parse(trimmed));
  } catch {
    return trimmed;
  }
}

// The gutter names the part of the game talking, not the `AgentEvent` variant, which is the `title`.
// The `text_box` arm is unreachable while `UNLOGGED` drops that kind.
export function category(kind: string): string {
  if (kind.startsWith('battle')) return 'battle';
  if (kind === 'text_box') return 'text';
  // The end of the game gets the badge strip's trophy.
  if (kind === 'hall_of_fame') return '🏆';
  return kind.includes('overworld') ? 'overworld' : kind.replace(/_/g, ' ');
}

/** A tool call that is not the turn's action, as the few words of a chip; the sentence is its `title`. */
export function toolChip(entry: ToolEntry): string {
  const args = parseArguments(entry.arguments);
  const path = typeof args.path === 'string' ? args.path : '';
  const name = path
    .replace(/^routes\//, 'way to ')
    .replace(/^guide\/current\.md$/, 'walkthrough')
    .replace(/^guide\//, 'guide ')
    .replace(/\.(md|png|rhai)$/, '')
    .replace(/\/$/, '');
  switch (entry.name) {
    case 'read_file':
      return name ? `📄 ${name}` : '📄 a file';
    case 'write_file':
    case 'edit_file':
      return name ? `✎ ${name}` : '✎ a file';
    default:
      return entry.name.replace(/_/g, ' ');
  }
}

/** Whether a tool call is the turn's move rather than looking something up. */
export function isAction(entry: ToolEntry): boolean {
  return ['choose_action', 'choose_battle_action', 'use_field_move', 'press_buttons', 'set_nickname', 'buy_item', 'forget_move', 'mimic_move', 'wait'].includes(entry.name);
}

/** A file written under `issues/` is meant for a person, so it reads as a sentence, never a chip. */
export function isIssue(entry: ToolEntry): boolean {
  if (entry.name === 'report_issue') return true;
  if (entry.name !== 'write_file') return false;
  const path = parseArguments(entry.arguments).path;
  return typeof path === 'string' && path.startsWith('issues/');
}
