use haven_native::{find_reconcile_optimistic_index, ChannelHistoryLru};

#[test]
fn test_reconcile_optimistic_finds_pending_even_after_interleaved_message() {
    let rows = vec![
        ("100".to_string(), "Alice".to_string(), "hello".to_string()),
        ("opt-179500001".to_string(), "Khanhs".to_string(), "my ping".to_string()),
        ("101".to_string(), "Bob".to_string(), "interleaved reply".to_string()),
    ];
    let idx = find_reconcile_optimistic_index(&rows, "Khanhs", "Khanhs", "my ping");
    assert_eq!(idx, Some(1));

    let miss = find_reconcile_optimistic_index(&rows, "Khanhs", "Bob", "my ping");
    assert_eq!(miss, None);
}

#[test]
fn test_channel_history_lru_evicts_oldest_and_updates_on_new_message() {
    let mut cache: ChannelHistoryLru<String> = ChannelHistoryLru::new(12, 80);
    for i in 1..=13 {
        cache.put(format!("ch{}", i), vec![format!("msg-{}", i)]);
    }
    assert!(cache.get("ch1").is_none());
    assert!(cache.get("ch2").is_some());

    cache.put("ch14".to_string(), vec!["msg-14".to_string()]);
    assert!(cache.get("ch2").is_some());
    assert!(cache.get("ch3").is_none());
}
