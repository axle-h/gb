# Scripting your battle turns

A battle turn you answer costs a whole request. A battle turn your script answers costs nothing and
happens instantly. Most battles in this game are mechanical, so write the mechanical part down once
and keep your attention for the rest.

`set_battle_script` installs a script; it then decides every battle turn until it fails or you
replace it. Passing no `script` removes it and battle turns come back to you one at a time, which is
the default.

## The language

**Rhai.** Close to Rust and JavaScript. Everything below is checked against the engine that actually
runs your script, so it is what works rather than a family resemblance.

```rhai
let x = 3;                           // statements end with ;
x += 1;  x *= 2;                     // += -= *= /= %=
if a > b { ... } else { ... }        // no brackets round the condition
for mon in battle.party { ... }      // iterate an array
while i < 3 { i += 1; }              // also `loop { ... break; }`
let t = switch n { 1 => "one", _ => "other" };
// comments are // only
```

- **Operators**: `+ - * / %`, `== != < > <= >=`, `&& || !`. Integer division truncates (`7 / 2` is
  `3`); write `7.0 / 2.0` for `3.5`.
- **Nothing** is `()`. Test with `x == ()` or `x != ()`. Several things below can be `()`.
- **Strings**: double quotes. `"a" + 1` concatenates. `.len`, `.contains("x")`, `.to_upper()`,
  `.to_lower()`.
- **Arrays**: `a[0]`, `a.len`, `a.push(x)`, and closures — `a.filter(|p| !p.fainted)`,
  `a.map(|m| m.damage)`, `a.reduce(|sum, m| sum + m.damage, 0)`, `a.sort(|x, y| y.hp - x.hp)`.
- **Objects**: `#{ a: 1 }`. Read fields with `o.a` or `o["a"]`.
- **Functions**: `fn name(a, b) { ... }`. The last expression is the return value; `return x;` works.

Three things will trip you up, so they are worth reading twice:

1. **A `fn` cannot see `battle`, or any other variable outside itself.** `fn f() { battle.turn }` is
   a runtime error: `Variable not found: battle`. Pass what it needs in — `fn f(b) { b.turn }`,
   called as `f(battle)` — or use a closure `|x| ...`, which can see outer variables.
2. **`type` and `switch` are reserved words.** A move's type is `mv.move_type` and switching is
   `battle.switch_to(...)`. `mv.type` and `battle.switch(...)` do not even parse.
3. There is no file, network, clock or `eval` access, and nothing to import.

## What your script can read

One global, `battle`. Every field is a snapshot of this turn: reading it twice gives the same answer.

```rhai
battle.kind        // "wild" or "trainer"
battle.turn        // 1 on the first turn of this battle, then 2, 3, ...
battle.can_run     // false in a trainer battle. Check before battle.run()
battle.ghost       // Pokemon Tower without the Silph Scope: only battle.run() works
battle.trapped     // true if Wrap, Bind, Fire Spin or Clamp has hold of you
battle.catch_rate  // the foe's live catch rate, 0-255. Higher is easier to catch

battle.me          // a Pokemon: the one that is out
battle.foe         // a Pokemon: what you are fighting
battle.party       // an array of Pokemon, in slot order, including battle.me
battle.moves       // an array of Move: shorthand for battle.me.moves
battle.best_move   // a Move, or () when nothing you know can damage the foe, ghosts included
battle.bag         // an array of Item
```

### A Pokemon

`battle.me`, `battle.foe` and every element of `battle.party` are the same shape, with these fields
and no others.

| field | type | |
|---|---|---|
| `slot` | number | party slot, 0 to 5. `battle.foe.slot` is meaningless; do not use it |
| `name` | string | its nickname. For the foe, its species |
| `species` | string | `"Charmander"` |
| `level` | number | |
| `hp` | number | current HP |
| `max_hp` | number | |
| `hp_frac` | number | `hp / max_hp`, 0.0 to 1.0. **This is the one you want** for thresholds |
| `status` | string | `""` when healthy, else `"poisoned"` `"burned"` `"paralyzed"` `"asleep"` `"frozen"` |
| `types` | array of string | `["Fire"]`, or `["Water", "Flying"]` for a dual type |
| `fainted` | bool | |
| `moves` | array of Move | only the moves it has, so 1 to 4 entries |

### A Move

Every element of any `moves` array, and `battle.best_move`.

| field | type | |
|---|---|---|
| `slot` | number | 0 to 3 |
| `name` | string | `"Vine Whip"` |
| `move_type` | string | `"Grass"`. **Not `type`** |
| `power` | number | 0 for a status move |
| `accuracy` | number | out of 255 |
| `pp` | number | left now |
| `max_pp` | number | |
| `damage` | number | expected HP it takes off **the foe in front of you right now**, 0 if it cannot damage it |
| `effectiveness` | number | type multiplier against that foe: 0.0, 0.25, 0.5, 1.0, 2.0 or 4.0 |
| `usable` | bool | choosable this turn: has PP, not Disabled, not a ghost. An unusable move cannot be chosen |

