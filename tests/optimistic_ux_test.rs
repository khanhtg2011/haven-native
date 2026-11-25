use haven_native::{
    find_reconcile_optimistic_index, format_utc_hhmm, ChannelHistoryLru,
};

#[test]
fn test_reconcile_optimistic_finds_pending_even_after_interleaved_message() {
    let rows = vec![
        ("100".to_string(), "Alice".to_string(), "hello".to_string()),
        ("opt-179500001".to_string(), "Khanhs".to_string(), "my ping".to_string()),
        ("101".to_string(), "Bob".to_string(), "interleaved reply".to_string()),
    ];
    let idx = find_reconcile_optimistic_index(&rows, None, "Khanhs", "Khanhs", "my ping");
    assert_eq!(idx, Some(1));

    let miss = find_reconcile_optimistic_index(&rows, None, "Khanhs", "Bob", "my ping");
    assert_eq!(miss, None);
}

#[test]
fn test_reconcile_optimistic_matches_nonce_even_with_display_name_mismatch() {
    let rows = vec![
        ("opt_179500002_1".to_string(), "khanhs_raw".to_string(), "@bob hello".to_string()),
        ("101".to_string(), "Bob".to_string(), "interleaved".to_string()),
    ];
    let idx = find_reconcile_optimistic_index(
        &rows,
        Some("opt_179500002_1"),
        "khanhs_raw",
        "Khanhs (Display)",
        "@Bob hello!",
    );
    assert_eq!(idx, Some(0));
}

#[test]
fn test_channel_history_lru_evicts_clears_and_deduplicates() {
    let mut cache: ChannelHistoryLru<String> = ChannelHistoryLru::new(12, 80);
    for i in 1..=13 {
        cache.put(format!("ch{}", i), vec![format!("msg-{}", i)]);
    }
    assert!(cache.get("ch1").is_none());
    assert!(cache.get("ch2").is_some());

    cache.put("ch14".to_string(), vec!["msg-14".to_string()]);
    assert!(cache.get("ch2").is_some());
    assert!(cache.get("ch3").is_none());

    cache.append_or_update_by_id("ch14", "msg-14".to_string(), |s| s.clone());
    assert_eq!(cache.get("ch14").unwrap().len(), 1);

    cache.clear();
    assert!(cache.get("ch2").is_none());
    assert!(cache.get("ch14").is_none());
}

#[test]
fn test_format_utc_hhmm_never_panics_on_utf8_or_short_strings() {
    assert_eq!(format_utc_hhmm("2026-11-25T12:34:56Z"), "12:34");
    assert_eq!(format_utc_hhmm("short"), "short");
    assert_eq!(format_utc_hhmm("🔥2026-11-25T12:34"), "🔥2026-11-25T12:34");
    assert_eq!(format_utc_hhmm("🔥🔥🔥abc"), "🔥🔥🔥abc");
}

