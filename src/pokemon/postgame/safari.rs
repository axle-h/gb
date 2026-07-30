//! Workstream **E — Safari Zone proper**. See `docs/postgame-coverage-plan.md` §6-E.
//!
//! The Safari Zone is currently entered only to grab HM03 and the Gold Teeth, and
//! `pick_battle_action` hard-codes RUN on every Safari encounter.
//!
//! Sub-steps: E1 model the 500-step budget · E2 replace the blanket RUN with a real catch policy
//! (Rock raises catch rate *and* flee rate; Bait does the inverse) · E3 catch a Safari-exclusive ·
//! E4 exit cleanly both ways, then `postgame-safari.bin`.
