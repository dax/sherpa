use sea_orm::DatabaseConnection;

use crate::models::{ai_analyses, review_sessions};
use crate::services::{
    ai_cli::{self, AiPrompt, AiSettings, ModelTier, PrimedSession},
    config::SherpaConfig,
    git_analysis,
    review_session::{ReviewPlan, ReviewSession, ReviewStep},
};

#[derive(Debug, Clone)]
pub struct AnalysisStatus {
    pub approach_ready: bool,
    pub summary_ready: bool,
    pub plan_ready: bool,
    pub step_count: Option<usize>,
    pub steps_explained: usize,
    pub has_failures: bool,
    pub failure_message: Option<String>,
}

fn load_ai_settings() -> Option<AiSettings> {
    ai_cli::load_ai_settings()
}

fn log_ai_error(error: &ai_cli::AiCliError) {
    tracing::error!("Background AI CLI call failed: {error}");

    if let Ok(log_dir) = SherpaConfig::config_dir().map(|d| d.join("logs")) {
        let _ = std::fs::create_dir_all(&log_dir);
        let log_path = log_dir.join("ai_errors.log");
        let timestamp = chrono::Utc::now().format("%H:%M:%S").to_string();
        let line = format!("[{timestamp}] background: {error}\n");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            use std::io::Write;
            let _ = file.write_all(line.as_bytes());
        }
    }
}

