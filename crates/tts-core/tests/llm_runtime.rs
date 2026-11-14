//! Tests of the runtime of the language models.
//!
//! The language models have their own runtime and their own tenant in the
//! scheduler. The scheduler may take the memory of a model back when a bigger
//! one needs it, but it must not do that while the model is answering.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use ai_scheduler::{Scheduler, SchedulerConfig};
use common::TempDir;
use serde_json::json;
use tts_core::llm::LlmRuntimeManager;
use tts_core::paths::AppPaths;
use tts_core::service::TtsService;
use tts_core::types::RuntimeState;

/// The model the tests load.
const MODEL: &str = "unsloth--Qwen3.8-27B-GGUF";

/// The fake server that stands in for `llama-server`.
///
/// The real one needs a checkpoint of many gigabytes. The fake one answers
/// with a fixed sentence and takes its time, so that the test can look at the
/// scheduler while the request is in flight.
const FAKE_SERVER: &str = r#"#!/usr/bin/env python3
import json
import sys
import time
from http.server import BaseHTTPRequestHandler, HTTPServer

port = 0
args = sys.argv[1:]
for index, arg in enumerate(args):
    if arg == "--port":
        port = int(args[index + 1])

# The answer takes a moment, so that the test finds the request in flight.
DELAY = 1.0


class Handler(BaseHTTPRequestHandler):
    def _send(self, payload):
        body = json.dumps(payload).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        self._send({"status": "ok"})

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        self.rfile.read(length)
        time.sleep(DELAY)
        self._send({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "ok"},
                "finish_reason": "stop",
            }],
        })

    def log_message(self, *args):
        pass


HTTPServer(("127.0.0.1", port), Handler).serve_forever()
"#;

/// Writes the fake server and makes it runnable.
fn write_fake_engine(root: &Path) -> PathBuf {
    let directory = root.join("fake-engine");
    std::fs::create_dir_all(&directory).expect("failed to create the directory");
    let path = directory.join("llama-server");
    std::fs::write(&path, FAKE_SERVER).expect("failed to write the server");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&path)
            .expect("the file is there")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("failed to make it runnable");
    }
    path
}

/// Writes the manifest that points the language runtime at the fake server.
fn write_manifest(paths: &AppPaths, entry: &Path) {
    let manifest = json!({
        "engines": {
            "llama.cpp": {
                "current": "test",
                "installed": [{
                    "version": "test",
                    "path": entry.parent().expect("the server has a directory"),
                    "entry": entry,
                    "installed_at": "2026-01-01T00:00:00Z",
                    "size_bytes": 0,
                    "source": "test",
                }],
            },
        },
    });
    let root = paths.engines_root();
    std::fs::create_dir_all(&root).expect("failed to create the directory");
    std::fs::write(root.join("engines.json"), manifest.to_string())
        .expect("failed to write the manifest");
}

/// Writes a checkpoint of the language runtime, so that the catalogue adopts
/// the model.
fn write_checkpoint(root: &Path, id: &str) {
    let directory = root.join("models/llm").join(id);
    std::fs::create_dir_all(&directory).expect("failed to create the directory");
    std::fs::write(directory.join("model-Q4_K_M.gguf"), [0_u8; 256])
        .expect("failed to write the weights");
}

/// The runtime of the language models, with a scheduler that lets go of a
/// model as soon as it is idle.
struct Fixture {
    manager: std::sync::Arc<LlmRuntimeManager>,
    scheduler: std::sync::Arc<Scheduler>,
    _dir: TempDir,
}

impl Fixture {
    /// Builds the fixture. It is `None` when the machine has no python3 for
    /// the fake server.
    fn new(prefix: &str) -> Option<Self> {
        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("the test needs python3 for the fake server");
            return None;
        }
        let dir = TempDir::new(prefix);
        let paths = AppPaths::with_root(dir.path());
        paths.ensure().expect("the directories are there");
        write_checkpoint(dir.path(), MODEL);
        let server = write_fake_engine(dir.path());
        write_manifest(&paths, &server);

