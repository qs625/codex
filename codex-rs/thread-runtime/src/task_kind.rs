//! Session task classification shared by turn state and runtime scheduling.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskKind {
    Regular,
    Review,
    Compact,
}