`damage` and `effectiveness` are worked out for you, so you never need a type chart. They are
computed against the current foe for **every** Pokemon's moves, benched ones included, which is how
you find out who is worth switching in.

### An Item

Every element of `battle.bag`. Only things usable in a battle appear, and only while you have one.

| field | type | |
|---|---|---|
| `name` | string | as the game spells it: `"Potion"`, `"SuperPotion"`, `"HyperPotion"`, `"FullRestore"`, `"PokeBall"`, `"GreatBall"`, `"Antidote"` |
| `count` | number | how many you have |

`use_item` ignores case and punctuation, so `"poke ball"`, `"POKE BALL"` and `"PokeBall"` all reach
the same item. Comparing `item.name` yourself is exact, so loop over `battle.bag` and pass
`item.name` straight through rather than guessing at a spelling.

## Choosing

Exactly one of these, and **calling one ends the script immediately** — nothing after it runs.

```rhai
battle.fight(mv)         // a Move object, its name, or its slot number
battle.switch_to(mon)    // a Pokemon object, its name, or its slot number
battle.use_item(name)    // a string
battle.run()             // wild battles only
battle.ask()             // hand THIS turn to yourself, and stay installed
```

Reaching the end without calling one is a failure.

`battle.ask()` is how you keep the battles that matter. A gym leader, a rival, anything you are
trying to catch: ask, decide it yourself, and let the script have the routine ones.

`print(...)` anything you want to see; it comes back in the battle report.

## What happens when it goes wrong

If the script fails — it errors, runs too long, chooses nothing, or chooses something the game will
not accept — it is **disarmed** and that turn comes back to you with the reason. It stays on disk, so
`read_battle_script` shows it to you, you fix it, and `set_battle_script` installs it again. It will
never quietly keep failing.

`set_battle_script` runs your script against seven made-up battles before installing it and tells you
what it chose in each. Read that table: it is your only chance to notice that a rule you meant does
something else. It is **not a proof** — every one of the seven is turn 1 of a made-up battle, so
anything depending on `battle.turn`, on a Pokemon you do not have yet, or on the bag holding
something is only tested for real when it runs.

The mistakes that actually happen:

- `battle.fight(battle.best_move)` with no guard. `best_move` is `()` whenever nothing you know can
  damage the foe, and passing `()` is an error. Check it first.
- Fighting a ghost (Pokemon Tower, no Silph Scope). Only Run works there: `best_move` is `()`, no
  move is `usable` and `battle.ghost` is true.
- A `fn` that reaches for `battle`. See the language section.
- Assuming an item is in the bag. Loop over `battle.bag` and act on what is there.

## After a battle

Your next turn carries a report: what your script chose each turn, the HP on both sides, the game's
own words for what happened ("Enemy ODDISH used ABSORB! It's super effective!", "fainted", "gained
198 EXP"), and anything you printed. That is your feedback, and the only account you get of a battle
you were not asked about. If the script is losing Pokemon, wasting items or running from things
worth catching, change it.

## A worked example

```rhai
// Nothing that can hurt it? Find someone who can.
if battle.best_move == () {
    for mon in battle.party {
        if !mon.fainted && mon.slot != battle.me.slot {
            for mv in mon.moves {
                if mv.usable && mv.damage > 0 { battle.switch_to(mon); }
            }
        }
    }
    if battle.can_run { battle.run(); }
    battle.ask();
}

// Badly hurt: heal, switch, or get out. Biggest heal first.
if battle.me.hp_frac < 0.2 {
    for want in ["FullRestore", "HyperPotion", "SuperPotion", "Potion"] {
        for item in battle.bag {
            if item.name == want {
                print(battle.me.name + " at " + battle.me.hp + "/" + battle.me.max_hp);
                battle.use_item(item.name);
            }
        }
    }
    let fresh = ();
    for mon in battle.party {
        if !mon.fainted && mon.slot != battle.me.slot && mon.hp_frac > 0.6 { fresh = mon; }
    }
    if fresh != () { battle.switch_to(fresh); }
    if battle.can_run { battle.run(); }
}

// A weakened wild Pokemon worth having.
if battle.kind == "wild" && battle.foe.hp_frac < 0.25 {
    for item in battle.bag {
        if item.name == "PokeBall" {
            print("throwing a ball at " + battle.foe.species);
            battle.use_item(item.name);
        }
    }
}

battle.fight(battle.best_move);
```
