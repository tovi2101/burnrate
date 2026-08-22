use burnrate_lib::live::{
    fetch_cursor_with_cookie, fetch_opencode_with_credentials, parse_cursor_usage_response,
    parse_opencode_usage_response, LiveError,
};
use burnrate_lib::models::ProviderId;
use burnrate_lib::profiles;
use reqwest::Client;
use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

fn fixture(path: &str) -> Value {
    serde_json::from_str(match path {
        "cursor" => include_str!("../../fixtures/cursor-usage.json"),
        "opencode" => include_str!("../../fixtures/opencode-usage.json"),
        _ => "{}",
    })
    .expect("fixture JSON is valid")
}

fn rejected_endpoint() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test endpoint");
    let address = listener.local_addr().expect("read test endpoint address");
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut request = [0_u8; 1024];
        let _ = stream.read(&mut request);
        let response =
            b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let _ = stream.write_all(response);
    });
    format!("http://{address}/usage")
}

#[test]
fn cursor_fixture_maps_to_monthly_usage_window() {
    let snapshot = parse_cursor_usage_response("Fixture", &fixture("cursor"))
        .expect("Cursor fixture should parse");
    assert_eq!(snapshot.provider, ProviderId::Cursor);
    assert_eq!(snapshot.profile_name, "Fixture");
    assert_eq!(snapshot.plan_name.as_deref(), Some("pro"));
    assert_eq!(snapshot.windows.len(), 1);
    assert_eq!(snapshot.windows[0].label, "Monthly");
    assert!((snapshot.windows[0].used_pct - 42.5).abs() < f64::EPSILON);
    assert!(snapshot.windows[0].resets_at.is_some());
}

#[test]
fn opencode_fixture_maps_to_rolling_and_weekly_windows() {
    let snapshot = parse_opencode_usage_response("Fixture", &fixture("opencode"))
        .expect("OpenCode fixture should parse");
    assert_eq!(snapshot.provider, ProviderId::Opencode);
    assert_eq!(snapshot.plan_name.as_deref(), Some("OpenCode Go"));
    assert_eq!(snapshot.windows.len(), 2);
    assert_eq!(snapshot.windows[0].label, "5h");
    assert!((snapshot.windows[0].used_pct - 31.5).abs() < f64::EPSILON);
    assert_eq!(snapshot.windows[1].label, "Weekly");
    assert!((snapshot.windows[1].used_pct - 12.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn manual_cookie_roundtrip_rejected_credentials_are_clean_errors() {
    let cursor_cookie = "burnrate-test-cursor-dummy";
    profiles::save_manual(&ProviderId::Cursor, cursor_cookie).expect("save Cursor dummy");
    assert_eq!(
        profiles::manual_value(&ProviderId::Cursor).as_deref(),
        Some(cursor_cookie)
    );
    let client = Client::builder()
        .no_proxy()
        .build()
        .expect("build test client");
    let cursor_result = fetch_cursor_with_cookie(
        "Personal",
        &client,
        profiles::manual_value(&ProviderId::Cursor).expect("read Cursor dummy"),
        &rejected_endpoint(),
    )
    .await;
    assert!(matches!(cursor_result, Err(LiveError::Request)));
    profiles::delete_manual(&ProviderId::Cursor).expect("remove Cursor dummy");

    let opencode_cookie = "burnrate-test-opencode-dummy";
    profiles::save_manual(&ProviderId::Opencode, opencode_cookie).expect("save OpenCode dummy");
    assert_eq!(
        profiles::manual_value(&ProviderId::Opencode).as_deref(),
        Some(opencode_cookie)
    );
    let opencode_result = fetch_opencode_with_credentials(
        "Personal",
        &client,
        None,
        profiles::manual_value(&ProviderId::Opencode),
        &rejected_endpoint(),
    )
    .await;
    assert!(matches!(opencode_result, Err(LiveError::Request)));
    profiles::delete_manual(&ProviderId::Opencode).expect("remove OpenCode dummy");
}
