use std::{
    process::{Child, Command},
    time::Duration,
};

use futures::{stream, StreamExt};
use http::header::{COOKIE, SET_COOKIE};
use reqwest::Client;

const PARALLEL_REQUESTS: usize = 20;
const TOTAL_REQUESTS: usize = 10_000;
const URL: &str = "http://localhost:3000";

struct ChildGuard {
    child: Child,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.child.kill().expect("Failed to kill example binary");
        self.child
            .wait()
            .expect("Failed to wait for example binary to exit");
    }
}

fn start_example_binary() -> ChildGuard {
    let child = Command::new("cargo")
        .arg("run")
        .arg("--example")
        .arg("counter-concurrent")
        .spawn()
        .expect("Failed to start example binary");

    std::thread::sleep(Duration::from_secs(2)); // Wait for the example binary to initialize.

    ChildGuard { child }
}

#[tokio::test]
async fn concurrent_counter() {
    let _child_guard = start_example_binary();

    let urls = vec![URL; TOTAL_REQUESTS];
    let client = Client::new();

    let resp = client.get(URL).send().await.unwrap();
    let session_cookie = resp.headers().get(SET_COOKIE).unwrap();

    let bodies = stream::iter(urls)
        .map(|url| {
            let client = client.clone();
            let session_cookie = session_cookie.clone();
            tokio::spawn(async move {
                let resp = client
                    .get(url)
                    .header(COOKIE, session_cookie)
                    .send()
                    .await?;
                resp.bytes().await
            })
        })
        .buffer_unordered(PARALLEL_REQUESTS);

    let sum = bodies
        .fold(0, |mut acc, b| async move {
            match b {
                Ok(Ok(b)) => {
                    acc += std::str::from_utf8(&b[..])
                        .unwrap()
                        .parse::<usize>()
                        .unwrap();
                    acc
                }
                Ok(Err(e)) => panic!("reqwest::Error: {}", e),
                Err(e) => panic!("tokio::JoinError: {}", e),
            }
        })
        .await;

    // Sum = (n/2) * [2a + (n-1)d].
    assert_eq!(sum, 50_005_000);
}
