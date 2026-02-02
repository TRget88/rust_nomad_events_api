use crate::models::database_models::*;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

// ============================================================================
// Real-time Dashboard - Current state of the site
// ============================================================================
#[derive(Debug, Serialize, Deserialize)]
pub struct RealtimeDashboard {
    pub timestamp: String,

    // Right Now
    pub users_online: i32, // Active in last 5 minutes
    pub active_sessions: i32,

    // Today So Far
    pub today: TodayMetrics,

    // Yesterday (for comparison)
    pub yesterday: TodayMetrics,

    // This Week
    pub this_week: WeekMetrics,

    // All Time
    pub all_time: AllTimeMetrics,

    // Recent Activity (last 24 hours)
    pub recent_activity: RecentActivity,

    // Top Content
    pub top_events: Vec<EventStats>,
    pub top_creators: Vec<CreatorStats>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TodayMetrics {
    pub new_users: i32,
    pub active_users: i32,
    pub logins: i32,
    pub events_created: i32,
    pub microevents_created: i32,
    pub favorites_added: i32,
    pub page_views: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WeekMetrics {
    pub new_users: i32,
    pub active_users: i32,
    pub events_created: i32,
    pub microevents_created: i32,
    pub total_engagement: i32, // Favorites + saves + creates
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AllTimeMetrics {
    pub total_users: i32,
    pub total_events: i32,
    pub total_microevents: i32,
    pub total_favorites: i32,
    pub total_saves: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RecentActivity {
    pub recent_signups: Vec<RecentUser>,
    pub recent_events: Vec<RecentEvent>,
    pub recent_logins: Vec<RecentLogin>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RecentUser {
    pub user_id: String,
    pub user_name: String,
    pub email: String,
    pub signed_up_at: String,
    pub oauth_provider: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RecentEvent {
    pub event_id: String,
    pub title: String,
    pub creator_name: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RecentLogin {
    pub user_name: String,
    pub login_at: String,
    pub login_count: i32, // Total login count for this user
}

// ============================================================================
// User Behavior Analytics
// ============================================================================
#[derive(Debug, Serialize, Deserialize)]
pub struct UserBehaviorMetrics {
    pub date_range: DateRange,

    // Retention
    pub day_1_retention: f64,  // % who return next day
    pub day_7_retention: f64,  // % who return in 7 days
    pub day_30_retention: f64, // % who return in 30 days

    // Activity Distribution
    pub power_users: i32,   // 10+ actions/day
    pub active_users: i32,  // 1-9 actions/day
    pub passive_users: i32, // View only
    pub churned_users: i32, // No activity in 30 days

    // Engagement Funnel
    pub visitors: i32,  // Total unique visitors
    pub signups: i32,   // Converted to signup
    pub activated: i32, // Created first event
    pub retained: i32,  // Came back 3+ times

    // Time-based patterns
    pub peak_hour: i32,   // 0-23, busiest hour
    pub peak_day: String, // Monday, Tuesday, etc.
    pub avg_sessions_per_user: f64,
    pub avg_session_duration_minutes: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DateRange {
    pub start_date: String,
    pub end_date: String,
}

// ============================================================================
// Growth Metrics
// ============================================================================
#[derive(Debug, Serialize, Deserialize)]
pub struct GrowthMetrics {
    pub period: String, // "daily", "weekly", "monthly"

    // User Growth
    pub user_growth_rate: f64,     // % change
    pub user_growth_absolute: i32, // Net new users

    // Content Growth
    pub event_growth_rate: f64,
    pub event_growth_absolute: i32,

    // Engagement Growth
    pub engagement_growth_rate: f64,
    pub engagement_growth_absolute: i32,

    // Comparison to previous period
    pub vs_previous_period: PeriodComparison,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PeriodComparison {
    pub users: f64, // % change
    pub events: f64,
    pub engagement: f64,
    pub revenue: f64, // If applicable
}

// ============================================================================
// System Health Metrics
// ============================================================================
#[derive(Debug, Serialize, Deserialize, FromRow)]
pub struct SystemHealthMetrics {
    pub timestamp: String,

    // API Performance
    pub avg_response_time_ms: f64,
    pub p95_response_time_ms: f64, // 95th percentile
    pub p99_response_time_ms: f64, // 99th percentile

    // Error Rates
    pub total_requests: i32,
    pub successful_requests: i32,
    pub failed_requests: i32,
    pub error_rate: f64, // Percentage

    // By endpoint (top 5 slowest)
    pub slowest_endpoints: Vec<EndpointMetric>,

    // Error breakdown
    pub errors_by_type: Vec<ErrorMetric>,

    // Database
    pub db_connection_pool_size: i32,
    pub db_active_connections: i32,
    pub db_avg_query_time_ms: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EndpointMetric {
    pub endpoint: String,
    pub method: String, // GET, POST, etc.
    pub avg_response_time_ms: f64,
    pub request_count: i32,
    pub error_count: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorMetric {
    pub error_type: String, // "DatabaseError", "Unauthorized", etc.
    pub count: i32,
    pub percentage: f64,
    pub first_seen: String,
    pub last_seen: String,
}

// ============================================================================
// Cohort Analysis
// ============================================================================
#[derive(Debug, Serialize, Deserialize)]
pub struct CohortAnalysis {
    pub cohort_date: String, // Week/Month users signed up
    pub cohort_size: i32,

    // Retention by period
    pub week_0: f64, // Always 100%
    pub week_1: f64, // % still active
    pub week_2: f64,
    pub week_3: f64,
    pub week_4: f64,
    pub week_8: f64,
    pub week_12: f64,
}
