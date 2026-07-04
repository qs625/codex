use protocol::protocol::CodexErrorInfo;
use protocol::protocol::ErrorEvent;
use protocol::protocol::NonSteerableTurnKind;
use protocol::user_input::UserInput;

#[derive(Debug, PartialEq)]
pub enum SteerInputError {
    NoActiveTurn(Vec<UserInput>),
    ExpectedTurnMismatch { expected: String, actual: String },
    ActiveTurnNotSteerable { turn_kind: NonSteerableTurnKind },
    EmptyInput,
}

impl SteerInputError {
    pub fn to_error_event(&self) -> ErrorEvent {
        match self {
            Self::NoActiveTurn(_) => ErrorEvent {
                message: "no active turn to steer".to_string(),
                codex_error_info: Some(CodexErrorInfo::BadRequest),
            },
            Self::ExpectedTurnMismatch { expected, actual } => ErrorEvent {
                message: format!("expected active turn id `{expected}` but found `{actual}`"),
                codex_error_info: Some(CodexErrorInfo::BadRequest),
            },
            Self::ActiveTurnNotSteerable { turn_kind } => {
                let turn_kind_label = match turn_kind {
                    NonSteerableTurnKind::Review => "review",
                    NonSteerableTurnKind::Compact => "compact",
                };
                ErrorEvent {
                    message: format!("cannot steer a {turn_kind_label} turn"),
                    codex_error_info: Some(CodexErrorInfo::ActiveTurnNotSteerable {
                        turn_kind: *turn_kind,
                    }),
                }
            }
            Self::EmptyInput => ErrorEvent {
                message: "input must not be empty".to_string(),
                codex_error_info: Some(CodexErrorInfo::BadRequest),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SteerableTaskKind {
    Regular,
    Review,
    Compact,
}

pub struct ActiveSteerTurn<'a> {
    pub turn_id: &'a str,
    pub task_kind: SteerableTaskKind,
}

#[derive(Debug)]
pub struct ValidatedSteerInput {
    pub active_turn_id: String,
    pub input: Vec<UserInput>,
}

pub fn validate_steer_input(
    input: Vec<UserInput>,
    expected_turn_id: Option<&str>,
    active_turn: Option<ActiveSteerTurn<'_>>,
) -> Result<ValidatedSteerInput, SteerInputError> {
    if input.is_empty() {
        return Err(SteerInputError::EmptyInput);
    }

    let Some(active_turn) = active_turn else {
        return Err(SteerInputError::NoActiveTurn(input));
    };

    if let Some(expected_turn_id) = expected_turn_id
        && expected_turn_id != active_turn.turn_id
    {
        return Err(SteerInputError::ExpectedTurnMismatch {
            expected: expected_turn_id.to_string(),
            actual: active_turn.turn_id.to_string(),
        });
    }

    match active_turn.task_kind {
        SteerableTaskKind::Regular => {}
        SteerableTaskKind::Review => {
            return Err(SteerInputError::ActiveTurnNotSteerable {
                turn_kind: NonSteerableTurnKind::Review,
            });
        }
        SteerableTaskKind::Compact => {
            return Err(SteerInputError::ActiveTurnNotSteerable {
                turn_kind: NonSteerableTurnKind::Compact,
            });
        }
    }

    Ok(ValidatedSteerInput {
        active_turn_id: active_turn.turn_id.to_string(),
        input,
    })
}

#[cfg(test)]
mod tests {
    use super::ActiveSteerTurn;
    use super::SteerInputError;
    use super::SteerableTaskKind;
    use super::validate_steer_input;
    use protocol::protocol::NonSteerableTurnKind;
    use protocol::user_input::UserInput;

    fn text_input() -> Vec<UserInput> {
        vec![UserInput::Text {
            text: "steer".to_string(),
            text_elements: Vec::new(),
        }]
    }

    #[test]
    fn requires_active_turn() {
        let err = validate_steer_input(
            text_input(),
            /*expected_turn_id*/ None,
            /*active_turn*/ None,
        )
        .expect_err("steering without active turn should fail");

        assert!(matches!(err, SteerInputError::NoActiveTurn(_)));
    }

    #[test]
    fn enforces_expected_turn_id() {
        let err = validate_steer_input(
            text_input(),
            Some("different-turn-id"),
            Some(ActiveSteerTurn {
                turn_id: "turn-1",
                task_kind: SteerableTaskKind::Regular,
            }),
        )
        .expect_err("mismatched expected turn id should fail");

        assert_eq!(
            err,
            SteerInputError::ExpectedTurnMismatch {
                expected: "different-turn-id".to_string(),
                actual: "turn-1".to_string(),
            }
        );
    }

    #[test]
    fn rejects_non_regular_turns() {
        for (task_kind, turn_kind) in [
            (SteerableTaskKind::Review, NonSteerableTurnKind::Review),
            (SteerableTaskKind::Compact, NonSteerableTurnKind::Compact),
        ] {
            let err = validate_steer_input(
                text_input(),
                /*expected_turn_id*/ None,
                Some(ActiveSteerTurn {
                    turn_id: "turn-1",
                    task_kind,
                }),
            )
            .expect_err("steering a non-regular turn should fail");

            assert_eq!(err, SteerInputError::ActiveTurnNotSteerable { turn_kind });
        }
    }

    #[test]
    fn accepts_regular_turn() {
        let validated = validate_steer_input(
            text_input(),
            Some("turn-1"),
            Some(ActiveSteerTurn {
                turn_id: "turn-1",
                task_kind: SteerableTaskKind::Regular,
            }),
        )
        .expect("regular turn should accept steering");

        assert_eq!(validated.active_turn_id, "turn-1");
        assert_eq!(validated.input.len(), 1);
    }
}