fn build_changed_files_summary(session: &ReviewSession) -> String {
    session
        .changed_files
        .iter()
        .map(|f| format!("{}\t{}", f.status, f.path))
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_ai_context(session: &ReviewSession) -> String {
    let files_summary = build_changed_files_summary(session);
    ai_cli::build_context(
        &session.branch,
        &session.default_branch,
        &files_summary,
        &session.diff,
    )
}

pub fn spawn_all_analyses(db: DatabaseConnection, session_db_id: i32, session: ReviewSession) {
    let context = build_ai_context(&session);

    {
        let db = db.clone();
        tokio::spawn(async move {
            let _ = ai_analyses::delete_failures(&db, session_db_id).await;
        });
    }

    let db_for_spawn = db.clone();
    let session_for_spawn = session.clone();
    let context_for_spawn = context.clone();

    tokio::spawn(async move {
        let settings = match load_ai_settings() {
            Some(s) => s,
            None => {
                tracing::warn!("Background: no AI backend configured");
                return;
            }
        };

        let model = settings.model_for_tier(ModelTier::Deep);
        match ai_cli::prime_session(settings.cli, &context_for_spawn, settings.timeout, model).await
        {
            Ok(primed) => {
                tracing::info!(
                    "Background: primed session {} created, forking analyses",
                    primed.session_id
                );

                let _ = review_sessions::update_primed_session(
                    &db_for_spawn,
                    &session_for_spawn.id,
                    &primed.session_id,
                )
                .await;

                if let Ok(mut file_session) = ReviewSession::load(&session_for_spawn.id) {
                    file_session.primed_session_id = Some(primed.session_id.clone());
                    let _ = file_session.save();
                }

                spawn_forked_analyses(db_for_spawn, session_db_id, session_for_spawn, primed).await;
            }
            Err(e) => {
                tracing::warn!(
                    "Background: prime_session failed ({e}), falling back to legacy mode"
                );
                spawn_legacy_analyses(
                    db_for_spawn,
                    session_db_id,
                    session_for_spawn,
                    context_for_spawn,
                )
                .await;
            }
        }
    });
}

async fn spawn_forked_analyses(
    db: DatabaseConnection,
    session_db_id: i32,
    session: ReviewSession,
    primed: PrimedSession,
) {
    let approach_handle = {
        let db = db.clone();
        let primed = primed.clone();
        tokio::spawn(async move {
            spawn_forked_section(db, session_db_id, &primed, "approach", ModelTier::Deep).await;
        })
    };

    let plan_handle = {
        let db = db.clone();
        let primed = primed.clone();
        let session = session.clone();
        tokio::spawn(async move {
            spawn_forked_plan_and_steps(db, session_db_id, &session, &primed).await;
        })
    };

    let _ = tokio::join!(approach_handle, plan_handle);
}

async fn spawn_forked_section(
    db: DatabaseConnection,
    session_db_id: i32,
    primed: &PrimedSession,
    analysis_type: &str,
    tier: ModelTier,
) {
    if let Ok(Some(_)) = ai_analyses::find_cached(&db, session_db_id, analysis_type, None).await {
        tracing::info!("Background {analysis_type}: already cached, skipping");
        return;
    }

    let settings = match load_ai_settings() {
        Some(s) => s,
        None => {
            tracing::warn!("Background {analysis_type}: no AI backend configured");
            return;
        }
    };

    let instruction = match analysis_type {
        "approach" => ai_cli::approach_instruction().to_string(),
        other => {
            tracing::error!("Background: unknown forked section type: {other}");
            return;
        }
    };

    let model = settings.model_for_tier(tier);
    match ai_cli::generate_forked(primed, &instruction, settings.timeout, model).await {
        Ok(content) => {
            if let Err(e) =
                ai_analyses::upsert(&db, session_db_id, analysis_type, None, &content).await
            {
                tracing::error!("Background {analysis_type}: DB error saving: {e}");
            } else {
                tracing::info!("Background {analysis_type}: completed via fork");
            }
        }
        Err(e) => {
            log_ai_error(&e);
            let _ = ai_analyses::record_failure(
                &db,
                session_db_id,
                analysis_type,
                None,
                &e.to_string(),
            )
            .await;
        }
    }
}

async fn spawn_forked_plan_and_steps(
    db: DatabaseConnection,
    session_db_id: i32,
    session: &ReviewSession,
    primed: &PrimedSession,
) {
    if let Ok(Some(model)) = review_sessions::Model::find_by_session_key(&db, &session.id).await {
        let loaded = model.to_review_session();
        if let Some(ref plan) = loaded.review_plan {
            tracing::info!("Background review_plan: already exists, spawning step explanations");
            spawn_forked_step_explanations(db, session_db_id, session, plan, primed);
            return;
        }
    }

    let settings = match load_ai_settings() {
        Some(s) => s,
        None => {
            tracing::warn!("Background review_plan: no AI backend configured");
            return;
        }
    };

    let model = settings.model_for_tier(ModelTier::Deep);
    match ai_cli::generate_forked(
        primed,
        ai_cli::review_plan_instruction(),
        settings.timeout,
        model,
    )
    .await
    {
        Ok(raw_response) => {
            match ai_cli::extract_json_from_response(&raw_response).and_then(|json| {
                serde_json::from_str::<ReviewPlanResponse>(&json)
                    .map_err(|e| format!("Failed to parse review plan JSON: {e}"))
            }) {
                Ok(plan_response) => {
                    let plan = ReviewPlan {
                        steps: plan_response.steps,
                        generated_at: chrono::Utc::now().format("%H:%M:%S").to_string(),
                    };
                    let plan_json = serde_json::to_string(&plan).ok();

                    if let Err(e) =
                        review_sessions::update_review_plan(&db, &session.id, plan_json).await
                    {
                        tracing::error!("Background review_plan: DB error saving plan: {e}");
                        return;
                    }

                    if let Ok(mut file_session) = ReviewSession::load(&session.id) {
                        file_session.review_plan = Some(plan.clone());
                        let _ = file_session.save();
                    }

                    tracing::info!(
                        "Background review_plan: completed via fork with {} steps",
                        plan.steps.len()
                    );

                    spawn_forked_step_explanations(db, session_db_id, session, &plan, primed);
                }
                Err(error) => {
                    tracing::error!("Background review_plan: parse error: {error}");
                    let _ = ai_analyses::record_failure(
                        &db,
                        session_db_id,
                        "review_plan",
                        None,
                        &error,
                    )
                    .await;
                }
            }
        }
        Err(e) => {
            log_ai_error(&e);
            let _ = ai_analyses::record_failure(
                &db,
                session_db_id,
                "review_plan",
                None,
                &e.to_string(),
            )
            .await;
        }
    }
}

fn spawn_forked_step_explanations(
    db: DatabaseConnection,
    session_db_id: i32,
    session: &ReviewSession,
    plan: &ReviewPlan,
    primed: &PrimedSession,
) {
    for (i, step) in plan.steps.iter().enumerate() {
        let step_number = (i + 1) as i32;
        let db = db.clone();
        let session = session.clone();
        let step = step.clone();
        let primed = primed.clone();

        tokio::spawn(async move {
            if let Ok(Some(_)) =
                ai_analyses::find_cached(&db, session_db_id, "step_explanation", Some(step_number))
                    .await
            {
                tracing::info!("Background step_explanation {step_number}: already cached");
                return;
            }

            let settings = match load_ai_settings() {
                Some(s) => s,
                None => {
                    tracing::warn!(
                        "Background step_explanation {step_number}: no AI backend configured"
                    );
                    return;
                }
            };

            let file_refs_tuples: Vec<(String, Option<(usize, usize)>)> = step
                .file_refs
                .iter()
                .map(|f| (f.path.clone(), f.diff_lines))
                .collect();
            let step_diff = git_analysis::extract_diff_for_files(&session.diff, &file_refs_tuples);

            let instruction = ai_cli::step_explanation_instruction(&step.title, &step_diff);
            let model = settings.model_for_tier(ModelTier::Fast);
            match ai_cli::generate_forked(&primed, &instruction, settings.timeout, model).await {
                Ok(content) => {
                    if let Err(e) = ai_analyses::upsert(
                        &db,
                        session_db_id,
                        "step_explanation",
                        Some(step_number),
                        &content,
                    )
                    .await
                    {
                        tracing::error!("Background step_explanation {step_number}: DB error: {e}");
                    } else {
                        tracing::info!(
                            "Background step_explanation {step_number}: completed via fork"
                        );
                    }
                }
                Err(e) => {
                    log_ai_error(&e);
                    let _ = ai_analyses::record_failure(
                        &db,
                        session_db_id,
                        "step_explanation",
                        Some(step_number),
                        &e.to_string(),
                    )
                    .await;
                }
            }
        });
    }
}

async fn spawn_legacy_analyses(
    db: DatabaseConnection,
    session_db_id: i32,
    session: ReviewSession,
    context: String,
) {
    let approach_handle = {
        let db = db.clone();
        let context = context.clone();
        tokio::spawn(async move {
            spawn_section_analysis(
                db,
                session_db_id,
                "approach",
                ai_cli::approach_instruction(),
                &context,
                ModelTier::Deep,
            )
            .await;
        })
    };

    let plan_handle = {
        let db = db.clone();
        let context = context.clone();
        let session = session.clone();
        tokio::spawn(async move {
            spawn_review_plan_and_steps(db, session_db_id, &session, &context).await;
        })
    };

    let _ = tokio::join!(approach_handle, plan_handle);
}

async fn spawn_section_analysis(
    db: DatabaseConnection,
    session_db_id: i32,
    analysis_type: &str,
    instruction: &str,
    context: &str,
    tier: ModelTier,
) {
    if let Ok(Some(_)) = ai_analyses::find_cached(&db, session_db_id, analysis_type, None).await {
        tracing::info!("Background {analysis_type}: already cached, skipping");
        return;
    }

    let settings = match load_ai_settings() {
        Some(s) => s,
        None => {
            tracing::warn!("Background {analysis_type}: no AI backend configured");
            return;
        }
    };

    let prompt = AiPrompt {
        context: context.to_string(),
        instruction: instruction.to_string(),
    };

    let model = settings.model_for_tier(tier);
    match ai_cli::generate_with_timeout(settings.cli, &prompt, settings.timeout, model).await {
        Ok(content) => {
            if let Err(e) =
                ai_analyses::upsert(&db, session_db_id, analysis_type, None, &content).await
            {
                tracing::error!("Background {analysis_type}: DB error saving: {e}");
            } else {
                tracing::info!("Background {analysis_type}: completed and cached");
            }
        }
        Err(e) => {
            log_ai_error(&e);
            let _ = ai_analyses::record_failure(
                &db,
                session_db_id,
                analysis_type,
                None,
                &e.to_string(),
            )
            .await;
        }
    }
}

async fn spawn_review_plan_and_steps(
    db: DatabaseConnection,
    session_db_id: i32,
    session: &ReviewSession,
    context: &str,
) {
    if let Ok(Some(model)) = review_sessions::Model::find_by_session_key(&db, &session.id).await {
        let loaded = model.to_review_session();
        if let Some(ref plan) = loaded.review_plan {
            tracing::info!("Background review_plan: already exists, spawning step explanations");
            spawn_step_explanations(db, session_db_id, session, plan);
            return;
        }
    }

    let settings = match load_ai_settings() {
        Some(s) => s,
        None => {
            tracing::warn!("Background review_plan: no AI backend configured");
            return;
        }
    };

    let prompt = AiPrompt {
        context: context.to_string(),
        instruction: ai_cli::review_plan_instruction().to_string(),
    };

    let model = settings.model_for_tier(ModelTier::Deep);
    match ai_cli::generate_with_timeout(settings.cli, &prompt, settings.timeout, model).await {
        Ok(raw_response) => {
            match ai_cli::extract_json_from_response(&raw_response).and_then(|json| {
                serde_json::from_str::<ReviewPlanResponse>(&json)
                    .map_err(|e| format!("Failed to parse review plan JSON: {e}"))
            }) {
                Ok(plan_response) => {
                    let plan = ReviewPlan {
                        steps: plan_response.steps,
                        generated_at: chrono::Utc::now().format("%H:%M:%S").to_string(),
                    };
                    let plan_json = serde_json::to_string(&plan).ok();

                    if let Err(e) =
                        review_sessions::update_review_plan(&db, &session.id, plan_json).await
                    {
                        tracing::error!("Background review_plan: DB error saving plan: {e}");
                        return;
                    }

                    if let Ok(mut file_session) = ReviewSession::load(&session.id) {
                        file_session.review_plan = Some(plan.clone());
                        let _ = file_session.save();
                    }

                    tracing::info!(
                        "Background review_plan: completed with {} steps, spawning step explanations",
                        plan.steps.len()
                    );

                    spawn_step_explanations(db, session_db_id, session, &plan);
                }
                Err(error) => {
                    tracing::error!("Background review_plan: parse error: {error}");
                    let _ = ai_analyses::record_failure(
                        &db,
                        session_db_id,
                        "review_plan",
                        None,
                        &error,
                    )
                    .await;
                }
            }
        }
        Err(e) => {
            log_ai_error(&e);
            let _ = ai_analyses::record_failure(
                &db,
                session_db_id,
                "review_plan",
                None,
                &e.to_string(),
            )
            .await;
        }
    }
}

#[derive(serde::Deserialize)]
struct ReviewPlanResponse {
    steps: Vec<ReviewStep>,
}

fn spawn_step_explanations(
    db: DatabaseConnection,
    session_db_id: i32,
    session: &ReviewSession,
    plan: &ReviewPlan,
) {
    for (i, step) in plan.steps.iter().enumerate() {
        let step_number = (i + 1) as i32;
        let db = db.clone();
        let session = session.clone();
        let step = step.clone();

        tokio::spawn(async move {
            if let Ok(Some(_)) =
                ai_analyses::find_cached(&db, session_db_id, "step_explanation", Some(step_number))
                    .await
            {
                tracing::info!("Background step_explanation {step_number}: already cached");
                return;
            }

            let settings = match load_ai_settings() {
                Some(s) => s,
                None => {
                    tracing::warn!(
                        "Background step_explanation {step_number}: no AI backend configured"
                    );
                    return;
                }
            };

            let file_refs_tuples: Vec<(String, Option<(usize, usize)>)> = step
                .file_refs
                .iter()
                .map(|f| (f.path.clone(), f.diff_lines))
                .collect();
            let step_diff = git_analysis::extract_diff_for_files(&session.diff, &file_refs_tuples);

            let context = build_ai_context(&session);
            let prompt = AiPrompt {
                context,
                instruction: ai_cli::step_explanation_instruction(&step.title, &step_diff),
            };

            let model = settings.model_for_tier(ModelTier::Fast);
            match ai_cli::generate_with_timeout(settings.cli, &prompt, settings.timeout, model)
                .await
            {
                Ok(content) => {
                    if let Err(e) = ai_analyses::upsert(
                        &db,
                        session_db_id,
                        "step_explanation",
                        Some(step_number),
                        &content,
                    )
                    .await
                    {
                        tracing::error!("Background step_explanation {step_number}: DB error: {e}");
                    } else {
                        tracing::info!(
                            "Background step_explanation {step_number}: completed and cached"
                        );
                    }
                }
                Err(e) => {
                    log_ai_error(&e);
                    let _ = ai_analyses::record_failure(
                        &db,
                        session_db_id,
                        "step_explanation",
                        Some(step_number),
                        &e.to_string(),
                    )
                    .await;
                }
            }
        });
    }
}

/// Spawn background tasks for a step that was completed by an agent in live mode.
/// The agent pushes the diff directly, so we don't extract from session.diff.
/// This spawns step_explanation (if not already cached) and step_relation (for step >= 2).
pub fn spawn_live_step_analyses(
    db: DatabaseConnection,
    session_db_id: i32,
    session: ReviewSession,
    step_number: usize,
    step_diff: String,
) {
    let step_idx = step_number - 1;
    let plan = match session.review_plan.as_ref() {
        Some(p) => p.clone(),
        None => return,
    };
    let step = match plan.steps.get(step_idx) {
        Some(s) => s.clone(),
        None => return,
    };

    {
        let db = db.clone();
        let session = session.clone();
        let step_title = step.title.clone();
        let diff = step_diff.clone();
        tokio::spawn(async move {
            let sn = step_number as i32;
            if let Ok(Some(_)) =
                ai_analyses::find_cached(&db, session_db_id, "step_explanation", Some(sn)).await
            {
                tracing::info!("Live step_explanation {sn}: already cached");
                return;
            }

            let settings = match load_ai_settings() {
                Some(s) => s,
                None => {
                    tracing::warn!("Live step_explanation {sn}: no AI backend configured");
                    return;
                }
            };

            let instruction = ai_cli::step_explanation_instruction(&step_title, &diff);
            let context = build_ai_context(&session);
            let prompt = AiPrompt {
                context,
                instruction,
            };

            let model = settings.model_for_tier(ModelTier::Fast);
            match ai_cli::generate_with_timeout(settings.cli, &prompt, settings.timeout, model)
                .await
            {
                Ok(content) => {
                    if let Err(e) = ai_analyses::upsert(
                        &db,
                        session_db_id,
                        "step_explanation",
                        Some(sn),
                        &content,
                    )
                    .await
                    {
                        tracing::error!("Live step_explanation {sn}: DB error: {e}");
                    } else {
                        tracing::info!("Live step_explanation {sn}: completed");
                    }
                }
                Err(e) => {
                    log_ai_error(&e);
                    let _ = ai_analyses::record_failure(
                        &db,
                        session_db_id,
                        "step_explanation",
                        Some(sn),
                        &e.to_string(),
                    )
                    .await;
                }
            }
        });
    }

    if step_number >= 2 {
        let prev_title = plan.steps[step_idx - 1].title.clone();
        let current_title = step.title.clone();
        let diff = step_diff;
        tokio::spawn(async move {
            let sn = step_number as i32;
            if let Ok(Some(_)) =
                ai_analyses::find_cached(&db, session_db_id, "step_relation", Some(sn)).await
            {
                tracing::info!("Live step_relation {sn}: already cached");
                return;
            }

            let settings = match load_ai_settings() {
                Some(s) => s,
                None => {
                    tracing::warn!("Live step_relation {sn}: no AI backend configured");
                    return;
                }
            };

            let instruction = ai_cli::step_relation_instruction(&prev_title, &current_title, &diff);
            let context = build_ai_context(&session);
            let prompt = AiPrompt {
                context,
                instruction,
            };

            let model = settings.model_for_tier(ModelTier::Fast);
            match ai_cli::generate_with_timeout(settings.cli, &prompt, settings.timeout, model)
                .await
            {
                Ok(content) => {
                    if let Err(e) =
                        ai_analyses::upsert(&db, session_db_id, "step_relation", Some(sn), &content)
                            .await
                    {
                        tracing::error!("Live step_relation {sn}: DB error: {e}");
                    } else {
                        tracing::info!("Live step_relation {sn}: completed");
                    }
                }
                Err(e) => {
                    log_ai_error(&e);
                    let _ = ai_analyses::record_failure(
                        &db,
                        session_db_id,
                        "step_relation",
                        Some(sn),
                        &e.to_string(),
                    )
                    .await;
                }
            }
        });
    }
}

pub async fn check_analysis_status(
    db: &DatabaseConnection,
    session_db_id: i32,
    session_key: &str,
) -> AnalysisStatus {
    let approach_ready = ai_analyses::find_cached(db, session_db_id, "approach", None)
        .await
        .ok()
        .flatten()
        .is_some();

    let summary_ready = approach_ready;

    let (plan_ready, step_count) =
        match review_sessions::Model::find_by_session_key(db, session_key).await {
            Ok(Some(model)) => {
                let session = model.to_review_session();
                match session.review_plan {
                    Some(ref plan) => (true, Some(plan.steps.len())),
                    None => (false, None),
                }
            }
            _ => (false, None),
        };

    let steps_explained = if let Some(count) = step_count {
        let mut explained = 0;
        for i in 1..=count {
            if ai_analyses::find_cached(db, session_db_id, "step_explanation", Some(i as i32))
                .await
                .ok()
                .flatten()
                .is_some()
            {
                explained += 1;
            }
        }
        explained
    } else {
        0
    };

    let has_failures = ai_analyses::has_failures(db, session_db_id)
        .await
        .unwrap_or(false);
    let failure_message = if has_failures {
        ai_analyses::first_failure_message(db, session_db_id)
            .await
            .unwrap_or(None)
    } else {
        None
    };

    AnalysisStatus {
        approach_ready,
        summary_ready,
        plan_ready,
        step_count,
        steps_explained,
        has_failures,
        failure_message,
    }
}
