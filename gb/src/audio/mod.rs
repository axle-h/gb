use bincode::{Decode, Encode};
use frame_sequencer::{FrameSequencer, FrameSequencerEvent};
use blip::BlipStereo;
use master_volume::MasterVolume;
use square_channel::SquareWaveChannel;
use crate::audio::noise_channel::NoiseChannel;
use crate::audio::panning::Panning;
use crate::audio::sample::AudioSample;
use crate::audio::wave_channel::WaveChannel;
use crate::cycles::MachineCycles;
use crate::divider::DividerClocks;
use crate::savestate::{labels, SectionReader, SectionWriter};

pub mod panning;
pub mod master_volume;
pub mod sweep;
pub mod length;
pub mod volume;
pub mod square_channel;
pub mod frame_sequencer;
pub mod sample;
pub mod dac;
pub mod wave_channel;
pub mod noise_channel;
pub mod blip;
mod timer;
#[cfg(test)]
mod reference;

pub const GB_SAMPLE_RATE: usize = 1048576; // Game Boy native audio frequency

#[derive(Debug, Clone)]
pub struct Audio {
    enabled: bool,
    panning: Panning,
    master_volume: MasterVolume,
    frame_sequencer: FrameSequencer,
    channel1: SquareWaveChannel,
    channel2: SquareWaveChannel,
    channel3: WaveChannel,
    channel4: NoiseChannel,
    /// Band-limited synthesis and resampling to the sink's rate. Also supplies the DC blocker that
    /// used to be a separate `CapacitanceFilter` — see [`blip::DEFAULT_BASS_HZ`].
    output: BlipStereo,
    /// Length in M-cycles of the instruction the CPU is executing. Set once per instruction; see
    /// [`Audio::set_instruction_length`]. Transient, so it is excluded from `PartialEq` and from the
    /// `apu` save-state section, exactly as `output` is.
    access_machine_cycles: u8,
    /// The mixer's current output level (C4). Derived from the channels, the panning and the
    /// master volume, so — like `output` — it is neither serialised nor part of equality; a
    /// restored `Audio` recomputes it on its first update because `mix_dirty` starts set.
    mixed: AudioSample,
    /// The packed channel levels [`Audio::mixed`] was computed from. Derived, like `mixed`.
    levels: u32,
    /// Something the levels cannot show has moved — panning, master volume, the power switch — so
    /// [`Audio::mixed`] is stale. Set by every register write, which is cheap and cannot be wrong.
    mix_dirty: bool,
    /// Whether the mixer and the resampler run at all. See [`Audio::set_output_enabled`].
    ///
    /// Derived state, like `output` itself: a property of the sink rather than of the machine, so
    /// it is neither serialised nor part of equality. `true` by default, which is what every caller
    /// that has not thought about it wants.
    output_enabled: bool,
    /// Video M-cycles the four channels have **not** been advanced by yet, and equally the cycles
    /// the resampler's clock is behind the machine. See the batching note in [`Audio::update`];
    /// zero again after [`Audio::sync`].
    ///
    /// Derived, like `output` and `mixed`: a debt against the channels rather than state of its
    /// own, so it is neither serialised nor part of equality. [`Audio::settled`] is how the cold
    /// paths that read the whole APU see past it.
    pending: u64,
    /// Video M-cycles from the last flush to the soonest moment any channel's output could move
    /// on its own — the bound `pending` may not reach. `0` means "unknown, flush on the next
    /// update", which is the initial value and what every path that disturbs a channel leaves
    /// behind.
    channel_deadline: u64,
    /// Whether a batch is taken at all. Always `true` outside a test build, where this field does
    /// not exist at all — see [`Audio::deadline_after_flush`] for why it exists in one.
    ///
    /// ⚠️ **And not under `bench` either, which is not tidiness.** §4.1 of
    /// `docs/emulator-performance.md` records a guarded branch on a never-taken path in this
    /// function costing 1.5%, purely in the layout of `MMU::update`, which the whole of
    /// [`Audio::update`] inlines into. This one measured **2.5%** the same way — 105.1-106.1x
    /// against 108.2-109.3x, gated, interleaved. `bench_core_throughput` is a `#[test]`, so
    /// without this second condition the one instrument this file's numbers are taken with would
    /// be reporting a machine 2.5% slower than the one that ships. The cost is to the two tests
    /// that need a control machine, which do not exist under `bench`; they are in the default
    /// tier, which is what gates everything.
    #[cfg(all(test, not(feature = "slow-tests")))]
    batching: bool,
}

impl Default for Audio {
    fn default() -> Self {
        Self {
            enabled: false,
            panning: Panning::default(),
            master_volume: MasterVolume::default(),
            frame_sequencer: FrameSequencer::default(),
            channel1: SquareWaveChannel::channel1(),
            channel2: SquareWaveChannel::channel2(),
            channel3: WaveChannel::default(),
            channel4: NoiseChannel::default(),
            output: BlipStereo::default(),
            access_machine_cycles: 0,
            mixed: AudioSample::ZERO,
            levels: 0,
            // Nothing has been mixed yet, so the first update must not trust `mixed`.
            mix_dirty: true,
            output_enabled: true,
            pending: 0,
            // Nothing has been measured yet, so the first update must flush rather than batch.
            channel_deadline: 0,
            #[cfg(all(test, not(feature = "slow-tests")))]
            batching: true,
        }
    }
}

