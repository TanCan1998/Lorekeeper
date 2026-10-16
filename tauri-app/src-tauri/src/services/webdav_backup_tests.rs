use super::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{atomic::AtomicUsize, Mutex};
use std::thread::JoinHandle;

const ORIGIN: &str = "123e4567-e89b-42d3-a456-426614174000";
const GEN1: &str = "11111111-1111-4111-8111-111111111111";
const GEN2: &str = "22222222-2222-4222-8222-222222222222";
const CREATED: &str = "2026-10-16T10:00:00.000Z";

struct Server {
    url: String,
    objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    put_order: Arc<Mutex<Vec<String>>>,
    stop: CancelFlag,
    fail_after_put: Arc<AtomicBool>,
    bad_auth: Arc<AtomicBool>,
    requests: Arc<AtomicUsize>,
    thread: Option<JoinHandle<()>>,
}
impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let objects = Arc::new(Mutex::new(BTreeMap::<String, Vec<u8>>::new()));
        let put_order = Arc::new(Mutex::new(Vec::new()));
        let stop = cancellation();
        let fail_after_put = Arc::new(AtomicBool::new(false));
        let bad_auth = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(AtomicUsize::new(0));
        let (map, order, done, uncertain, auth, counter) = (
            objects.clone(),
            put_order.clone(),
            stop.clone(),
            fail_after_put.clone(),
            bad_auth.clone(),
            requests.clone(),
        );
        let thread = std::thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        socket.set_nodelay(true).unwrap();
                        socket
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        let mut reader = BufReader::new(socket.try_clone().unwrap());
                        let mut first = String::new();
                        if reader.read_line(&mut first).is_err() {
                            continue;
                        }
                        let parts: Vec<_> = first.split_whitespace().collect();
                        if parts.len() < 2 {
                            continue;
                        }
                        let (method, path) = (parts[0], parts[1]);
                        let mut headers = BTreeMap::new();
                        loop {
                            let mut line = String::new();
                            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                                break;
                            }
                            if let Some((key, value)) = line.split_once(':') {
                                headers.insert(key.to_ascii_lowercase(), value.trim().to_string());
                            }
                        }
                        let size = headers
                            .get("content-length")
                            .and_then(|s| s.parse::<usize>().ok())
                            .unwrap_or(0);
                        if size > 4 * 1024 * 1024 {
                            continue;
                        }
                        let mut body = vec![0; size];
                        if reader.read_exact(&mut body).is_err() {
                            continue;
                        }
                        counter.fetch_add(1, Ordering::SeqCst);
                        if auth.load(Ordering::SeqCst) {
                            respond(&mut socket, 401, b"", None);
                            continue;
                        }
                        if headers.get("authorization").map(String::as_str) != Some("Basic dTpz") {
                            respond(&mut socket, 401, b"", None);
                            continue;
                        }
                        match method {
                            "MKCOL" => respond(&mut socket, 201, b"", None),
                            "PUT" => {
                                let mut objects = map.lock().unwrap();
                                if headers.get("if-none-match").map(String::as_str) != Some("*")
                                    || objects.contains_key(path)
                                {
                                    respond(&mut socket, 412, b"", None);
                                } else {
                                    objects.insert(path.into(), body);
                                    order.lock().unwrap().push(path.into());
                                    respond(
                                        &mut socket,
                                        if uncertain.swap(false, Ordering::SeqCst) {
                                            500
                                        } else {
                                            201
                                        },
                                        b"",
                                        None,
                                    );
                                }
                            }
                            "GET" | "HEAD" => {
                                let objects = map.lock().unwrap();
                                match objects.get(path) {
                                    Some(bytes) => respond(
                                        &mut socket,
                                        200,
                                        if method == "HEAD" { b"" } else { bytes },
                                        if method == "HEAD" {
                                            Some(bytes.len())
                                        } else {
                                            None
                                        },
                                    ),
                                    None => respond(&mut socket, 404, b"", None),
                                }
                            }
                            "PROPFIND" => {
                                let mut paths = BTreeSet::new();
                                paths.insert(path.to_string());
                                if headers.get("depth").map(String::as_str) == Some("1") {
                                    for key in map.lock().unwrap().keys() {
                                        if let Some(relative) = key.strip_prefix(path) {
                                            if let Some((id, _)) = relative.split_once('/') {
                                                paths.insert(format!("{path}{id}/"));
                                            }
                                        }
                                    }
                                }
                                let xml = format!(
                                    "<d:multistatus xmlns:d=\"DAV:\">{}</d:multistatus>",
                                    paths
                                        .iter()
                                        .map(|p| format!(
                                            "<d:response><d:href>{p}</d:href></d:response>"
                                        ))
                                        .collect::<String>()
                                );
                                respond(&mut socket, 207, xml.as_bytes(), None);
                            }
                            _ => respond(&mut socket, 405, b"", None),
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2))
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url: format!("http://{addr}/dav/"),
            objects,
            put_order,
            stop,
            fail_after_put,
            bad_auth,
            requests,
            thread: Some(thread),
        }
    }
    fn account(&self) -> WebDavAccount {
        WebDavAccount {
            endpoint: self.url.clone(),
            username: "u".into(),
            secret: "s".into(),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}
fn respond(socket: &mut TcpStream, status: u16, body: &[u8], length: Option<usize>) {
    let header = format!(
        "HTTP/1.1 {status} fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        length.unwrap_or(body.len())
    );
    let mut packet = header.into_bytes();
    packet.extend_from_slice(body);
    let _ = socket.write_all(&packet);
    let _ = socket.shutdown(std::net::Shutdown::Write);
}
fn root() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "lorekeeper-webdav-{}",
        crate::project_access::random_uuid_v4()
    ));
    std::fs::create_dir(&root).unwrap();
    std::fs::canonicalize(root).unwrap()
}
async fn append(
    service: &WebDavService,
    server: &Server,
    archive: &Path,
    id: &str,
    parents: &[String],
) -> Result<WebDavGeneration, String> {
    service
        .append(AppendInput {
            account: &server.account(),
            cloud_book_id: "book",
            archive,
            origin_project_id: ORIGIN,
            snapshot_generation: "snapshot",
            parents,
            generation_id: id,
            created_at: CREATED,
            cancel: &cancellation(),
        })
        .await
}
#[tokio::test]
async fn append_list_download_are_verified_and_completion_is_last() {
    let server = Server::start();
    let service = WebDavService::new().unwrap();
    let account = server.account();
    let cancel = cancellation();
    service.check_connection(&account, &cancel).await.unwrap();
    let root = root();
    let source = root.join("source");
    std::fs::write(&source, b"unit-archive").unwrap();
    server.fail_after_put.store(true, Ordering::SeqCst);
    append(
        &service,
        &server,
        &source,
        GEN1,
        &["parent-A".into(), "parent-B".into()],
    )
    .await
    .unwrap();
    let order = server.put_order.lock().unwrap().clone();
    assert!(order[0].ends_with("archive.ainovel"));
    assert!(order[1].ends_with("manifest.json"));
    assert!(order[2].ends_with("completion.json"));
    append(
        &service,
        &server,
        &source,
        GEN2,
        &["parent-B".into(), "parent-A".into()],
    )
    .await
    .unwrap();
    let generations = service.list(&account, "book", &cancel).await.unwrap();
    assert_eq!(generations.len(), 2);
    assert!(generations
        .iter()
        .all(|g| g.has_sibling && g.sibling_generation_ids.len() == 1));
    let target = root.join("download");
    service
        .download(&account, "book", GEN1, &target, &cancel)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"unit-archive");
    assert_eq!(
        service
            .download(&account, "book", GEN1, &target, &cancel)
            .await
            .unwrap_err(),
        "WEBDAV_TARGET_EXISTS"
    );
    // 内容已经提交但随后被服务器改写，列表不可把它当作完整备份。
    server.objects.lock().unwrap().insert(
        format!("/dav/books/book/generations/{GEN1}/archive.ainovel"),
        b"tampered".to_vec(),
    );
    assert_eq!(
        service.list(&account, "book", &cancel).await.unwrap().len(),
        1
    );
    let failed = root.join("failed");
    assert!(service
        .download(&account, "book", GEN1, &failed, &cancel)
        .await
        .is_err());
    assert!(!failed.exists());
    assert!(!std::fs::read_dir(&root).unwrap().any(|p| p
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".portable-project-attempt-")));
    let _ = std::fs::remove_dir_all(root);
}
#[tokio::test]
async fn conflicting_remote_object_and_cancel_do_not_publish_completion() {
    let server = Server::start();
    let service = WebDavService::new().unwrap();
    let root = root();
    let source = root.join("source");
    std::fs::write(&source, b"unit-archive").unwrap();
    server.objects.lock().unwrap().insert(
        format!("/dav/books/book/generations/{GEN1}/archive.ainovel"),
        b"other".to_vec(),
    );
    assert!(append(&service, &server, &source, GEN1, &[]).await.is_err());
    assert!(!server
        .objects
        .lock()
        .unwrap()
        .keys()
        .any(|p| p.ends_with("completion.json")));
    assert!(service
        .list(&server.account(), "book", &cancellation())
        .await
        .unwrap()
        .is_empty());
    let flag = cancellation();
    flag.store(true, Ordering::SeqCst);
    let count = server.requests.load(Ordering::SeqCst);
    assert_eq!(
        service
            .check_connection(&server.account(), &flag)
            .await
            .unwrap_err(),
        "WEBDAV_CANCELLED"
    );
    assert_eq!(server.requests.load(Ordering::SeqCst), count);
    server.bad_auth.store(true, Ordering::SeqCst);
    assert_eq!(
        service
            .check_connection(&server.account(), &cancellation())
            .await
            .unwrap_err(),
        "WEBDAV_AUTH"
    );
    let _ = std::fs::remove_dir_all(root);
}

