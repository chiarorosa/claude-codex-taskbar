use super::*;

#[test]
fn notifications_default_to_opt_in_with_two_thresholds() {
    let cfg = Config::default();
    assert!(!cfg.notify_near_limit);
    assert_eq!(cfg.notify_thresholds, [80, 95]);
}

#[test]
fn old_config_keeps_settings_and_gets_notification_defaults() {
    let cfg: Config = serde_json::from_str(
        r#"{"claude_enabled":true,"limits_interval_secs":300,"hidden_items":["gpu"]}"#,
    )
    .unwrap();
    assert!(cfg.claude_enabled);
    assert_eq!(cfg.limits_interval_secs, 300);
    assert_eq!(cfg.hidden_items, ["gpu"]);
    assert!(!cfg.notify_near_limit);
    assert_eq!(cfg.notify_thresholds, [80, 95]);
}

#[test]
fn thresholds_are_clamped_sorted_and_deduplicated() {
    for (input, expected) in [
        (vec![95, 80], vec![80, 95]),
        (vec![0, 101, u32::MAX, 1, 100], vec![1, 100]),
        (vec![95, 80, 80], vec![80, 95]),
        (vec![1, 75, 100], vec![1, 75, 100]),
        (vec![], vec![80, 95]),
    ] {
        let cfg = Config {
            notify_near_limit: true,
            notify_thresholds: input,
            ..Config::default()
        }
        .clamped();
        assert_eq!(cfg.notify_thresholds, expected);
        assert!(cfg.notify_near_limit);
        assert_eq!(cfg.clone().clamped(), cfg);
    }
}

#[test]
fn load_normalizes_notification_thresholds() {
    let path = std::env::temp_dir().join(format!("cct-notify-{}.json", std::process::id()));
    std::fs::write(
        &path,
        r#"{"notify_near_limit":true,"notify_thresholds":[999,0,80,80]}"#,
    )
    .unwrap();
    let cfg = Config::load_from(&path);
    assert!(cfg.notify_near_limit);
    assert_eq!(cfg.notify_thresholds, [1, 80, 100]);
    cfg.save_to(&path).unwrap();
    assert_eq!(Config::load_from(&path), cfg);
    std::fs::remove_file(path).unwrap();
}
