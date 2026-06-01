// src/util.rs
//
// Cross-cutting helpers that don't fit the routes → logic → context →
// models layering. Functions here must be:
//   - Pure (no I/O, no DB, no env vars; the `AppError` dependency is fine
//     because it's pure data, no I/O).
//   - Standalone (no awareness of domain types — these are mechanical
//     string/byte/integer operations that any layer can call).
//
// If a helper grows enough state or contextual knowledge to need a struct,
// it belongs in its own module — `util.rs` is for the small stuff.

use crate::errors::AppError;

/// Defaults and bounds for `?limit=`/`?offset=` query params on list
/// endpoints. `DEFAULT_LIMIT` is wide enough that the original
/// `/event/search` frontend (which doesn't yet send `limit`) sees the same
/// behavior at realistic densities. `MAX_LIMIT` matches the existing
/// `EventContext::get_by_id_list` cap so all paginated paths agree.
pub const DEFAULT_PAGINATION_LIMIT: i64 = 200;
pub const MAX_PAGINATION_LIMIT: i64 = 500;

/// Validate `limit` and `offset` query params, returning the values to bind
/// into the SQL. Defaults apply when the caller omits the param. Explicit
/// out-of-range values are rejected with 400 rather than silently clamped —
/// a caller that passed `limit=-1` or `limit=99999` has a bug they want to
/// see, not a silent reinterpretation.
///
/// Returns `(limit, offset)` where:
///   - `limit` is in `[1, MAX_PAGINATION_LIMIT]`.
///   - `offset` is `>= 0`.
///
/// Lives in `util.rs` (not `event_logic.rs`) so every list endpoint that
/// uses `?limit=&offset=` can share the same defaults, caps, and error
/// messages — preventing drift if a future "make audit-log a different cap"
/// instinct overrides the unified pattern.
pub fn validate_pagination(
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<(i64, i64), AppError> {
    let l = limit.unwrap_or(DEFAULT_PAGINATION_LIMIT);
    if !(1..=MAX_PAGINATION_LIMIT).contains(&l) {
        return Err(AppError::ValidationError(format!(
            "limit must be in [1, {}] (got {})",
            MAX_PAGINATION_LIMIT, l
        )));
    }
    let o = offset.unwrap_or(0);
    if o < 0 {
        return Err(AppError::ValidationError(format!(
            "offset must be >= 0 (got {})",
            o
        )));
    }
    Ok((l, o))
}

/// Escape `%`, `_`, and `\` so user-supplied search input matches literally
/// against SQL LIKE patterns. Use with `ESCAPE '\\'` on the query side so
/// SQLite knows which character is the escape.
///
/// Without this, a user typing `100%` would match every row in the table.
pub fn escape_like_pattern(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_through_safe_characters() {
        assert_eq!(escape_like_pattern("hello world"), "hello world");
        assert_eq!(escape_like_pattern("Renaissance"), "Renaissance");
    }

    #[test]
    fn escapes_percent_sign() {
        assert_eq!(escape_like_pattern("100%"), "100\\%");
        assert_eq!(escape_like_pattern("%off%"), "\\%off\\%");
    }

    #[test]
    fn escapes_underscore() {
        assert_eq!(escape_like_pattern("event_name"), "event\\_name");
    }

    #[test]
    fn escapes_backslash() {
        assert_eq!(escape_like_pattern("a\\b"), "a\\\\b");
    }

    #[test]
    fn handles_combined_specials() {
        // All three together — order preserved, each one escaped.
        assert_eq!(escape_like_pattern("a%b_c\\d"), "a\\%b\\_c\\\\d");
    }

    #[test]
    fn empty_string_is_empty() {
        assert_eq!(escape_like_pattern(""), "");
    }

    // ========================================================================
    // validate_pagination
    // ========================================================================

    #[test]
    fn validate_pagination_defaults_when_unset() {
        let (l, o) = validate_pagination(None, None).unwrap();
        assert_eq!(l, DEFAULT_PAGINATION_LIMIT);
        assert_eq!(o, 0);
    }

    #[test]
    fn validate_pagination_accepts_in_range_values() {
        let (l, o) = validate_pagination(Some(50), Some(100)).unwrap();
        assert_eq!(l, 50);
        assert_eq!(o, 100);
    }

    #[test]
    fn validate_pagination_accepts_boundary_values() {
        // limit=1 (smallest legal page), limit=MAX (largest), offset=0.
        let (l, _) = validate_pagination(Some(1), Some(0)).unwrap();
        assert_eq!(l, 1);
        let (l, _) = validate_pagination(Some(MAX_PAGINATION_LIMIT), Some(0)).unwrap();
        assert_eq!(l, MAX_PAGINATION_LIMIT);
    }

    #[test]
    fn validate_pagination_rejects_zero_or_negative_limit() {
        // Loud rejection over silent clamping — caller has a bug.
        assert!(validate_pagination(Some(0), None).is_err());
        assert!(validate_pagination(Some(-1), None).is_err());
    }

    #[test]
    fn validate_pagination_rejects_limit_over_cap() {
        // The cap is the contract — `?limit=99999` is a caller bug, not
        // a request to return 500 silently.
        let err = validate_pagination(Some(MAX_PAGINATION_LIMIT + 1), None).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn validate_pagination_rejects_negative_offset() {
        assert!(validate_pagination(None, Some(-1)).is_err());
    }

    #[test]
    fn validate_pagination_allows_large_offset() {
        // Offset has no upper bound by design — paginating deep into a
        // result set is legitimate. The query just returns no rows once
        // offset > total.
        let (_, o) = validate_pagination(None, Some(1_000_000)).unwrap();
        assert_eq!(o, 1_000_000);
    }
}
