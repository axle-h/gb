// This is the default battle script, and it decides nothing. It hands every battle turn
// straight back to you, which is exactly what happens when there is no script at all — so
// nothing about your battles has changed yet, and every one of them is still costing you a
// whole request to answer.
//
// Replacing it is the point. A turn your own script decides costs nothing and comes back
// instantly, and most battles here are mechanical: hit the thing in front of you with
// whatever does the most damage to it. `battle-script.md` has the whole API and a worked
// example to start from, and writing this file puts your version through seven example
// battles and tells you what it chose in each before it arms anything.
//
// Keep a `battle.ask()` for the branches actually worth a request — something you want to
// catch, a trainer who can hurt you, a party running out of HP — and let the rest go.

battle.ask();