        let service = TtsService::with_paths(paths.clone()).expect("the service");
        // The idle time of a model is zero, so the scheduler would take it
        // away right away. Only a model that is answering is protected.
        let scheduler = Scheduler::new(SchedulerConfig {
            total_bytes: 8 * 1024 * 1024 * 1024,
            reserved_bytes: 0,
            default_ttl: Duration::from_secs(0),
            ..Default::default()
        });
        let manager = LlmRuntimeManager::new(
            service.registry.clone(),
            scheduler.clone(),
            service.engines.clone(),
            paths.llm_logs_root(),
        );
        Some(Self {
            manager,
            scheduler,
            _dir: dir,
        })
    }

    /// The key of the tenant of the model in the scheduler.
    fn tenant_key(&self) -> String {
        format!("llm:{MODEL}")
    }
}

/// A model that is answering a request is not taken away by the scheduler.
///
/// The scheduler may take the memory of an idle model back. A model that is
/// in the middle of a request is not idle, and the request must survive.
#[tokio::test]
async fn a_language_model_that_answers_is_not_evicted() {
    let Some(fixture) = Fixture::new("llm-runtime") else {
        return;
    };
    let manager = fixture.manager.clone();
    let scheduler = fixture.scheduler.clone();
    let key = fixture.tenant_key();

    let runtime = manager.load_model(MODEL).await.expect("the model starts");
    assert_eq!(runtime.state, RuntimeState::Running);
    let loaded = scheduler
        .tenant(&fixture.tenant_key())
        .expect("the tenant is known");

    // The request goes to the engine.
    let chat = {
        let manager = manager.clone();
        tokio::spawn(async move {
            manager
                .chat(MODEL, json!({"messages": [{"role": "user", "content": "hi"}]}))
                .await
        })
    };

    // Wait until the request reached the scheduler.
    let mut registered = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let tenant = scheduler.tenant(&key).expect("the tenant is known");
        if tenant.active_requests > 0 {
            registered = true;
            break;
        }
    }
    assert!(registered, "the request is registered with the scheduler");

    // The sweeper runs. A model that is answering must survive it.
    let evicted = scheduler
        .sweep_expired()
        .await
        .expect("the sweep runs");
    assert!(
        evicted.is_empty(),
        "a model that is answering is not taken away, but {evicted:?} was"
    );

    // The answer arrives.
    let answer = chat
        .await
        .expect("the task did not panic")
        .expect("the model answers");
    assert_eq!(answer["choices"][0]["message"]["content"], "ok");

    // The idle time counts from the last use, not from the moment the model
    // was loaded.
    let after = scheduler.tenant(&key).expect("the tenant is known");
    assert_eq!(after.active_requests, 0, "the request is over");
    assert!(
        after.last_used_at > loaded.last_used_at,
        "the idle timer follows the requests"
    );

    manager.unload_model(MODEL).await.ok();
}

/// A pinned language model stays in memory, even when it is idle.
#[tokio::test]
async fn a_language_model_can_be_pinned() {
    let Some(fixture) = Fixture::new("llm-pin") else {
        return;
    };
    let manager = fixture.manager.clone();
    let scheduler = fixture.scheduler.clone();
    let key = fixture.tenant_key();

    manager.load_model(MODEL).await.expect("the model starts");

    // Without the pin the scheduler takes the idle model away.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let evicted = scheduler
        .sweep_expired()
        .await
        .expect("the sweep runs");
    assert_eq!(evicted, vec![key.clone()], "an idle model is taken away");

    // The model is loaded again and pinned.
    manager.load_model(MODEL).await.expect("the model starts");
    manager.set_pinned(MODEL, true).expect("the model is pinned");
    assert!(
        manager
            .runtime(MODEL)
            .expect("the runtime is known")
            .pinned,
        "the runtime knows that the model is pinned"
    );

    let evicted = scheduler
        .sweep_expired()
        .await
        .expect("the sweep runs");
    assert!(evicted.is_empty(), "a pinned model stays in memory");

    manager.unload_model(MODEL).await.ok();
}
