use forksync::github::Github;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

#[derive(Clone)]
struct MockResponse {
    status: u16,
    body: String,
    headers: Vec<(String, String)>,
}

struct MockServer {
    base_url: String,
    task: tokio::task::JoinHandle<()>,
}

impl MockServer {
    async fn start(responses: Vec<MockResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let queue = Arc::new(Mutex::new(VecDeque::from(responses)));
        let queue_clone = queue.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let queue = queue_clone.clone();
                tokio::spawn(async move {
                    let mut data = Vec::new();
                    let mut buf = [0_u8; 1024];
                    loop {
                        match socket.read(&mut buf).await {
                            Ok(0) => break,
                            Ok(n) => {
                                data.extend_from_slice(&buf[..n]);
                                if data.windows(4).any(|window| window == b"\r\n\r\n") {
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }

                    let response = queue.lock().await.pop_front().unwrap_or(MockResponse {
                        status: 404,
                        body: "{}".to_owned(),
                        headers: Vec::new(),
                    });
                    let status_text = match response.status {
                        200 => "OK",
                        201 => "Created",
                        404 => "Not Found",
                        409 => "Conflict",
                        429 => "Too Many Requests",
                        500 => "Internal Server Error",
                        _ => "OK",
                    };
                    let mut headers = response.headers.clone();
                    headers.push(("Content-Type".to_owned(), "application/json".to_owned()));
                    headers.push(("Content-Length".to_owned(), response.body.len().to_string()));
                    headers.push(("Connection".to_owned(), "close".to_owned()));
                    let mut head = format!("HTTP/1.1 {} {}\r\n", response.status, status_text);
                    for (name, value) in headers {
                        head.push_str(&format!("{name}: {value}\r\n"));
                    }
                    head.push_str("\r\n");
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(response.body.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        Self {
            base_url: format!("http://{}", addr),
            task,
        }
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn merge_upstream_retries_rate_limit() {
    let server = MockServer::start(vec![
        MockResponse {
            status: 429,
            body: r#"{"message":"rate limited"}"#.to_owned(),
            headers: vec![("Retry-After".to_owned(), "0".to_owned())],
        },
        MockResponse {
            status: 200,
            body: r#"{"message":"Syncing..."}"#.to_owned(),
            headers: Vec::new(),
        },
    ])
    .await;

    let client = Github::new_with_base("token", server.base_url.clone()).unwrap();
    let repo = forksync::github::Repo {
        full_name: "owner/repo".to_owned(),
        default_branch: "main".to_owned(),
        fork: true,
        archived: false,
        disabled: false,
    };

    let result = client.merge_upstream(&repo).await.unwrap();
    assert!(matches!(result, forksync::github::Merge::Updated));
}

#[tokio::test]
async fn merge_upstream_reports_conflict() {
    let server = MockServer::start(vec![MockResponse {
        status: 409,
        body: r#"{"message":"Merge conflict"}"#.to_owned(),
        headers: Vec::new(),
    }])
    .await;

    let client = Github::new_with_base("token", server.base_url.clone()).unwrap();
    let repo = forksync::github::Repo {
        full_name: "owner/repo".to_owned(),
        default_branch: "main".to_owned(),
        fork: true,
        archived: false,
        disabled: false,
    };

    let error = client.merge_upstream(&repo).await.unwrap_err().to_string();
    assert!(error.contains("409"));
    assert!(error.contains("Merge conflict"));
}

#[tokio::test]
async fn forks_paginates() {
    let mut first_page = String::from("[");
    for i in 0..100 {
        if i > 0 {
            first_page.push(',');
        }
        first_page.push_str(&format!(
            r#"{{"full_name":"owner/repo-{i}","default_branch":"main","fork":true,"archived":false,"disabled":false}}"#
        ));
    }
    first_page.push(']');

    let server = MockServer::start(vec![
        MockResponse {
            status: 200,
            body: first_page,
            headers: Vec::new(),
        },
        MockResponse {
            status: 200,
            body: r#"[{"full_name":"owner/repo-100","default_branch":"main","fork":true,"archived":false,"disabled":false}]"#.to_owned(),
            headers: Vec::new(),
        },
    ])
    .await;

    let client = Github::new_with_base("token", server.base_url.clone()).unwrap();
    let repos = client.forks("owner", "owner").await.unwrap();
    assert_eq!(repos.len(), 101);
}