impl Audio {
    /// Retune the resampler to the sink's rate.
    ///
    /// Not part of the serialised state (see the `Encode`/`Decode` impls below), so a caller that
    /// loads a save state has to re-apply this afterwards.
    pub fn set_output_sample_rate(&mut self, sample_rate: u32) {
        self.output.set_sample_rate(sample_rate);
    }

    /// The rate the resampler is currently producing at.
    ///
    /// Exists so that "every `load_state` has to re-apply this" can be *asserted* rather than
    /// inferred from how much audio came out — see `host::tests`. It reads derived state, so it
    /// takes no part in equality or serialisation.
    pub fn output_sample_rate(&self) -> u32 {
        self.output.sample_rate()
    }

    /// Tell the resampler how fast the emulator is running relative to real time (1.0 = realtime).
    ///
    /// Without this, fast-forwarding produces audio faster than the sink drains it: the queue grows
    /// without bound, latency climbs, and the buffer eventually starts dropping the backlog. With
    /// it, the sped-up audio plays back sped-up — higher pitched, like fast-forwarding a tape.
    ///
    /// Like the output sample rate, this is not part of the serialised state.
    pub fn set_emulation_speed(&mut self, speed: f64) {
        self.output.set_speed(speed);
    }

    /// What [`Self::set_emulation_speed`] was last told. Same reason as
    /// [`Self::output_sample_rate`]: a missed re-apply should fail a test, not a listener's ear.
    pub fn emulation_speed(&self) -> f64 {
        self.output.speed()
    }

    /// Turn the mixer and the resampler on or off, without touching the four channels.
    ///
    /// **The sink says whether anything is listening, and the APU's output side is nearly a tenth
    /// of the emulator.** `host.rs` encodes nothing while nobody has pressed the speaker — see
    /// `drain_audio` — and for that whole time the APU was still mixing four channels, quantising
    /// them and scatter-adding band-limited steps into a buffer that would be thrown away. Off, the
    /// machine is unchanged in every way software can observe: the channels keep clocking, NR52
    /// keeps answering, the wave RAM aperture still opens, and `next_event` still bounds the HALT
    /// skip. Only [`Audio::read_samples_f32`] notices, by having nothing to hand back.
    ///
    /// ⚠️ **Coming back on is a resync, not a resume.** The resampler's 16.16 time cursor stops
    /// advancing while this is off, so anything still in the buffer belongs to a moment that may be
    /// hours old and the synth's last amplitude is a level the machine has long since left. Both
    /// are dropped here, which is the same 0→1 policy `drain_audio` already applies to the encoder.
    /// `mix_dirty` is what makes the next update re-report the level from scratch.
    ///
    /// Derived state: not serialised, not part of equality, and — like the sample rate and the
    /// emulation speed — **not restored by `load_state`**, which is why the caller re-applies it
    /// every tick rather than once.
    pub fn set_output_enabled(&mut self, enabled: bool) {
        if enabled == self.output_enabled {
            return;
        }
        // ⚠️ The gate cannot move with a batch outstanding. Those cycles were incurred under the
        // old regime and [`Audio::update`]'s `lead` would hand them to whichever resampler clock
        // is running when they are finally paid: on the way in, cycles from before anyone was
        // listening, ending up in front of the first sample a listener hears; on the way out,
        // nothing, since the clock is about to stop. Paying them off here leaves the next update
        // with `lead` at zero either way.
        self.sync();
        self.output_enabled = enabled;
        if enabled {
            self.output.clear();
            self.mix_dirty = true;
        }
    }

    /// Whether the mixer and the resampler are running. Same reason as
    /// [`Self::output_sample_rate`]: a gate stuck shut should fail a test rather than a listener.
    pub fn output_enabled(&self) -> bool {
        self.output_enabled
    }

    /// How far behind the rest of the machine the four channels are being allowed to run, in
    /// video M-cycles. See the batching note in [`Audio::update`].
    ///
    /// Exists for the same reason [`Self::output_enabled`] does: batching that silently stopped
    /// happening would cost 10% gated and 14% with a listener attached, and nothing would say so.
    /// A test can assert it engaged.
    #[cfg(test)]
    pub fn pending_channel_cycles(&self) -> u64 {
        self.pending
    }

    /// Turn the batch off, so a test has a per-instruction machine to compare a batching one
    /// against. **The only callers are tests**, and the field it sets does not exist in a release
    /// build or under `bench`; see [`Audio::deadline_after_flush`].
    #[cfg(all(test, not(feature = "slow-tests")))]
    pub fn set_channel_batching(&mut self, batching: bool) {
        self.sync();
        self.batching = batching;
    }

    /// Log every amplitude transition handed to the synth, run-length merged by
    /// [`Self::take_output_transitions`]. The instrument the C6 test compares two machines with:
    /// it is upstream of the samples and says *when* a level moved, which is the whole property
    /// batching has to preserve.
    #[cfg(test)]
    pub fn capture_output_transitions(&mut self) {
        self.output.start_capture();
    }

    #[cfg(test)]
    pub fn take_output_transitions(&mut self) -> Vec<(u16, i16, i16)> {
        self.output.take_capture()
    }

