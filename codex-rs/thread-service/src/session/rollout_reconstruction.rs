use super::*;
use rollout_api::RolloutReconstruction;
use rollout_api::RolloutReconstructionOptions;

impl Session {
    pub(super) async fn reconstruct_history_from_rollout(
        &self,
        turn_context: &TurnContext,
        rollout_items: &[RolloutItem],
    ) -> RolloutReconstruction {
        rollout_api::reconstruct_history_from_rollout(
            rollout_items,
            RolloutReconstructionOptions {
                truncation_policy: turn_context.truncation_policy,
                summary_prefix: Some(compact::SUMMARY_PREFIX),
            },
        )
    }
}
