//! Lightweight behavior tree engine tailored for hourly simulation ticks.
//!
//! Trees are pure data (no closures), making them inspectable and eventually serializable.
//! Execution state is tracked separately so the same tree definition can be shared.

/// Result of ticking a behavior node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Status {
    Success,
    Failure,
    Running,
}

/// A behavior tree node. Pure data — describes *what* to do, not *how*.
/// Actions and conditions are identified by index into external lookup tables.
#[derive(Debug, Clone)]
pub enum Behavior {
    /// Leaf: execute an action. Returns Running while in progress, Success/Failure when done.
    Action(usize),
    /// Leaf: check a condition. Returns Success if true, Failure if false. Never Running.
    Condition(usize),
    /// Run children in order. Fails immediately on first Failure. Succeeds when all succeed.
    Sequence(Vec<Behavior>),
    /// Try children in order. Succeeds on first Success. Fails when all fail.
    Selector(Vec<Behavior>),
    /// Invert child result (Success↔Failure, Running stays Running).
    Invert(Box<Behavior>),
}

/// Execution state for a behavior tree. Tracks progress through composite nodes.
/// Created fresh or persisted across ticks for Running behaviors.
#[derive(Debug, Clone)]
pub struct BtState {
    /// Stack of (child index) for nested composites.
    /// Each entry is "which child are we currently on" in that composite.
    pub running_child: Vec<usize>,
}

impl Default for BtState {
    fn default() -> Self {
        Self::new()
    }
}

impl BtState {
    pub fn new() -> Self {
        Self {
            running_child: Vec::new(),
        }
    }

    /// Reset state (start tree from beginning).
    pub fn reset(&mut self) {
        self.running_child.clear();
    }
}

/// Trait that the simulation implements to execute actions and check conditions.
/// The BT engine calls these methods when it reaches leaf nodes.
pub trait BtContext {
    /// Execute action identified by `id`. Return Running if still in progress.
    fn execute_action(&mut self, id: usize) -> Status;
    /// Check condition identified by `id`. Return Success (true) or Failure (false).
    fn check_condition(&mut self, id: usize) -> Status;
}