    /// Fill `out` with interleaved L/R frames, returning the number of *frames* written; zero means
    /// nothing was ready.
    ///
    /// Knows nothing about the sink: an audio queue, a WAV file and a network stream all look the
    /// same from here. [`BlipStereo::read_interleaved_i16`] is the 16-bit equivalent, if a sink ever
    /// wants one.
    pub fn read_samples_f32(&mut self, out: &mut [f32]) -> usize {
        self.output.read_interleaved_f32(out)
    }

    fn reset(&mut self) {
        // Only ever reached from a NR52 write, which has already flushed — but the channels are
        // being replaced wholesale here, so a debt against the old ones would be nonsense.
        self.pending = 0;
        self.channel_deadline = 0;
        self.frame_sequencer.reset();
        self.panning = Panning::default();
        self.master_volume = MasterVolume::default();
        self.channel1 = SquareWaveChannel::channel1();
        self.channel2 = SquareWaveChannel::channel2();
        self.channel3.reset(); // not all of the wave channel is reset
        self.channel4 = NoiseChannel::default();
        // Deliberately *not* clearing the output buffer, which is what the old ring buffer did here.
        // A power-off already drives the mix to zero through `push_sample`, so the synth ramps down
        // on its own; throwing away audio the sink has not read yet would just add a click.
    }

    /// Advance the APU by `delta` and hand the resampler whatever the mixer is putting out.
    ///
    /// **C4: the mix is recomputed only when something feeding it moves.** This runs once per CPU
    /// instruction, and the four `output_f32()`s, four pans, multiply and divide below were
    /// measured at 10.5% of the whole emulator — spent, overwhelmingly, arriving at a number
    /// identical to last time. Each channel now reports whether its digital level changed, and
    /// [`Audio::mix_dirty`] covers everything the channels cannot see: panning, master volume and
    /// the power switch, all of which only move on a register write.
    ///
    /// The output is bit-identical either way: `mixed` is exactly the value the old code would
    /// have recomputed, and the resampler's 16.16 time cursor still ends up on the same clock —
    /// per instruction when C4 landed, and since C6 in two pieces per flush, which is the same
    /// arithmetic.
    pub fn update(&mut self, delta: MachineCycles, div_clocks: DividerClocks) {
        if !self.enabled {
            // Nothing is ever outstanding here, and it is enforced where the state is *created*
            // rather than checked here: the only way to reach a powered-off APU is a NR52 write,
            // which flushes on the way through [`Audio::write`] and is then followed by
            // [`Audio::reset`] clearing the debt outright; a fresh or restored `Audio` starts
            // clear. ⚠️ **Do not "just be safe" and flush here.** Even a guarded `if pending > 0 {
            // sync() }` measured **1.5%** on the pokemon fixture — a branch this one never takes,
            // paid for in the layout of `MMU::update`, which this whole function inlines into.
            debug_assert_eq!(self.pending, 0, "a powered-off APU is carrying a batch");
            self.mixed = AudioSample::ZERO;
            if self.output_enabled {
                self.push_sample(delta, AudioSample::ZERO);
            }
            return;
        }

        let events = self.frame_sequencer.update(div_clocks);

        // ⭐ **C5: the four channels are advanced to a deadline rather than once per instruction.**
        // They were **20% of the emulator** with the output side already gated, and almost all of
        // it was four counters being decremented past a moment that had not arrived: at the
        // periods a game actually plays, the soonest of the four phase timers is tens of M-cycles
        // out and an instruction is two or three. [`Audio::next_event`] — the bound the HALT skip
        // has always respected — is exactly the "how long can this be left alone" answer, so this
        // is the same skip applied to the CPU's *running* cycles as well as its idle ones.
        //
        // ⚠️ **Nothing may advance a phase inside a batch, and that is what makes it free rather
        // than approximate.** `channel_deadline` is the *minimum* over the four channels, so the
        // flush finds every one of them still short of its next tick and the closed forms in
        // `PhaseTimer::update` and `NoiseChannel::update` take their one-step path exactly as they
        // did per instruction. Everything else that can move a channel — a length counter, an
        // envelope, the sweep — hangs off the frame sequencer, and an event breaks the batch on
        // the line below; the rest needs a register write, and [`Audio::write`] flushes first.
        //
        // ⭐ **C6: a listener gets the batch too, and pays for it with one extra `end_frame`.**
        // This used to flush every instruction whenever anything was listening, because the
        // resampler has to be told *when* a level moved and not merely that it did — which cost
        // the desktop UI and anyone who pressed the speaker **14%** on the pokemon fixture and
        // 22% on `cpu_instrs`, measured. It does not have to: the deadline is the moment the
        // soonest level *could* move, so the cycles before it are silence by construction, and
        // `end_frame(a); end_frame(b)` is `end_frame(a + b)`.
        // `lead` below is those cycles, handed to the resampler in one call, after which the
        // transition lands on exactly the instruction boundary it landed on per instruction.
        let lead = self.pending;
        self.pending += delta.m_cycles();
        if events.is_empty() && self.pending < self.channel_deadline {
            return;
        }
        if !events.is_empty() {
            // ⚠️ **The batch is paid off before the event, not with it.** A channel applies its
            // length counter, envelope and sweep at the *start* of the window it is handed and
            // only then advances, so carrying the batch into this call would move cycles that
            // belong before the tick to after it. That is not a rounding error: a length counter
            // that deactivates the channel returns early, and the whole batch is dropped on the
            // floor — which is what made an idle noise channel freeze 60 cycles late.
            if lead > 0 {
                self.advance_channels(MachineCycles::from_m(lead), FrameSequencerEvent::empty());
            }
            self.pending = delta.m_cycles();
        }
        let batch = MachineCycles::from_m(std::mem::take(&mut self.pending));
        self.advance_channels(batch, events);
        // `None` — nothing is clocking — batches until the frame sequencer next has something to
        // say, which is the correct answer and a few thousand cycles rather than for ever.
        self.channel_deadline = self.deadline_after_flush();

        // ⭐ **Everything past here produces samples for somebody, and when there is nobody it is
        // skipped.** Worth **+10.2%** on `bench_core_throughput` — mixing, `BlipStereo` and
        // `end_frame` — against a headless run that is the deployment's normal state, since a
        // viewer has to press the speaker before a single packet is encoded. The
        // four channels above are *not* skipped and must not be: their registers are CPU-visible
        // through NR52 and the wave RAM, and [`Audio::next_event`] is the HALT skip's bound.
        //
        // ⚠️ Leaving `mixed`, `levels` and the resampler's clock frozen is the whole trick, and
        // [`Audio::set_output_enabled`] is where the cost of it is paid: coming back is a resync.
        if !self.output_enabled {
            return;
        }

        // ⚠️ **The batched cycles, paid to the resampler's clock and to nothing else.** No level
        // can have moved in them — that is what `channel_deadline` means — so there is no
        // transition to report and one `end_frame` puts the clock where per-instruction driving
        // would have left it. It has to come *before* the mixing below, because whatever that
        // finds moved, moved at the boundary these cycles end on. Bounded by the deadline, which
        // is bounded in turn by the frame sequencer's 2048 M-cycles, so the cast is safe.
        if lead > 0 {
            self.output.end_frame(lead as u32);
        }

        // When all four channel DACs are off, the master volume units are disconnected from the
        // sound output and the output level becomes 0.
        //
        // ⚠️ Kept **ahead of `digital_levels`**, not folded into it. It asks the same question far
        // more cheaply, and it is the state a test ROM sits in for its whole run — blargg's power
        // the APU on and never play a note. Computing the packed levels there instead cost 10% of
        // `cpu_instrs`, buying a mix-skip that this branch already gives for free.
        if !self.channel1.dac_enabled() && !self.channel2.dac_enabled()
            && !self.channel3.dac_enabled() && !self.channel4.dac_enabled() {
            self.mixed = AudioSample::ZERO;
            self.push_sample(delta, AudioSample::ZERO);
            return;
        }

        let levels = self.digital_levels();
        if levels != self.levels || self.mix_dirty {
            self.levels = levels;
            self.mix_dirty = false;
            self.mixed = self.mix();
            // ⭐ Only *now* is there a transition to report. `BlipStereo::update` quantises with a
            // libm `roundf` per channel before `BlipSynth` discovers the amplitude has not moved —
            // `perf` put 5.5% of the whole emulator in `roundf` alone, essentially all of it
            // arriving at last instruction's answer. The clock still lands on the same cycle, so
            // the output is bit-identical: the `end_frame` above and the one below sum to what
            // the skipped instructions and this one would have advanced it by one at a time.
            self.output.update(self.mixed);
        }
        // ⚠️ The two early returns above hand the resampler a literal `ZERO` instead, and must keep
        // doing so: leaving either state needs a register write, and `Audio::write` sets
        // `mix_dirty`, so this branch is guaranteed to re-report `mixed` on the way back.
        self.output.end_frame(delta.m_cycles() as u32);
    }

