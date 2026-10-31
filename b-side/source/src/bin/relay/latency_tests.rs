//! Loopback-only transport tests. No HAL, TEE, Binder or real relay endpoint.
use super::*;
use std::io::{BufRead, BufReader, Read};
use std::net::{TcpListener, TcpStream};

fn read_request(stream: &mut TcpStream) -> (String, Vec<u8>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut reader = BufReader::new(stream);
    let mut headers = String::new();
    let mut length = 0;
    loop {
        let mut line = String::new();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if line == "\r\n" {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            if key.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse::<usize>().unwrap();
            }
        }
        headers.push_str(&line);
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    (headers, body)
}

fn respond(stream: &mut TcpStream, body: &str) {
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
}

#[test]
fn empty_poll_floor_applies_only_to_fast_empty_responses() {
    assert_eq!(empty_poll_delay(Duration::ZERO), Duration::from_millis(80));
    assert_eq!(
        empty_poll_delay(Duration::from_millis(35)),
        Duration::from_millis(45)
    );
    assert_eq!(empty_poll_delay(Duration::from_secs(20)), Duration::ZERO);
    assert_eq!(POLL_TIMEOUT_SEC, 20);
    assert_eq!(POLL_WALL_LIMIT, Duration::from_secs(25));
}

#[test]
fn held_poll_does_not_block_result_upload_on_shared_client() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let cfg = RelayConfig {
        server: format!("http://{}", listener.local_addr().unwrap()),
        device_id: "software-test".into(),
        machine_id: "mock".into(),
        token: "test-only".into(),
        soter_enabled: false,
        soter_allow_mutation: false,
        soter_probe: None,
        wakelock: false,
        bind_iface: "none".into(),
    };
    set_uplink_config(&cfg);
    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut poll_stream, _) = listener.accept().unwrap();
        let (headers, _) = read_request(&mut poll_stream);
        assert!(headers.starts_with("GET /api/b/poll/?"));
        assert!(headers.contains("timeout=20"));
        assert!(headers
            .to_ascii_lowercase()
            .contains("x-relay-token: test-only"));
        ready_tx.send(()).unwrap();
        // Wait for the bounded upload call to finish; then release the held poll.
        // A broken shared-client lock fails in <=2s rather than waiting 25s.
        listener.set_nonblocking(true).unwrap();
        loop {
            match listener.accept() {
                Ok((mut upload_stream, _)) => {
                    let (headers, body) = read_request(&mut upload_stream);
                    assert!(headers.starts_with("POST /api/b/result/"));
                    assert_eq!(
                        serde_json::from_slice::<Value>(&body).unwrap()["task_id"],
                        "mock-task"
                    );
                    respond(&mut upload_stream, "{\"status\":\"ok\"}");
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if release_rx.recv_timeout(Duration::from_millis(5)).is_ok() {
                        break;
                    }
                }
                Err(error) => panic!("accept: {error}"),
            }
        }
        respond(
            &mut poll_stream,
            "{\"task_id\":\"mock-task\",\"task_type\":\"attest\",\"payload\":{}}",
        );
        listener.set_nonblocking(false).unwrap();
        let (mut empty_stream, _) = listener.accept().unwrap();
        let (headers, _) = read_request(&mut empty_stream);
        assert!(headers.contains("timeout=20"));
        // Longer than the former 3s short-poll deadline; the production helper
        // must accept a held 204 instead of resetting its connection pool.
        thread::sleep(Duration::from_millis(3200));
        write!(
            empty_stream,
            "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
    });
    let poll_cfg = cfg.clone();
    let poll = thread::spawn(move || poll_tasks(&poll_cfg, &mut BatteryTelemetry::default()));
    ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let result = http_request_with_timeout(
        "POST",
        &format!("{}/api/b/result/", cfg.server),
        &[("Content-Type".into(), "application/json".into())],
        Some(br#"{"task_id":"mock-task","device_id":"software-test","result":{}}"#),
        Some(Duration::from_secs(2)),
    );
    let _ = release_tx.send(());
    let task = poll
        .join()
        .unwrap()
        .unwrap_or_else(|error| panic!("poll: {:#}", error.error))
        .unwrap();
    assert_eq!(task.0, "mock-task");
    assert_eq!(result.unwrap().status, 200);
    assert!(poll_tasks(&cfg, &mut BatteryTelemetry::default())
        .unwrap_or_else(|error| panic!("held poll: {:#}", error.error))
        .is_none());
    server.join().unwrap();
    assert_eq!(reported_caps(&cfg), "");
    reset_http_client();
}