/// Tick a behavior tree node, returning its status.
/// For composite nodes, uses `depth` to index into `state.running_child`.
pub fn tick(node: &Behavior, state: &mut BtState, ctx: &mut dyn BtContext, depth: usize) -> Status {
    match node {
        Behavior::Action(id) => ctx.execute_action(*id),
        Behavior::Condition(id) => ctx.check_condition(*id),

        Behavior::Sequence(children) => {
            // Ensure state has an entry for this depth
            while state.running_child.len() <= depth {
                state.running_child.push(0);
            }

            let start = state.running_child[depth];
            #[allow(clippy::needless_range_loop)]
            for i in start..children.len() {
                state.running_child[depth] = i;
                let status = tick(&children[i], state, ctx, depth + 1);
                match status {
                    Status::Failure => {
                        // Reset this level for next time
                        state.running_child[depth] = 0;
                        // Truncate deeper state
                        state.running_child.truncate(depth + 1);
                        return Status::Failure;
                    }
                    Status::Running => return Status::Running,
                    Status::Success => continue,
                }
            }
            // All children succeeded
            state.running_child[depth] = 0;
            state.running_child.truncate(depth + 1);
            Status::Success
        }

        Behavior::Selector(children) => {
            while state.running_child.len() <= depth {
                state.running_child.push(0);
            }

            let start = state.running_child[depth];
            #[allow(clippy::needless_range_loop)]
            for i in start..children.len() {
                state.running_child[depth] = i;
                let status = tick(&children[i], state, ctx, depth + 1);
                match status {
                    Status::Success => {
                        state.running_child[depth] = 0;
                        state.running_child.truncate(depth + 1);
                        return Status::Success;
                    }
                    Status::Running => return Status::Running,
                    Status::Failure => continue,
                }
            }
            // All children failed
            state.running_child[depth] = 0;
            state.running_child.truncate(depth + 1);
            Status::Failure
        }

        Behavior::Invert(child) => match tick(child, state, ctx, depth) {
            Status::Success => Status::Failure,
            Status::Failure => Status::Success,
            Status::Running => Status::Running,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestCtx {
        actions: Vec<Status>,     // what each action returns
        conditions: Vec<bool>,    // what each condition returns
        action_calls: Vec<usize>, // track which actions were called
    }

    impl BtContext for TestCtx {
        fn execute_action(&mut self, id: usize) -> Status {
            self.action_calls.push(id);
            self.actions[id]
        }
        fn check_condition(&mut self, id: usize) -> Status {
            if self.conditions[id] {
                Status::Success
            } else {
                Status::Failure
            }
        }
    }

    #[test]
    fn sequence_runs_all_on_success() {
        let tree = Behavior::Sequence(vec![
            Behavior::Action(0),
            Behavior::Action(1),
            Behavior::Action(2),
        ]);
        let mut state = BtState::new();
        let mut ctx = TestCtx {
            actions: vec![Status::Success, Status::Success, Status::Success],
            conditions: vec![],
            action_calls: vec![],
        };
        let result = tick(&tree, &mut state, &mut ctx, 0);
        assert_eq!(result, Status::Success);
        assert_eq!(ctx.action_calls, vec![0, 1, 2]);
    }

    #[test]
    fn sequence_stops_on_failure() {
        let tree = Behavior::Sequence(vec![
            Behavior::Action(0),
            Behavior::Action(1),
            Behavior::Action(2),
        ]);
        let mut state = BtState::new();
        let mut ctx = TestCtx {
            actions: vec![Status::Success, Status::Failure, Status::Success],
            conditions: vec![],
            action_calls: vec![],
        };
        let result = tick(&tree, &mut state, &mut ctx, 0);
        assert_eq!(result, Status::Failure);
        assert_eq!(ctx.action_calls, vec![0, 1]); // never reached action 2
    }

    #[test]
    fn sequence_resumes_after_running() {
        let tree = Behavior::Sequence(vec![Behavior::Action(0), Behavior::Action(1)]);
        let mut state = BtState::new();

        // First tick: action 0 succeeds, action 1 is running
        let mut ctx = TestCtx {
            actions: vec![Status::Success, Status::Running],
            conditions: vec![],
            action_calls: vec![],
        };
        let result = tick(&tree, &mut state, &mut ctx, 0);
        assert_eq!(result, Status::Running);

        // Second tick: resumes at action 1 (skips action 0)
        ctx.actions[1] = Status::Success;
        ctx.action_calls.clear();
        let result = tick(&tree, &mut state, &mut ctx, 0);
        assert_eq!(result, Status::Success);
        assert_eq!(ctx.action_calls, vec![1]); // only action 1 was ticked
    }

    #[test]
    fn selector_succeeds_on_first_success() {
        let tree = Behavior::Selector(vec![Behavior::Action(0), Behavior::Action(1)]);
        let mut state = BtState::new();
        let mut ctx = TestCtx {
            actions: vec![Status::Failure, Status::Success],
            conditions: vec![],
            action_calls: vec![],
        };
        let result = tick(&tree, &mut state, &mut ctx, 0);
        assert_eq!(result, Status::Success);
        assert_eq!(ctx.action_calls, vec![0, 1]);
    }

    #[test]
    fn condition_gates_sequence() {
        // Sequence: [Condition(0), Action(0)]
        // If condition fails, action never runs
        let tree = Behavior::Sequence(vec![Behavior::Condition(0), Behavior::Action(0)]);
        let mut state = BtState::new();
        let mut ctx = TestCtx {
            actions: vec![Status::Success],
            conditions: vec![false],
            action_calls: vec![],
        };
        let result = tick(&tree, &mut state, &mut ctx, 0);
        assert_eq!(result, Status::Failure);
        assert_eq!(ctx.action_calls, vec![]); // action never called
    }
}
