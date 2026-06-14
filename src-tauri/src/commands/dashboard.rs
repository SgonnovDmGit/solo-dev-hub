use crate::db::AppDb;
use crate::models::*;
use chrono;
use tauri::State;

// ── Stats / Graph summaries ──────────────────────────────────────────────────
// Stats are live-computed from the `bugs` and `bug_events` tables — no
// persisted counters, no recalc commands. The legacy `*_stat` write-stubs
// (kept from v0.16.0 stats-table→VIEW migration) were removed in v0.30.0
// (T-000093) along with their unused TS wrappers.

#[tauri::command]
pub fn get_repo_stats_summary(
    db: State<AppDb>,
    repository_id: i64,
) -> Result<StatsSummary, String> {
    db.stats_summary_for_repo(repository_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_project_stats_summary(
    db: State<AppDb>,
    project_id: i64,
) -> Result<StatsSummary, String> {
    db.stats_summary_for_project(project_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_project_graph(db: State<AppDb>, project_id: i64) -> Result<ProjectGraph, String> {
    db.get_project_graph(project_id).map_err(|e| e.to_string())
}

// ── Dashboard v0.17.0 ────────────────────────────────────────────────────────

/// v0.17.0 Dashboard — single aggregator command.
/// Returns full DashboardData snapshot for given filter (period + projects).
#[tauri::command]
pub fn read_dashboard(db: State<AppDb>, filter: DashboardFilter) -> Result<DashboardData, String> {
    let project_ids_opt: Option<Vec<i64>> = match &filter.project_ids {
        Some(ids) if !ids.is_empty() => Some(ids.clone()),
        _ => None,
    };
    let pid_slice: Option<&[i64]> = project_ids_opt.as_deref();

    let p_start = &filter.period.start;
    let p_end = &filter.period.end;
    let cp = filter.compare_period.as_ref();

    // KPI 1: Active bugs
    let active = db.count_active_bugs(pid_slice).map_err(|e| e.to_string())?;
    let critical = db
        .count_active_bugs_with_severity(pid_slice, "critical")
        .map_err(|e| e.to_string())?;

    // KPI 2: Closed in period (+ compare)
    let closed = db
        .count_closed_bugs_in_period(pid_slice, p_start, p_end)
        .map_err(|e| e.to_string())?;
    let closed_prev = if let Some(c) = cp {
        Some(
            db.count_closed_bugs_in_period(pid_slice, &c.start, &c.end)
                .map_err(|e| e.to_string())? as f64,
        )
    } else {
        None
    };

    // KPI 3: Tasks done (from done.md per repo)
    let tasks_done = aggregate_tasks_done(&db, pid_slice, p_start, p_end)?;
    let tasks_done_prev = if let Some(c) = cp {
        Some(aggregate_tasks_done(&db, pid_slice, &c.start, &c.end)? as f64)
    } else {
        None
    };

    // KPI 4: % solve
    let opened = db
        .count_opened_bugs_in_period(pid_slice, p_start, p_end)
        .map_err(|e| e.to_string())?;
    let solve_rate = if closed + opened > 0 {
        Some((closed as f64 / (closed + opened) as f64) * 100.0)
    } else {
        None
    };
    let solve_rate_prev = if let Some(c) = cp {
        let cl = db
            .count_closed_bugs_in_period(pid_slice, &c.start, &c.end)
            .map_err(|e| e.to_string())?;
        let op = db
            .count_opened_bugs_in_period(pid_slice, &c.start, &c.end)
            .map_err(|e| e.to_string())?;
        if cl + op > 0 {
            Some((cl as f64 / (cl + op) as f64) * 100.0)
        } else {
            None
        }
    } else {
        None
    };

    // KPI 5: avg attempts
    let avg_attempts = db
        .avg_attempts_per_closed_in_period(pid_slice, p_start, p_end)
        .map_err(|e| e.to_string())?;
    let avg_attempts_prev = if let Some(c) = cp {
        db.avg_attempts_per_closed_in_period(pid_slice, &c.start, &c.end)
            .map_err(|e| e.to_string())?
    } else {
        None
    };

    // Top-hot (shown when >1 or all projects selected)
    let show_top_hot = match &filter.project_ids {
        Some(ids) => ids.len() > 1 || ids.is_empty(),
        None => true,
    };
    let top_hot = if show_top_hot {
        db.top_hot_projects(pid_slice, Some((p_start, p_end)), 3)
            .map_err(|e| e.to_string())?
    } else {
        vec![]
    };

    // Bugs per day
    let bugs_daily = db
        .bugs_per_day(pid_slice, p_start, p_end)
        .map_err(|e| e.to_string())?;

    // Tasks per day
    let tasks_daily = tasks_daily_flow(&db, pid_slice, p_start, p_end)?;

    // Categories
    let categories = db
        .category_efficiency(pid_slice, p_start, p_end)
        .map_err(|e| e.to_string())?;

    Ok(DashboardData {
        // active_bugs is a point-in-time count, not a period flow — delta
        // vs prev_value would be misleading (e.g. "10 active" today vs
        // "8 active" three months ago says nothing about throughput).
        // The critical-count sub-line carries the only meaningful overlay.
        active_bugs: KpiCard {
            value: Some(active as f64),
            prev_value: None,
            critical_count: Some(critical),
        },
        closed_in_period: KpiCard {
            value: Some(closed as f64),
            prev_value: closed_prev,
            critical_count: None,
        },
        tasks_done: KpiCard {
            value: Some(tasks_done as f64),
            prev_value: tasks_done_prev,
            critical_count: None,
        },
        solve_rate: KpiCard {
            value: solve_rate,
            prev_value: solve_rate_prev,
            critical_count: None,
        },
        attempts_per_closed: KpiCard {
            value: avg_attempts,
            prev_value: avg_attempts_prev,
            critical_count: None,
        },
        top_hot,
        bugs_daily,
        tasks_daily,
        categories,
    })
}

/// Helper: walks all filtered repos that have local_path, parses done.md, sums entries.
fn aggregate_tasks_done(
    db: &AppDb,
    project_ids: Option<&[i64]>,
    start: &str,
    end: &str,
) -> Result<i64, String> {
    let repos = db
        .list_repos_with_local_path(project_ids)
        .map_err(|e| e.to_string())?;
    let mut total = 0i64;
    for r in repos {
        if let Some(lp) = &r.local_path {
            let done_path = std::path::PathBuf::from(lp).join("docs").join("done.md");
            let entries = crate::export::parse_done_entries_in_period(&done_path, start, end)
                .map_err(|e| e.to_string())?;
            total += entries.iter().map(|(_, n)| n).sum::<i64>();
        }
    }
    Ok(total)
}

/// Helper: produces DailyFlowDay vec for tasks (done only).
fn tasks_daily_flow(
    db: &AppDb,
    project_ids: Option<&[i64]>,
    start: &str,
    end: &str,
) -> Result<Vec<DailyFlowDay>, String> {
    let repos = db
        .list_repos_with_local_path(project_ids)
        .map_err(|e| e.to_string())?;
    use std::collections::BTreeMap;
    let mut per_day: BTreeMap<String, i64> = BTreeMap::new();

    for r in repos {
        if let Some(lp) = &r.local_path {
            let done_path = std::path::PathBuf::from(lp).join("docs").join("done.md");
            let entries = crate::export::parse_done_entries_in_period(&done_path, start, end)
                .map_err(|e| e.to_string())?;
            for (date, count) in entries {
                *per_day.entry(date).or_insert(0) += count;
            }
        }
    }

    let start_d =
        chrono::NaiveDate::parse_from_str(start, "%Y-%m-%d").map_err(|e| e.to_string())?;
    let end_d = chrono::NaiveDate::parse_from_str(end, "%Y-%m-%d").map_err(|e| e.to_string())?;
    let today = chrono::Local::now().date_naive();

    let mut out = Vec::new();
    let mut d = start_d;
    while d <= end_d {
        let key = d.format("%Y-%m-%d").to_string();
        out.push(DailyFlowDay {
            date: key.clone(),
            opened: None,
            closed: None,
            done: Some(*per_day.get(&key).unwrap_or(&0)),
            is_future: d > today,
        });
        match d.succ_opt() {
            Some(next) => d = next,
            None => break,
        }
    }
    Ok(out)
}

// ── Activity feed (v0.19.0) ──────────────────────────────────────────────────

#[tauri::command]
pub fn read_recent_activity(
    db: State<AppDb>,
    limit: u32,
) -> Result<Vec<crate::models::ActivityEvent>, String> {
    db.recent_activity(limit).map_err(|e| e.to_string())
}
