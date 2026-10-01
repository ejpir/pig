use super::*;
use gpui::TestAppContext;
use serde_json::json;
use std::{path::PathBuf, time::Duration};

pub(super) async fn ask(
    checker: &Entity<Checker>,
    request: serde_json::Value,
    cx: &mut TestAppContext,
) -> String {
    let request = serde_json::from_value(request).unwrap();
    checker
        .update(cx, |c, cx| c.handle(request, cx))
        .await
        .unwrap()
}

#[gpui::test]
async fn nothing_is_reported_without_language_services(cx: &mut TestAppContext) {
    let checker = cx.new(|_| Checker::new(|_| None));
    let text = ask(&checker, json!({"op": "file", "path": "/dir/a.rs"}), cx).await;
    assert_eq!(text, "");
}

/// Sends one line the way the extension does and returns the parsed reply.
fn request(bridge: &Bridge, line: serde_json::Value, cx: &mut TestAppContext) -> serde_json::Value {
    use std::io::{BufRead as _, BufReader, Read, Write};
    let env = |name| {
        bridge
            .env()
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.clone())
    };
    let address = env(ADDRESS_ENV).unwrap().into_string().unwrap();
    let mut line = line;
    if let Some(token) = env(TOKEN_ENV) {
        line.as_object_mut()
            .unwrap()
            .entry("token")
            .or_insert(token.into_string().unwrap().into());
    }
    let client = std::thread::spawn(move || {
        let mut stream: Box<dyn ReadWrite> = match address.strip_prefix("tcp:") {
            Some(port) => Box::new(
                std::net::TcpStream::connect(("127.0.0.1", port.parse::<u16>().unwrap())).unwrap(),
            ),
            #[cfg(unix)]
            None => Box::new(std::os::unix::net::UnixStream::connect(address).unwrap()),
            #[cfg(not(unix))]
            None => unreachable!("sockets are Unix-only"),
        };
        stream.write_all(format!("{line}\n").as_bytes()).unwrap();
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).unwrap();
        reply
    });
    trait ReadWrite: Read + Write + Send {}
    impl<T: Read + Write + Send> ReadWrite for T {}
    while !client.is_finished() {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(5));
    }
    serde_json::from_str(&client.join().unwrap()).unwrap()
}

/// A session handler that answers `echo` and refuses anything else.
fn echo() -> Handler {
    Rc::new(|request, _| match request["op"].as_str() {
        Some("echo") => Task::ready(Ok(request["text"].as_str().unwrap_or("").to_owned())),
        _ => Task::ready(Err(anyhow::anyhow!("Unknown request"))),
    })
}

fn transports() -> Vec<Transport> {
    #[cfg(unix)]
    return vec![Transport::Unix, Transport::Tcp];
    #[cfg(not(unix))]
    return vec![Transport::Tcp];
}

#[gpui::test]
fn each_transport_answers_one_json_line_per_connection(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    for transport in transports() {
        let bridge = cx
            .update(|cx| Bridge::start_with(|_| None, |_, _| true, echo(), transport, cx))
            .unwrap();
        assert_eq!(
            request(&bridge, json!({"op": "run_end"}), cx),
            json!({"text": ""}),
            "{transport:?}"
        );
        let unknown = request(&bridge, json!({"op": "format_disk"}), cx);
        assert!(unknown["error"].is_string(), "{transport:?}");
        assert_eq!(
            request(&bridge, json!({"op": "echo", "text": "the session's"}), cx),
            json!({"text": "the session's"}),
            "other requests go to the session"
        );

        let address = PathBuf::from(&bridge.env()[0].1);
        drop(bridge);
        assert!(
            !address.exists(),
            "closing the session removes a socket file"
        );
    }
}

#[gpui::test]
fn a_port_answers_only_requests_with_its_token(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let bridge = cx
        .update(|cx| Bridge::start_with(|_| None, |_, _| true, echo(), Transport::Tcp, cx))
        .unwrap();
    let wrong = request(&bridge, json!({"op": "run_end", "token": "guess"}), cx);
    assert_eq!(wrong, json!({"error": "Missing or wrong token"}));
    assert_eq!(
        request(&bridge, json!({"op": "run_end"}), cx),
        json!({"text": ""})
    );
}
