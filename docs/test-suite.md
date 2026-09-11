# The test suite

Read before running anything but `cargo test --release --workspace`, before regenerating a fixture,
and before adding a test. Two tiers: the default one, and `slow-tests`. Everything else is a name
filter or an environment variable.

## Commands

```bash
cargo test --release --workspace                        # the default tier, ~30 s
cargo test --workspace                                  # the same, dev profile, ~95 s
cargo test --release --workspace --features slow-tests   # everything, about an hour

# The two pre-push gates. Run both after every major work item.
cargo test --release -p poke-agent --features slow-tests --lib -- full_playthrough
cargo test --release -p poke-agent --features slow-tests --lib -- godmode_run

cargo test --release -p poke-agent --features slow-tests --lib -- hall_of_fame
cargo test --release -p poke-agent --features slow-tests --lib -- soak --nocapture
cargo test --release -p poke-agent --features slow-tests --lib -- coverage_walk --nocapture
cargo test --release --workspace --features slow-tests -- probe_ --ignored --nocapture
```

- Always `--release`: these tests emulate every frame. The dev profile works — `Cargo.toml` gives
  `gb` and `poke-agent` `opt-level = 3` and every dependency `opt-level = 2`, without which the
  agent's frames overflow the harness's stack and the mock endpoint is slow enough to change what
  the emulator has done by the time a turn is answered.
- Agent and policy tracing goes to stdout, so add `--nocapture` when you want it.
- A filter that matches nothing prints `0 passed` and exits 0. Check the count.

## The two gates

- `full_playthrough` (~235 s) is the scripted route walking the whole of Kanto to eight badges. It
  is the only test that proves the `PolicyStep` legs compose, and it is what keeps
  `--policy deterministic` honest.
- `godmode_run` (~40 s) plays a fresh save to the Hall of Fame through the deployed `LlmPolicy`, the
  worker and the wire, against an in-process mock endpoint, with a god party and a battle script so
  no battle costs a request. It never touches `PolicyStep`.
- Neither replaces the other: they gate different halves, and the leg tier is not a substitute for
  either.

## Fixtures

- Every leg snapshots its end state for the next leg to start from, and the write is a no-op unless
  `GB_REGEN_FIXTURES=1` — otherwise every run silently rewrites the next run's inputs. Regenerate in
  chain order, one leg at a time:
  `GB_REGEN_FIXTURES=1 cargo test --release -p poke-agent --features slow-tests --lib -- can_clear_ss_anne --exact`.
- `at-cerulean.bin` is the root every leg fixture descends from; `regen_at_cerulean_fixture` re-cuts
  it from a fresh save. A `PartyRef` that does not resolve waits for ever rather than failing, so a
  party change on the mainline shows up as a row of legs going red at once.
- Cut a fixture where the mainline stands — a leg that opens with `enter(X)` needs a root saved
  *inside* the previous building, not in the street — and where the party is healed.
- A fixture's name says where it is cut; a leg that walks further than its name is two tests and two
  fixtures (`vr1f-strength`, `vr2f-ladder`).
- The mid-leg cuts (`seafoam-b3f`, `vr3f-strength`) and the terrace cuts (`split-cerulean`,
  `split-celadon`, `pocket-route14`, `route21-islands`, `safari-west-shelf`) are not in the chain, so
  they are free to re-cut — but each carries *which tile*, and one re-cut somewhere tidier is a test
  of nothing. Each cutter asserts the property it has to keep.
- The four `branch-*.bin` are cut one decision before a choice that is exclusive per save, by the
  `regen_*` tests in `integration_tests::branch_points`. None may be cut inside a trainer's line of
  sight: a trainer walks up on the map's own tick, so such a state restores straight into a battle.
- The four battle fixtures are read twice over — by `stalls` for the jam each was cut in and by
  `battle_refusals` for the refusal each one contains — so a re-cut has to keep *which battle* as
  well as which jam.
- `soak-*.bin` are not in the chain and are re-cut wholesale by `regen_soak_checkpoints`.
- `every_committed_fixture_decodes` (default tier, in `gb`) loads all of them, so a save-state layout
  break fails in two seconds rather than an hour into `slow-tests`.

## Writing a test

- A leg's game-time budget is sized to encounters, not walking. An `enter_at` naming the wrong
  landing only fails from a cold fixture, because the mainline re-routes over a world graph the
  leg's fresh agent has never observed.
- A helper that reads published events after a `pump_*` must use `events_until`, not `try_iter`:
  `try_iter` returns what has arrived, which is a race.
- `soak` is gated as a module rather than with `#[ignore]`, so it never appears in the ignored list —
  that list is a backlog of blocked tests and a fuzzer is not one.
- Every `#[ignore]` names its reason in a few words. The probes, the fixture cutters and the
  benchmarks are all `#[ignore]`d under `slow-tests`, because their pass/fail is not a signal.
- `gb/tools/blip-golden/build.sh` regenerates the resampler's golden vectors from the vendored C++,
  after `capture_golden_input` if the input needs refreshing.
