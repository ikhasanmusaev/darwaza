//! Ответ человека на планы агента (#205, #206).
//!
//! Окно спрашивает, что ждёт решения, и отвечает. Ничего больше: сам план
//! составляет агент, выполняет — сторона инструментов.

use crate::agent::tools::emit_plan_gone;
use crate::agent::{PendingPlan, PlanGoneReason, PlanVerdict};
use crate::AppState;
use tauri::{AppHandle, State};
use tracing::info;

#[tauri::command]
pub async fn agent_plans_pending(state: State<'_, AppState>) -> Result<Vec<PendingPlan>, String> {
    Ok(state.plans.list().await)
}

/// Подтвердить план.
///
/// `false` означает, что плана уже нет: агент не дождался и ушёл, или
/// подтвердили дважды. Это не ошибка — окну надо просто убрать карточку.
#[tauri::command]
pub async fn agent_plan_approve(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    let answered = state.plans.resolve(&id, PlanVerdict::Approved).await;
    info!(target: "agent", "план {id} подтверждён: {answered}");
    Ok(answered)
}

#[tauri::command]
pub async fn agent_plan_reject(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<bool, String> {
    let answered = state.plans.resolve(&id, PlanVerdict::Rejected).await;
    info!(target: "agent", "план {id} отклонён: {answered}");
    emit_plan_gone(&app, &id, PlanGoneReason::Rejected);
    Ok(answered)
}

/// Отклонить всё разом — «стоп» на случай, когда агент делает не то.
///
/// Именно отклонить, а не забыть: агент должен узнать, что ему отказали, а
/// не остаться ждать ответа навсегда.
#[tauri::command]
pub async fn agent_plans_reject_all(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    let ids = state.plans.reject_all().await;
    for id in &ids {
        emit_plan_gone(&app, id, PlanGoneReason::Rejected);
    }
    let count = ids.len();
    info!(target: "agent", "отклонено планов разом: {count}");
    Ok(count)
}
