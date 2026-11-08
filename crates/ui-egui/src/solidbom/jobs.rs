//! The application's polling seam for SolidBOM work that runs beside the window: the shared
//! service host and each session's local source analysis.
//!
//! A frame calls [`advance`] once. It never waits for a request: it installs the configured host,
//! takes finished analyses, routes a service completion to the session that submitted it, and
//! asks for a repaint while anything is still pending. The analysis itself runs in a scratch
//! session on a worker thread (see `solidcraft_engine::solidbom::jobs`), so the window keeps
//! drawing and the existing design keeps answering selection and navigation while it runs.

use std::sync::Arc;
use std::time::Duration;

use solidcraft_engine::Session;

use crate::SolveApp;
use crate::documents;

/// Poll every SolidBOM worker once and deliver each answer to its owning session.
pub(crate) fn advance(app: &mut SolveApp, ctx: &egui::Context) {
    let configured = app.services.solidbom.clone();
    if app.session.solidbom_host.is_none() {
        app.session.solidbom_host = configured.clone();
    }
    for slot in app.docs.background_mut() {
        if slot.session.solidbom_host.is_none() {
            slot.session.solidbom_host = configured.clone();
        }
    }
    // A completion may bind a session to a reviewed source: an import publishes its analysis here
    // and a reopen completion binds one later. Capture the policy generations first, so a binding
    // cancels whatever that session was authoring for the design it replaced.
    let active_review_binding = app.review_binding();
    let background_review_generations: Vec<u64> = app.docs.background().map(|session| session.review_generation()).collect();
    // Local analyses are per session: each one takes only its own answer, so drain them in place.
    app.session.drain_solidbom_analysis();
    for slot in app.docs.background_mut() {
        slot.session.drain_solidbom_analysis();
    }
    let mut hosts = Vec::<solidcraft_engine::solidbom::HostHandle>::new();
    for host in std::iter::once(&app.session).chain(app.docs.background()).filter_map(|session| session.solidbom_host.as_ref()) {
        if !hosts.iter().any(|known| Arc::ptr_eq(known, host)) {
            hosts.push(host.clone());
        }
    }
    for host in hosts {
        for completion in host.poll() {
            let owns = |session: &Session| {
                session.solidbom_host.as_ref().is_some_and(|installed| Arc::ptr_eq(installed, &host))
                    && session.solidbom.pending.contains_key(&completion.job_id)
            };
            if owns(&app.session) {
                app.session.apply_solidbom_completion(completion);
            } else if let Some(slot) = app.docs.background_mut().find(|slot| owns(&slot.session)) {
                slot.session.apply_solidbom_completion(completion);
            }
        }
    }
    app.cancel_authoring_if_review_changed(active_review_binding);
    for (slot, generation) in app.docs.background_mut().zip(background_review_generations) {
        if slot.session.review_generation() != generation {
            slot.cancel_authoring_for_review_change();
        }
    }
    let pending_jobs = std::iter::once(&app.session).chain(app.docs.background()).any(|session| !session.solidbom.pending.is_empty());
    let pending_analysis = std::iter::once(&app.session).chain(app.docs.background()).any(|session| session.solidbom_analysis.is_some());
    if pending_jobs || pending_analysis {
        // Polling is the only wakeup path: keep frames coming while anything runs.
        ctx.request_repaint_after(Duration::from_millis(100));
    }
    documents::continue_review_close(app);
}