fn one_shot(response: Vec<u8>, delay: Duration) -> (String, CancelFlag, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/dav/", listener.local_addr().unwrap());
    let started = cancellation();
    let flag = started.clone();
    let thread = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(socket.try_clone().unwrap());
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                break;
            }
        }
        flag.store(true, Ordering::SeqCst);
        std::thread::sleep(delay);
        let _ = socket.write_all(&response);
        let _ = socket.shutdown(std::net::Shutdown::Write);
    });
    (url, started, thread)
}
#[tokio::test]
async fn redirects_are_not_followed_and_inflight_cancel_interrupts_network_wait() {
    let target = Server::start();
    let response = format!(
        "HTTP/1.1 302 fixture\r\nLocation: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        target.url
    )
    .into_bytes();
    let (endpoint, _, thread) = one_shot(response, Duration::ZERO);
    let service = WebDavService::new().unwrap();
    let account = WebDavAccount {
        endpoint,
        username: "u".into(),
        secret: "s".into(),
    };
    assert_eq!(
        service
            .check_connection(&account, &cancellation())
            .await
            .unwrap_err(),
        "WEBDAV_NETWORK"
    );
    assert_eq!(target.requests.load(Ordering::SeqCst), 0);
    thread.join().unwrap();
    let response =
        b"HTTP/1.1 207 fixture\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
    let (endpoint, started, thread) = one_shot(response, Duration::from_secs(2));
    let flag = cancellation();
    let worker_flag = flag.clone();
    let worker = tokio::spawn(async move {
        service
            .check_connection(
                &WebDavAccount {
                    endpoint,
                    username: "u".into(),
                    secret: "s".into(),
                },
                &worker_flag,
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while !started.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    flag.store(true, Ordering::SeqCst);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err(),
        "WEBDAV_CANCELLED"
    );
    thread.join().unwrap();
}
#[tokio::test]
async fn declared_response_size_is_checked_before_allocating_body() {
    let response =
        b"HTTP/1.1 207 fixture\r\nContent-Length: 1048577\r\nConnection: close\r\n\r\n".to_vec();
    let (endpoint, _, thread) = one_shot(response, Duration::ZERO);
    let account = WebDavAccount {
        endpoint,
        username: "u".into(),
        secret: "s".into(),
    };
    let result = WebDavService::new()
        .unwrap()
        .check_connection(&account, &cancellation())
        .await;
    assert_eq!(result.unwrap_err(), "WEBDAV_RESPONSE_TOO_LARGE");
    thread.join().unwrap();
}

#[test]
fn dav_hrefs_reject_cross_origin_encoded_escape_entities_and_shape_drift() {
    let url = Url::parse("https://example.test/dav/books/book/generations/").unwrap();
    let wrap = |href: &str| format!("<d:response><d:href>{href}</d:href></d:response>");
    assert_eq!(
        generation_hrefs(&wrap(&format!("{GEN1}/")), &url).unwrap(),
        vec![GEN1]
    );
    for href in [
        "https://attacker.test/x",
        "//attacker.test/x",
        "../escape",
        "%2E%2E/escape",
        "%zz",
        "&evil;",
        "../../",
        "https://u:p@example.test/x",
    ] {
        assert!(generation_hrefs(&wrap(href), &url).is_err(), "{href}");
    }
    assert!(generation_hrefs(&format!("<!DOCTYPE x>{}", wrap(&format!("{GEN1}/"))), &url).is_err());
    assert!(generation_hrefs(
        "<d:response><d:href>x</d:href><d:href>y</d:href></d:response>",
        &url
    )
    .is_err());
    assert!(generation_hrefs("<d:response><d:href>x</d:href>", &url).is_err());
}