    /// How many **video** M-cycles the APU can be left alone for, or `None` if nothing is
    /// clocking. This is the bound C2's HALT skip must respect on the audio side.
    ///
    /// Only the four phase timers are in here, and that is the whole story: everything else that
    /// moves a channel's level — length counters, volume envelopes, the sweep — hangs off the
    /// frame sequencer, which hangs off DIV, and DIV is [`Ev::Divider`](crate::schedule::Ev). The
    /// rest only moves on a register write, which a halted CPU cannot make.
    ///
    /// **One M-cycle short of the clock, deliberately.** [`Audio::push_sample`] reports a level as
    /// changing at the *start* of the window it is given, so a skip that swallowed the clock would
    /// backdate the transition by the whole span — audible jitter, since HALT is 65% of Pokémon's
    /// cycles. Stopping a cycle early leaves the transition to a one-cycle step, which is exactly
    /// what the per-instruction driver used to produce.
    pub fn next_event(&self) -> Option<u64> {
        if !self.enabled {
            return None;
        }
        // ⚠️ Net of the outstanding batch. The channels are up to `pending` M-cycles behind the
        // rest of the machine (see [`Audio::update`]), so their own answer is that much too late.
        // It cannot actually go negative — a batch is bounded by this very minimum, and every
        // path that moves the minimum clears the batch first — so the saturation is a guard on
        // that argument rather than a case.
        let soonest = self.soonest_channel_event()?.saturating_sub(self.pending);
        Some(soonest.saturating_sub(1).max(1))
    }

