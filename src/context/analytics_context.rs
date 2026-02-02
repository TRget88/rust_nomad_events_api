use crate::errors::AppError;
//use crate::models::analytics::DailyAnalytics;
use crate::models::analytics::RealtimeDashboard;
use crate::models::analytics::TodayMetrics;
//use crate::models::analytics::WeekMetrics;
//use crate::models::analytics::AllTimeMetrics;
use chrono::{DateTime, NaiveDate, Utc};
use sqlx::SqlitePool;

pub struct AnalyticsContext {
    pool: SqlitePool,
}

impl AnalyticsContext {
    // Get real-time dashboard
    //pub async fn get_realtime_dashboard(&self) -> Result<RealtimeDashboard, AppError> {
    //let today = self.get_today_metrics().await?;
    //let yesterday = self.get_yesterday_metrics().await?;
    //let this_week = self.get_week_metrics().await?;
    //let all_time = self.get_all_time_metrics().await?;
    //let recent_activity = self.get_recent_activity().await?;
    //let top_events = self.get_top_events(10).await?;
    //let top_creators = self.get_top_creators(10).await?;
    //
    //Ok(RealtimeDashboard {
    //timestamp: Utc::now().to_rfc3339(),
    //users_online: self.count_online_users().await?,
    //active_sessions: self.count_active_sessions().await?,
    //today,
    //yesterday,
    //this_week,
    //all_time,
    //recent_activity,
    //top_events,
    //top_creators,
    //})
    //}

    // Track user activity
    pub async fn log_activity(
        &self,
        user_id: &str,
        activity_type: &str,
        resource_type: Option<&str>,
        resource_id: Option<&str>,
    ) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO user_activity_log 
             (user_id, activity_type, resource_type, resource_id)
             VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(user_id)
        .bind(activity_type)
        .bind(resource_type)
        .bind(resource_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    // Increment event view count
    pub async fn track_event_view(
        &self,
        event_id: &str,
        user_id: Option<&str>,
    ) -> Result<(), AppError> {
        // Update analytics
        sqlx::query(
            "INSERT INTO event_analytics (event_id, views, unique_viewers, last_viewed_at)
             VALUES (?1, 1, 1, datetime('now'))
             ON CONFLICT(event_id) DO UPDATE SET
             views = views + 1,
             unique_viewers = unique_viewers + CASE 
                 WHEN ?2 IS NOT NULL THEN 1 
                 ELSE 0 
             END,
             last_viewed_at = datetime('now')",
        )
        .bind(event_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    // Get today's metrics -- rewrite this as a get by date then increment accordingly instead of a new method for each and every thing.
    //async fn get_today_metrics(&self) -> Result<TodayMetrics, AppError> {
    //let today = Utc::now().format("%Y-%m-%d").to_string();
    //
    //let metrics = sqlx::query_as::<_, TodayMetrics>(
    //"SELECT
    //new_users,
    //active_users,
    //total_logins as logins,
    //events_created,
    //microevents_created,
    //total_favorites as favorites_added,
    //page_views
    //FROM daily_analytics
    //WHERE date = ?1"
    //)
    //.bind(&today)
    //.fetch_optional(&self.pool)
    //.await?
    //.unwrap_or_default();
    //
    //Ok(metrics)
    //}

    //async fn get_metrics_by_date(&self) -> Result<_, AppError> {
    //let today = Utc::now().format("%Y-%m-%d").to_string();
    //
    //let metrics = sqlx::query_as::<_, _>(
    //"SELECT
    //new_users,
    //active_users,
    //total_logins as logins,
    //events_created,
    //microevents_created,
    //total_favorites as favorites_added,
    //page_views
    //FROM daily_analytics
    //WHERE date = ?1"
    //)
    //.bind(&today)
    //.fetch_optional(&self.pool)
    //.await?
    //.unwrap_or_default();
    //
    //Ok(metrics)
    //}
}
