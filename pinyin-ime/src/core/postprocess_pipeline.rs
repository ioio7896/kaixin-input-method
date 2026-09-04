use super::*;

/// Immutable data shared by candidate transformation stages.  Fields can be
/// added here as ranking stages are migrated out of the legacy postprocessor.
#[derive(Debug)]
pub(super) struct CandidatePipelineContext;

pub(super) trait CandidateStage {
    fn apply(&self, context: &CandidatePipelineContext, candidates: &mut Vec<RankedCandidate>);
}

pub(super) fn run_candidate_pipeline(
    context: &CandidatePipelineContext,
    candidates: &mut Vec<RankedCandidate>,
    stages: &[&dyn CandidateStage],
) {
    for stage in stages {
        stage.apply(context, candidates);
    }
}