    /// Video M-cycles from where the *channels* have got to until the soonest of them could move
    /// its own output, or `None` if none of them is clocking at all.
    ///
    /// Measured from the channels rather than from the machine, so a caller that cares about the
    /// machine has to net off `pending` — [`Audio::next_event`] is the one that does.
    ///
    /// ⚠️ **`None` has to survive out to [`MMU::schedule`](crate::mmu::MMU::schedule)**, which adds
    /// what it is given to the absolute clock. A stand-in `u64::MAX` wraps that sum round to a
    /// deadline in the past and the HALT skip stops skipping — silently, and only for a game whose
    /// APU is on with every channel idle, which is not a case any ROM here covers.
    #[inline]
    fn soonest_channel_event(&self) -> Option<u64> {
        // ⚠️ `perf` blames `flatten.rs` for 1.3% of the whole emulator here, and folding the four
        // `Option`s by hand to get rid of it measured **no change at all** — the compiler was
        // already doing it, and the samples are attribution rather than work. Leave it idiomatic.
        [
            self.channel1.next_event(),
            self.channel2.next_event(),
            self.channel3.next_event(),
            self.channel4.next_event(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Pay off the outstanding batch, so the channels are where the rest of the machine is.
    ///
    /// Cheap and almost always a no-op: with the output side running there is never a batch, and
    /// with it gated this is only reached from a register write and the handful of cold paths that
    /// read the whole APU. The events are empty because a batch never spans one — see
    /// [`Audio::update`].
    ///
    /// `channel_deadline` is left at zero rather than recomputed: whatever is about to happen is
    /// the reason this was called, and one extra flush on the next update is cheaper than being
    /// wrong about it.
    ///
    /// ⚠️ **The deadline is cleared even when there was nothing to pay off**, which is not
    /// belt-and-braces. A write that lands on an instruction whose predecessor happened to flush
    /// finds `pending` at zero and still moves the channel it writes to — a new frequency, a
    /// trigger — so the deadline measured before it is a promise about a machine that no longer
    /// exists. With the output side gated that was merely wasteful; the closed forms in
    /// `PhaseTimer::update` are exact over any window, so the channels came out right whenever the
    /// flush happened. With a listener it is [`Audio::update`]'s `lead` that assumes it, and a
    /// transition reported tens of cycles late is audible and nothing else here would catch it.
    fn sync(&mut self) {
        self.channel_deadline = 0;
        if self.pending == 0 {
            return;
        }
        let delta = MachineCycles::from_m(std::mem::take(&mut self.pending));
        self.advance_channels(delta, FrameSequencerEvent::empty());
        // ⚠️ **And the resampler's clock with them.** `pending` is a debt against the output side
        // as much as against the channels (see [`Audio::update`]'s `lead`), and this is the one
        // flush that is not followed by an `end_frame` for the same window — so without this the
        // cycles between the last flush and the write are simply deleted from the timeline, and
        // every transition after them lands early by that much, for the rest of the run. There is
        // nothing to *report* in them, only time to account for: the deadline says no level moved.
        //
        // [`Audio::set_output_enabled`] is the other caller and it reads correctly both ways: the
        // gate still holds its old value here, so cycles played to a listener are paid to the
        // clock on the way out, and on the way in there is no clock to pay — it is about to be
        // cleared.
        if self.output_enabled {
            self.output.end_frame(delta.m_cycles() as u32);
        }
    }

    /// How long the channels may be left alone from a flush that has just happened.
    ///
    /// One line, in a function of its own, because the tests need a machine that never batches to
    /// compare a batching one against — and after C6 there is no longer any configuration of the
    /// real thing that provides one. See `Audio::set_channel_batching`.
    #[inline]
    fn deadline_after_flush(&self) -> u64 {
        #[cfg(all(test, not(feature = "slow-tests")))]
        {
            if !self.batching {
                return 0;
            }
        }
        self.soonest_channel_event().unwrap_or(u64::MAX)
    }

    /// Hand all four channels one window. The only place any of them is advanced.
    #[inline]
    fn advance_channels(&mut self, delta: MachineCycles, events: FrameSequencerEvent) {
        self.channel1.update(delta, events);
        self.channel2.update(delta, events);
        self.channel3.update(delta, events);
        self.channel4.update(delta, events);
    }

    /// The four channels as they would be with the batch paid off, without paying it off.
    ///
    /// The cold half of [`Audio::sync`], for the two callers that read the whole APU through a
    /// `&self` they cannot flush: the `apu` save-state section and [`PartialEq`]. A save state
    /// carrying channels a few dozen cycles behind the CPU would restore a machine that is subtly
    /// not the one that was saved, and two machines batching differently — which is exactly what
    /// `the_halt_fast_path_matches_stepping_cycle_by_cycle` builds — are equal or the fast path is
    /// broken, so neither may see the debt.
    fn settled(&self) -> (SquareWaveChannel, SquareWaveChannel, WaveChannel, NoiseChannel) {
        let (mut c1, mut c2, mut c3, mut c4) =
            (self.channel1.clone(), self.channel2.clone(), self.channel3.clone(), self.channel4.clone());
        if self.pending > 0 {
            let delta = MachineCycles::from_m(self.pending);
            let events = FrameSequencerEvent::empty();
            c1.update(delta, events);
            c2.update(delta, events);
            c3.update(delta, events);
            c4.update(delta, events);
        }
        (c1, c2, c3, c4)
    }

    /// All four channels' DAC inputs packed into one word, so "has anything moved?" is a single
    /// comparison. `0xFF` marks a disconnected DAC — no channel can produce it, levels being 4-bit.
    ///
    /// Asking the channels once here beat having each `update` report its own change: that needed
    /// the level computed twice per channel and split every `update` in two, and measured *slower*
    /// than the mixing it saved.
    #[inline]
    fn digital_levels(&self) -> u32 {
        fn packed(level: Option<u8>) -> u32 {
            level.unwrap_or(0xFF) as u32
        }
        packed(self.channel1.digital_level())
            | packed(self.channel2.digital_level()) << 8
            | packed(self.channel3.digital_level()) << 16
            | packed(self.channel4.digital_level()) << 24
    }

    /// Unreachable with every DAC off — [`Audio::update`] returns before it.
    fn mix(&self) -> AudioSample {
        let channel1 = self.panning.channel1.pan(self.channel1.output_f32());
        let channel2 = self.panning.channel2.pan(self.channel2.output_f32());
        let channel3 = self.panning.channel3.pan(self.channel3.output_f32());
        let channel4 = self.panning.channel4.pan(self.channel4.output_f32());

        let volume = self.master_volume.volume_sample();
        volume * (channel1 + channel2 + channel3 + channel4) / 4.0
    }

    /// Hand the mixed output level to the resampler and advance its clock by `delta`.
    ///
    /// The level is reported as changing at the *start* of the window, which is what the old
    /// zero-order-hold loop here effectively did when it pushed `delta` copies of one value.
    ///
    /// There is no frame-time bookkeeping because there does not need to be any: the buffer's time
    /// cursor is 16.16 fixed point and carries its fractional part across calls, so ending a frame
    /// every instruction still lands every transition on the correct sub-sample phase. It also
    /// keeps latency at the kernel tail (8 output samples) rather than a chunk size.
    fn push_sample(&mut self, delta: MachineCycles, sample: AudioSample) {
        self.output.update(sample);
        self.output.end_frame(delta.m_cycles() as u32);
    }

    pub fn nr52_master_control(&self) -> u8 {
        // bits 4-6 are always 1
        let mut byte = 0x70;
        if self.enabled {
            byte |= 0x80; // Bit 7: Master enable
        }
        if self.channel1.is_active() {
            byte |= 0x01; // Bit 0: Channel 1 enable
        }
        if self.channel2.is_active() {
            byte |= 0x02; // Bit 1: Channel 2 enable
        }
        if self.channel3.is_active() {
            byte |= 0x04; // Bit 2: Channel 3 enable
        }
        if self.channel4.is_active() {
            byte |= 0x08; // Bit 3: Channel 4 enable
        }
        byte
    }

    pub fn set_nr52_master_control(&mut self, value: u8) {
        let enable = (value & 0x80) != 0; // Bit 7: Master enable
        // the rest of this register is not writable
        if self.enabled && !enable {
            // apu registers are cleared on the transition from 1 to 0 of bit 7
            self.reset();
        } else if !self.enabled && enable {
            // Reset frame sequencer when APU is re-enabled
            self.frame_sequencer.reset_to_max();
        }
        self.enabled = enable;
    }

    pub fn nr51_panning(&self) -> u8 {
        self.panning.get_byte()
    }

    pub fn set_nr51_panning_mut(&mut self, value: u8) {
        // not writable if APU is disabled
        if self.enabled {
            self.panning.set_byte(value);
        }
    }

    pub fn nr50_master_volume(&self) -> u8 {
        self.master_volume.get_byte()
    }

    pub fn set_nr50_master_volume(&mut self, value: u8) {
        // not writable if APU is disabled
        if self.enabled {
            self.master_volume.set_byte(value)
        }
    }

    pub fn read(&self, address: u16) -> u8 {
        let value = match address {
            0xFF10 => self.channel1.nr10(), // NR10: Channel 1 sweep register
            0xFF11 => self.channel1.nrx1_length_timer_duty_cycle(), // NR11: Channel 1 length and duty register
            0xFF12 => self.channel1.volume_envelope_register().get(), // NR12: Channel 1 volume and envelope register
            0xFF13 => self.channel1.nrx3_period_low(), // NR13: Channel 1 period low byte
            0xFF14 => self.channel1.nrx4_period_high_and_control(), // NR14: Channel 1 period high byte and control
            0xFF16 => self.channel2.nrx1_length_timer_duty_cycle(), // NR21: Channel 2 length and duty register
            0xFF17 => self.channel2.volume_envelope_register().get(), // NR22: Channel 2 volume and envelope register
            0xFF18 => self.channel2.nrx3_period_low(), // NR23: Channel 2 period low byte
            0xFF19 => self.channel2.nrx4_period_high_and_control(), // NR24: Channel 2 period high byte and control
            0xFF1A => self.channel3.nr30(), // NR30: Channel 3 DAC power
            0xFF1B => self.channel3.nr31_length_timer(), // NR31: Channel 3 length timer
            0xFF1C => self.channel3.nr32_output_level(), // NR32: Channel 3 output level
            0xFF1D => self.channel3.nr33_period_low(), // NR33: Channel 3 frequency low
            0xFF1E => self.channel3.nr34_period_high_and_control(), // NR34: Channel 3 frequency high and control
            0xFF20 => self.channel4.nr41_length_timer(), // NR41: Channel 4 length register
            0xFF21 => self.channel4.nr42_volume_and_envelope(), // NR42: Channel 4 volume and envelope register
            0xFF22 => self.channel4.nr43_frequency_and_randomness(), // NR43: Channel 4 frequency and randomness
            0xFF23 => self.channel4.nr44_control(), // NR44: Channel 4 control
            0xFF24 => self.nr50_master_volume(), // NR50: Sound volume register
            0xFF25 => self.nr51_panning(), // NR51: Sound panning register
            0xFF26 => self.nr52_master_control(), // NR52: Sound control register
            0xFF30..=0xFF3F => self.channel3().wave_ram((address - 0xFF30) as usize, self.access_offset()), // Wave RAM (0xFF30-0xFF3F)
            _ => {
                // ignore other audio registers for now
                0xFF
            }
        };

        // println!("Read from audio register: {:04X} = {:02X}", address, value);
        value
    }

    pub fn write(&mut self, address: u16, value: u8) {
        // println!("Write to audio register: {:04X} = {:02X}", address, value);
        // ⚠️ **Before anything else.** A write is the one thing that can move a channel without
        // the frame sequencer, so it is where a batch has to be paid off — a trigger, a frequency
        // change or a DAC switch applied on top of channels that are tens of cycles behind would
        // land at the wrong moment and, worse, invalidate the deadline that let them fall behind.
        // It also puts `access_offset` back on the instruction boundary the wave channel's
        // retrigger quirk measures from. `MMU::update` runs *after* the instruction's bus access,
        // so everything outstanding here belongs to strictly earlier instructions.
        self.sync();
        // Any APU register write can move the mixer's output, and several do so without the
        // channels seeing it at all (NR50/NR51/NR52). Marking it here rather than per register is
        // both cheaper and impossible to get wrong — see `Audio::update`.
        self.mix_dirty = true;
        let write_allowed = self.enabled || matches!(address, 0xFF11 | 0xFF16 | 0xFF1B | 0xFF20 | 0xFF26 | 0xFF30..=0xFF3F);
        if write_allowed {
            match address {
                0xFF10 => self.channel1.set_nr10(value), // NR10: Channel 1 sweep register
                0xFF11 => self.channel1.set_nrx1_length_timer_duty_cycle(value, self.enabled), // NR11: Channel 1 length and duty register
                0xFF12 => self.channel1.volume_envelope_register_mut().set(value), // NR12: Channel 1 volume and envelope register
                0xFF13 => self.channel1.set_nrx3_period_low(value), // NR13: Channel 1 period low byte
                0xFF14 => self.channel1.set_nrx4_period_high_and_control(value, &self.frame_sequencer), // NR14: Channel 1 period high byte and control
                0xFF16 => self.channel2.set_nrx1_length_timer_duty_cycle(value, self.enabled), // NR21: Channel 2 length and duty register
                0xFF17 => self.channel2.volume_envelope_register_mut().set(value), // NR22: Channel 2 volume and envelope register
                0xFF18 => self.channel2.set_nrx3_period_low(value), // NR23: Channel 2 period low byte
                0xFF19 => self.channel2.set_nrx4_period_high_and_control(value, &self.frame_sequencer), // NR24: Channel 2 period high byte and control
                0xFF1A => self.channel3.set_nr30(value), // NR30: Channel 3 DAC power
                0xFF1B => self.channel3.set_nr31_length_timer(value), // NR31: Channel 3 length timer
                0xFF1C => self.channel3.set_nr32_output_level(value), // NR32: Channel 3 output level
                0xFF1D => self.channel3.set_nr33_period_low(value), // NR33: Channel 3 frequency low
                0xFF1E => self.channel3.set_nr34_period_high_and_control(value, &self.frame_sequencer, self.access_offset()), // NR34: Channel 3 frequency high and control
                0xFF20 => self.channel4.set_nr41_length_timer(value), // NR41: Channel 4 length register
                0xFF21 => self.channel4.set_nr42_volume_and_envelope_mut(value), // NR42: Channel 4 volume and envelope register
                0xFF22 => self.channel4.set_nr43_frequency_and_randomness(value), // NR43: Channel 4 frequency and randomness
                0xFF23 => self.channel4.set_nr44_control(value, &self.frame_sequencer), // NR44: Channel 4 control
                0xFF24 => self.set_nr50_master_volume(value), // NR50: Sound volume register
                0xFF25 => self.set_nr51_panning_mut(value), // NR51: Sound panning register
                0xFF26 => self.set_nr52_master_control(value), // NR52: Sound control register
                0xFF30..=0xFF3F => { let offset = self.access_offset(); self.channel3_mut().set_wave_ram((address - 0xFF30) as usize, value, offset) } // Wave RAM (0xFF30-0xFF3F)
                _ => {
                    // ignore other audio registers for now
                }
            }
        }
    }

    /// Tell the APU how long, in M-cycles, the instruction now executing is.
    ///
    /// Peripherals are still advanced once per instruction — this changes nothing about *when*
    /// they run. It only lets the APU work out *where* the CPU's bus access sits inside the
    /// instruction it is about to be advanced over, which is the one thing DMG's wave-RAM
    /// aperture depends on: that window is a single tick wide, so "somewhere in this instruction"
    /// is not good enough. Only [`WaveChannel`] reads it.
    pub fn set_instruction_length(&mut self, machine_cycles: u8) {
        self.access_machine_cycles = machine_cycles;
    }

    /// Where in the current instruction the CPU's bus access falls, in wave-timer ticks (2
    /// T-cycles each). Hardware puts a load's or store's memory access in the instruction's final
    /// M-cycle, so it is one M-cycle short of the whole instruction.
    fn access_offset(&self) -> u16 {
        // ⚠️ **Plus the outstanding batch**, because the offset is measured from where the wave
        // timer has actually got to and [`Audio::update`] may have left it up to `pending`
        // M-cycles short of the instruction boundary. `fetch_at` subtracts the timer's counter,
        // which is stale by the same amount, so the two cancel exactly.
        //
        // On the *write* path this term is always zero — [`Audio::write`] flushes first — which is
        // what keeps `WaveChannel::trigger`'s `trigger_after(3 + access_offset)` measuring from
        // the instruction boundary it means. Reads are the case this exists for: [`Audio::read`]
        // takes `&self` and cannot flush.
        let batch = u16::try_from(self.pending).unwrap_or(u16::MAX).saturating_mul(2);
        ((self.access_machine_cycles.saturating_sub(1) as u16) * 2).saturating_add(batch)
    }

    pub fn channel3(&self) -> &WaveChannel {
        &self.channel3
    }
    
    pub fn channel3_mut(&mut self) -> &mut WaveChannel {
        &mut self.channel3
    }

}

/// Contents of the `apu` save-state section. Excludes `output` — the resampler is a sink, not
/// machine state.
#[derive(Debug, Clone, Decode, Encode)]
pub struct ApuSection {
    pub enabled: bool,
    pub panning: Panning,
    pub master_volume: MasterVolume,
    pub frame_sequencer: FrameSequencer,
    pub channel1: SquareWaveChannel,
    pub channel2: SquareWaveChannel,
    pub channel3: WaveChannel,
    pub channel4: NoiseChannel,
}

pub const APU_SECTION_VERSION: u16 = 1;

impl Audio {
    pub(crate) fn write_sections(&self, writer: &mut SectionWriter) -> Result<(), String> {
        // ⚠️ Settled, not as they stand: the channels may be running a batch behind the CPU (see
        // [`Audio::update`]) and a save state has to be the machine at one moment.
        let (channel1, channel2, channel3, channel4) = self.settled();
        writer.write(labels::APU, APU_SECTION_VERSION, &ApuSection {
            enabled: self.enabled,
            panning: self.panning,
            master_volume: self.master_volume.clone(),
            frame_sequencer: self.frame_sequencer.clone(),
            channel1,
            channel2,
            channel3,
            channel4,
        })
    }

    pub(crate) fn read_sections(&mut self, reader: &SectionReader) -> Result<(), String> {
        if let Some((_version, section)) = reader.read::<ApuSection>(labels::APU)? {
            self.enabled = section.enabled;
            self.panning = section.panning;
            self.master_volume = section.master_volume;
            self.frame_sequencer = section.frame_sequencer;
            self.channel1 = section.channel1;
            self.channel2 = section.channel2;
            self.channel3 = section.channel3;
            self.channel4 = section.channel4;
            // `mixed` is derived and not in the section, so the restored machine must recompute it
            // before trusting it — see `Audio::update`.
            self.mix_dirty = true;
            // The section was written settled, so the restored channels owe nothing; the deadline
            // that let them fall behind belonged to the machine that was saved, not this one.
            self.pending = 0;
            self.channel_deadline = 0;
        }
        Ok(())
    }
}

impl PartialEq for Audio {
    /// ⚠️ **Compares the channels settled**, for the same reason the save-state section writes
    /// them settled: two machines at the same instant may be carrying different batches (see
    /// [`Audio::update`]), and one stepping every M-cycle against one taking the HALT skip is
    /// precisely what `the_halt_fast_path_matches_stepping_cycle_by_cycle` builds. `pending` and
    /// `channel_deadline` themselves are derived and take no part.
    ///
    /// The clone is why it asks first, and why the cheap arm is "neither owes anything" rather
    /// than "both owe the same": advancing a channel is not injective — an inactive one has its
    /// output zeroed — so two equal debts are not enough to make the raw states comparable.
    fn eq(&self, other: &Self) -> bool {
        let header = self.enabled == other.enabled &&
            self.panning == other.panning &&
            self.master_volume == other.master_volume &&
            self.frame_sequencer == other.frame_sequencer;
        if !header {
            return false;
        }
        if self.pending == 0 && other.pending == 0 {
            return self.channel1 == other.channel1 &&
                self.channel2 == other.channel2 &&
                self.channel3 == other.channel3 &&
                self.channel4 == other.channel4;
        }
        self.settled() == other.settled()
    }
}

impl Eq for Audio {}

