use yaar_link::{Fact, Refusal, Reply, ReviewId, ThreadId};
use yaar_review_engine::{Attachment, EngineError, Offer, ReviewError};

use crate::agents::Agents;
use crate::wiring::Kept;

/// Answers with the CLIs to offer: none while the review's agent runs, its agent's CLI when it does
/// not, every supported CLI for a review of no agent.
pub(crate) fn attach(
    kept: &Kept,
    agents: &Agents,
    review: &ReviewId,
    thread: Option<&ThreadId>,
    facts: &mut Vec<Fact>,
) -> Result<Reply, Refusal> {
    let id = engine_id(review);
    let workspace = workspace_of(kept, review)?;
    let sessions = agents.sessions_now(kept, &yaar_link::WorkspaceId::new(workspace.as_str()));
    let attachment = match thread {
        Some(thread) => Attachment::Thread(yaar_review_engine::ThreadId::new(thread.as_str())),
        None => Attachment::Review,
    };
    let offer = kept
        .reviews
        .attach(&id, attachment, &sessions)
        .map_err(|error| refused(review, error))?;
    told(facts, agents, review, &workspace);
    let start = match offer {
        Offer::Nothing => Vec::new(),
        Offer::Start { cli } => agents
            .supported()
            .into_iter()
            .filter(|offered| offered.agent == cli)
            .collect(),
        Offer::AnyCli => agents.supported(),
    };
    Ok(Reply::AttachedReview { start })
}

pub(crate) fn detach(
    kept: &Kept,
    agents: &Agents,
    review: &ReviewId,
    facts: &mut Vec<Fact>,
) -> Result<Reply, Refusal> {
    kept.reviews
        .detach(&engine_id(review))
        .map_err(|error| refused(review, error))?;
    let workspace = workspace_of(kept, review)?;
    // Told even when nothing was attached: a Window that still shows an attachment learns
    // that there is none.
    told(facts, agents, review, &workspace);
    Ok(Reply::Done)
}

pub(crate) fn resolve(
    kept: &Kept,
    review: &ReviewId,
    thread: &ThreadId,
    resolved: bool,
    facts: &mut Vec<Fact>,
) -> Result<Reply, Refusal> {
    let id = engine_id(review);
    let thread = yaar_review_engine::ThreadId::new(thread.as_str());
    if resolved {
        kept.reviews.resolve(&id, &thread)
    } else {
        kept.reviews.reopen(&id, &thread)
    }
    .map_err(|error| refused(review, error))?;
    // A thread attached alone stays attached when it is resolved, so the agents' side has
    // nothing new to show.
    facts.push(reviews_changed(&workspace_of(kept, review)?));
    Ok(Reply::Done)
}

pub(crate) fn resolve_addressed(
    kept: &Kept,
    review: &ReviewId,
    facts: &mut Vec<Fact>,
) -> Result<Reply, Refusal> {
    let resolved = kept
        .reviews
        .resolve_addressed(&engine_id(review))
        .map_err(|error| refused(review, error))?;
    if !resolved.is_empty() {
        facts.push(reviews_changed(&workspace_of(kept, review)?));
    }
    Ok(Reply::Done)
}

pub(crate) fn attachment(
    kept: &Kept,
    agents: &Agents,
    review: &ReviewId,
) -> Result<Reply, Refusal> {
    let nothing = || Refusal::NothingToSend {
        review: review.clone(),
    };
    let id = engine_id(review);
    let held = kept
        .reviews
        .review(&id)
        .map_err(|error| refused(review, error))?;
    let instruction = held
        .agent()
        .map(yaar_review_engine::SessionId::new)
        .map(|session| kept.store.session(&session))
        .transpose()
        .map_err(|error| Refusal::Failed {
            reason: error.to_string(),
        })?
        .flatten()
        .and_then(|session| agents.instruction(&session.agent))
        .ok_or_else(nothing)?;
    match kept.reviews.attached_text(&id, instruction) {
        Ok(Some(text)) => Ok(Reply::Attachment { text: text.text }),
        Ok(None) => Err(nothing()),
        Err(error) => Err(refused(review, error)),
    }
}

/// A thread the review does not hold is told as sent: the Keeper asks only for threads it
/// holds.
pub(crate) fn thread_state(
    review: &yaar_review_engine::Review,
    thread: &yaar_review_engine::ThreadId,
) -> yaar_link::ThreadState {
    use yaar_review_engine::ThreadState as Engine;

    match review.state_of(thread) {
        Some(Engine::Resolved) => yaar_link::ThreadState::Resolved,
        Some(Engine::Draft) => yaar_link::ThreadState::Draft,
        Some(Engine::Addressed) => yaar_link::ThreadState::Addressed,
        Some(Engine::WaitsForReviewer) => yaar_link::ThreadState::WaitsForReviewer,
        Some(Engine::Sent) | None => yaar_link::ThreadState::Sent,
    }
}

fn engine_id(review: &ReviewId) -> yaar_review_engine::ReviewId {
    yaar_review_engine::ReviewId::new(review.as_str())
}

fn workspace_of(
    kept: &Kept,
    review: &ReviewId,
) -> Result<yaar_review_engine::WorkspaceId, Refusal> {
    kept.reviews
        .review(&engine_id(review))
        .map(|held| held.workspace().clone())
        .map_err(|error| refused(review, error))
}

fn reviews_changed(workspace: &yaar_review_engine::WorkspaceId) -> Fact {
    Fact::ReviewsChanged {
        workspace: yaar_link::WorkspaceId::new(workspace.as_str()),
    }
}

fn told(
    facts: &mut Vec<Fact>,
    agents: &Agents,
    review: &ReviewId,
    workspace: &yaar_review_engine::WorkspaceId,
) {
    facts.push(reviews_changed(workspace));
    agents.attachment_changed(review);
}

/// `review` is the one the request named: the engine's refusal of a send with nothing to carry does
/// not say which.
fn refused(review: &ReviewId, error: EngineError) -> Refusal {
    match error {
        EngineError::UnknownReview(unknown) => Refusal::UnknownReview {
            review: ReviewId::new(unknown.as_str()),
        },
        EngineError::Refused(ReviewError::UnknownThread(thread)) => Refusal::UnknownThread {
            thread: ThreadId::new(thread.as_str()),
        },
        EngineError::Refused(ReviewError::NothingToSend) => Refusal::NothingToSend {
            review: review.clone(),
        },
        other => Refusal::Failed {
            reason: other.to_string(),
        },
    }
}
