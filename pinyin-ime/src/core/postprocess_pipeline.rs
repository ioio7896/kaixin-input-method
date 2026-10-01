use super::*;

/// Immutable data shared by candidate transformation stages.  Fields can be
/// added here as ranking stages are migrated out of the legacy postprocessor.
pub(super) struct CandidatePipelineContext {
    trace: bool,
}

impl Default for CandidatePipelineContext {
    fn default() -> Self {
        Self {
            trace: runtime_log::enabled(RuntimeLogLevel::Verbose),
        }
    }
}

pub(super) trait CandidateStage {
    fn name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
    fn apply(&self, context: &CandidatePipelineContext, candidates: &mut Vec<RankedCandidate>);
}

pub(super) struct NamedStage<F>(pub &'static str, pub F);
impl<F: Fn(&mut Vec<RankedCandidate>)> CandidateStage for NamedStage<F> {
    fn name(&self) -> &'static str {
        self.0
    }
    fn apply(&self, _: &CandidatePipelineContext, candidates: &mut Vec<RankedCandidate>) {
        (self.1)(candidates);
    }
}

pub(super) fn run_candidate_pipeline(
    context: &CandidatePipelineContext,
    candidates: &mut Vec<RankedCandidate>,
    stages: &[&dyn CandidateStage],
) {
    for stage in stages {
        if crate::core::cache::current_lookup_request_superseded() {
            break;
        }
        let before = context.trace.then(|| {
            candidates
                .iter()
                .map(|row| row.phrase.clone())
                .collect::<Vec<_>>()
        });
        let started = context.trace.then(Instant::now);
        stage.apply(context, candidates);
        if let Some(before) = before {
            // Positions explain stage effects without logging input or phrases.
            let moves = candidates
                .iter()
                .take(9)
                .enumerate()
                .map(|(to, row)| {
                    let from = before.iter().position(|phrase| phrase == &row.phrase);
                    format!(
                        "{}>{}",
                        from.map_or_else(|| "new".to_string(), |n| (n + 1).to_string()),
                        to + 1
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            runtime_log::log_engine(
                RuntimeLogLevel::Verbose,
                "srf_candidate_stage",
                format!(
                    "stage={} before={} after={} positions={} elapsed={}us",
                    stage.name(),
                    before.len(),
                    candidates.len(),
                    moves,
                    started.map_or(0, |t| t.elapsed().as_micros())
                ),
            );
        }
    }
}
