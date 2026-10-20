//! Explicit finite qualification fault, owned by one existing peer process.
//! A new generation permits receipt release; the operator independently records
//! the device restart and confirms its durable completion while this host lives.
use coaptic::provisioning::{Receipt, TelemetryOperation};

enum State {
    Waiting,
    Arming(Receipt),
    Held { receipt: Receipt, generation: u64 },
    Released { receipt: Receipt },
}

pub(super) struct Qualification {
    deadline: u64,
    state: State,
}
const _: () = assert!(core::mem::size_of::<Qualification>() <= 96);
const _: () = assert!(core::mem::size_of::<Option<Qualification>>() <= 96);

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Action {
    Deliver,
    Withhold,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Event {
    Armed {
        sequence: u64,
        generation: u64,
    },
    Released {
        sequence: u64,
        previous: u64,
        current: u64,
    },
}

impl Qualification {
    pub(super) fn new(now: u64) -> Self {
        Self {
            deadline: now.saturating_add(90_000),
            state: State::Waiting,
        }
    }

    pub(super) fn expired(&self, now: u64) -> bool {
        now >= self.deadline
    }

    pub(super) fn released(&self) -> bool {
        matches!(self.state, State::Released { .. })
    }

    /// Reject any other work before calling the effect journal. Receipt custody
    /// has no payload copy: its complete digest already binds resource and bytes.
    pub(super) fn permits(&self, operation: &TelemetryOperation<'_>) -> bool {
        match self.state {
            State::Waiting => true,
            State::Arming(receipt) | State::Held { receipt, .. } | State::Released { receipt } => {
                receipt.id() == operation.id() && receipt.digest() == operation.digest()
            }
        }
    }

    /// Call only after Journal::commit succeeds, hence after its AfterSync hook.
    /// That bounded, nondeleting journal assigns count+1 only to a new row;
    /// duplicates return an existing sequence before invoking the checkpoint.
    pub(super) fn recorded(
        &mut self,
        receipt: Receipt,
        effects_before: usize,
    ) -> Result<Action, &'static str> {
        match self.state {
            State::Waiting => {
                let before = u64::try_from(effects_before).map_err(|_| "invalid effect count")?;
                if receipt.sequence() <= before {
                    return Ok(Action::Deliver);
                }
                if before.checked_add(1) != Some(receipt.sequence()) {
                    return Err("new receipt differs from the bounded journal checkpoint");
                }
                self.state = State::Arming(receipt);
                Ok(Action::Withhold)
            }
            State::Arming(original)
            | State::Held {
                receipt: original, ..
            } if original == receipt => Ok(Action::Withhold),
            State::Released { receipt: original } if original == receipt => Ok(Action::Deliver),
            _ => Err("receipt differs from the exact retained qualification row"),
        }
    }

    /// Sample only after the dispatch pass: the pass itself may install a fresh
    /// session before committing the first effect. That generation arms custody;
    /// it must never simultaneously count as the later generation releasing it.
    pub(super) fn after_poll(
        &mut self,
        generation: Option<u64>,
    ) -> Result<Option<Event>, &'static str> {
        match self.state {
            State::Arming(receipt) => {
                let generation =
                    generation.ok_or("durable checkpoint has no established protected session")?;
                self.state = State::Held {
                    receipt,
                    generation,
                };
                Ok(Some(Event::Armed {
                    sequence: receipt.sequence(),
                    generation,
                }))
            }
            State::Held {
                receipt,
                generation: previous,
            } => match generation {
                Some(current) if current > previous => {
                    self.state = State::Released { receipt };
                    Ok(Some(Event::Released {
                        sequence: receipt.sequence(),
                        previous,
                        current,
                    }))
                }
                Some(current) if current < previous => {
                    Err("protected session generation regressed")
                }
                _ => Ok(None),
            },
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coaptic::provisioning::OperationId;

    fn operation(id: u8, body: &'static [u8]) -> TelemetryOperation<'static> {
        TelemetryOperation::new(OperationId::new([id; 16]), [3; 32], Some(60), body).unwrap()
    }
    fn receipt(operation: &TelemetryOperation<'_>, sequence: u64) -> Receipt {
        Receipt::from_parts(operation.id(), *operation.digest(), sequence).unwrap()
    }

    #[test]
    fn only_first_new_successful_row_arms_and_generation_is_sampled_after_dispatch() {
        let op = operation(1, b"same bytes");
        let mut fixture = Qualification::new(100);
        assert_eq!(
            fixture.recorded(receipt(&operation(9, b"older effect"), 2), 2),
            Ok(Action::Deliver)
        );
        assert_eq!(fixture.after_poll(Some(7)), Ok(None));
        let selected = receipt(&op, 3);
        assert_eq!(fixture.recorded(selected, 2), Ok(Action::Withhold));
        assert_eq!(
            fixture.after_poll(Some(8)),
            Ok(Some(Event::Armed {
                sequence: 3,
                generation: 8
            }))
        );
        assert!(!fixture.released());
        assert_eq!(fixture.after_poll(None), Ok(None));
        assert_eq!(fixture.after_poll(Some(8)), Ok(None));
        assert_eq!(fixture.recorded(selected, 3), Ok(Action::Withhold));
        assert_eq!(
            fixture.after_poll(Some(9)),
            Ok(Some(Event::Released {
                sequence: 3,
                previous: 8,
                current: 9
            }))
        );
        assert_eq!(fixture.recorded(selected, 3), Ok(Action::Deliver));
        assert!(fixture.released());
        assert!(!fixture.expired(90_099));
        assert!(fixture.expired(90_100));
    }

    #[test]
    fn conflicts_and_invalid_progress_cannot_replace_or_release_the_selected_effect() {
        let op = operation(1, b"original");
        let mut fixture = Qualification::new(0);
        assert!(fixture.recorded(receipt(&op, 4), 2).is_err());
        assert_eq!(fixture.after_poll(Some(1)), Ok(None));
        let selected = receipt(&op, 3);
        fixture.recorded(selected, 2).unwrap();
        assert!(fixture.after_poll(None).is_err());
        fixture.after_poll(Some(2)).unwrap();
        for other in [operation(2, b"original"), operation(1, b"changed")] {
            assert!(!fixture.permits(&other));
            assert!(fixture.recorded(receipt(&other, 3), 3).is_err());
        }
        assert!(fixture.recorded(receipt(&op, 4), 3).is_err());
        assert!(fixture.after_poll(Some(1)).is_err());
        assert_eq!(fixture.recorded(selected, 3), Ok(Action::Withhold));
        fixture.after_poll(Some(3)).unwrap();
        assert!(!fixture.permits(&operation(2, b"fresh")));
        assert!(fixture.permits(&op));
        assert_eq!(fixture.recorded(selected, 3), Ok(Action::Deliver));
    }
}
