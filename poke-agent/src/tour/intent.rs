//! What a scripted run means to do next, resolved against the rendered action menu alone.

use crate::tour::turn::TurnRequest;

/// One thing the run means to do next, resolved against the rendered action menu alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// Take transitions toward `map` until the `Location:` line says the player is there.
    Enter(&'static str),
    /// Choose the row whose id ends in `:{0}` — a person's name, `Pc`, `CutTree`, `Grass`.
    Row(&'static str),
    /// Choose the first row whose *description* contains `{0}`.
    Says(&'static str),
    /// Choose the row whose description contains `{0}` until the menu stops offering it.
    Repeat(&'static str),
    /// Nothing to do: end the turn without moving.
    Wait,
}

impl Intent {
    /// Whether the situation says this intent is already satisfied, so no turn is spent.
    pub fn satisfied_by(&self, request: &TurnRequest) -> bool {
        match self {
            Self::Enter(map) => request.location().as_deref() == Some(map),
            Self::Repeat(fragment) => !request.menu_rows().iter()
                .any(|(_, description)| description.contains(fragment)),
            // A situation cannot show that a person was talked to, so these are one intent per
            // turn.
            Self::Row(_) | Self::Says(_) | Self::Wait => false,
        }
    }

    /// The id that carries this intent out, or `None` if the menu does not offer one.
    pub fn resolve(&self, request: &TurnRequest) -> Option<String> {
        let rows = request.menu_rows();
        match self {
            Self::Wait => None,
            Self::Enter(map) => rows
                .iter()
                // A connection row says "go to ViridianCity"; a warp row "take the warp to OaksLab,
                // arriving at (12, 12)".
                .find(|(_, description)| names_map(description, map))
                .map(|(id, _)| id.clone()),
            Self::Row(kind) => rows
                .iter()
                .find(|(id, _)| id.rsplit(':').next() == Some(*kind))
                .map(|(id, _)| id.clone()),
            Self::Says(fragment) | Self::Repeat(fragment) => rows
                .iter()
                .find(|(_, description)| description.contains(fragment))
                .map(|(id, _)| id.clone()),
        }
    }
}

/// Whether `description` names exactly this map, rather than one whose name starts the same way.
pub fn names_map(description: &str, map: &str) -> bool {
    description
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| word == map)
}
