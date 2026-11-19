//! Integration tests of the global scheduler.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use ai_scheduler::{evictor_from_fn, Scheduler, SchedulerConfig, TenantKind, TenantState};

fn scheduler(total: u64) -> Arc<Scheduler> {
    Scheduler::new(SchedulerConfig {
        total_bytes: total,
        reserved_bytes: 0,
        default_ttl: Duration::from_secs(30),
        sweep_interval: Duration::from_secs(60),
        acquire_timeout: Duration::from_millis(500),
    })
}

/// Counter that records how often an eviction callback ran.
fn eviction_counter() -> (Arc<AtomicUsize>, ai_scheduler::Evictor) {
    let counter = Arc::new(AtomicUsize::new(0));
    let evictor = {
        let counter = counter.clone();
        evictor_from_fn(move |_key: String| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        })
    };
    (counter, evictor)
}

#[tokio::test]
async fn acquire_reserves_and_release_frees() {
    let scheduler = scheduler(1000);
    scheduler.register_tenant("llm:a", TenantKind::Llm, 400, None, None);
    let lease = scheduler.acquire("llm:a", 400, None).await.unwrap();
    assert_eq!(scheduler.snapshot().used_bytes, 400);
    assert_eq!(scheduler.snapshot().available_bytes, 600);
    lease.release();
    assert_eq!(scheduler.snapshot().used_bytes, 0);
}

#[tokio::test]
async fn dropping_the_lease_frees_the_memory() {
    let scheduler = scheduler(1000);
    scheduler.register_tenant("llm:a", TenantKind::Llm, 400, None, None);
    {
        let _lease = scheduler.acquire("llm:a", 400, None).await.unwrap();
        assert_eq!(scheduler.snapshot().used_bytes, 400);
    }
    assert_eq!(scheduler.snapshot().used_bytes, 0);
}

#[tokio::test]
async fn unknown_tenant_is_rejected() {
    let scheduler = scheduler(1000);
    let error = scheduler
        .acquire("llm:missing", 10, None)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ai_scheduler::SchedulerError::UnknownTenant(_)
    ));
}

#[tokio::test]
async fn insufficient_memory_is_reported() {
    let scheduler = scheduler(1000);
    scheduler.register_tenant("llm:a", TenantKind::Llm, 800, None, None);
    let _lease = scheduler.acquire("llm:a", 800, None).await.unwrap();
    // A second tenant does not fit and the first one cannot be evicted
    // because it has no eviction callback.
    scheduler.register_tenant("llm:b", TenantKind::Llm, 400, None, None);
    let error = scheduler.acquire("llm:b", 400, None).await.unwrap_err();
    assert!(matches!(
        error,
        ai_scheduler::SchedulerError::InsufficientMemory { .. }
    ));
}

#[tokio::test]
async fn idle_tenants_are_evicted_in_lru_order() {
    let scheduler = scheduler(1000);
    let (evicted, evictor) = eviction_counter();
    for key in ["llm:a", "llm:b"] {
        scheduler.register_tenant(key, TenantKind::Llm, 500, None, Some(evictor.clone()));
    }

    let _lease_a = scheduler.acquire("llm:a", 500, None).await.unwrap();
    // `llm:a` becomes the least recently used tenant.
    tokio::time::sleep(Duration::from_millis(10)).await;
    let _lease_b = scheduler.acquire("llm:b", 400, None).await.unwrap();
    assert_eq!(scheduler.snapshot().used_bytes, 900);

    // The third tenant does not fit, so the scheduler evicts the least
    // recently used tenant.
    scheduler.register_tenant("llm:c", TenantKind::Llm, 400, None, None);
    let _lease_c = scheduler.acquire("llm:c", 400, None).await.unwrap();
    assert_eq!(evicted.load(Ordering::SeqCst), 1);
    assert!(scheduler.tenant("llm:a").is_none());
    assert!(scheduler.tenant("llm:b").is_some());
}

#[tokio::test]
async fn pinned_tenants_are_never_evicted() {
    let scheduler = scheduler(1000);
    let (evicted, evictor) = eviction_counter();
    scheduler.register_tenant("llm:a", TenantKind::Llm, 500, None, Some(evictor));
    scheduler.set_pinned("llm:a", true).unwrap();
    let _lease = scheduler.acquire("llm:a", 500, None).await.unwrap();

    scheduler.register_tenant("llm:b", TenantKind::Llm, 600, None, None);
    let error = scheduler.acquire("llm:b", 600, None).await.unwrap_err();
    assert!(matches!(
        error,
        ai_scheduler::SchedulerError::InsufficientMemory { .. }
    ));
    assert_eq!(evicted.load(Ordering::SeqCst), 0);
    assert!(scheduler.tenant("llm:a").is_some());
}

#[tokio::test]
async fn busy_tenants_are_never_evicted() {
    let scheduler = scheduler(1000);
    let (evicted, evictor) = eviction_counter();
    scheduler.register_tenant("llm:a", TenantKind::Llm, 500, None, Some(evictor));
    let _lease = scheduler.acquire("llm:a", 500, None).await.unwrap();
    scheduler.begin_request("llm:a").unwrap();

    scheduler.register_tenant("llm:b", TenantKind::Llm, 600, None, None);
    // The busy tenant holds enough memory for the request, so the request
    // waits for it instead of failing at once. It times out: the tenant
    // stays busy.
    let error = scheduler
        .acquire("llm:b", 600, Some(Duration::from_millis(300)))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ai_scheduler::SchedulerError::Timeout(_)
    ));
    assert_eq!(evicted.load(Ordering::SeqCst), 0);
    assert!(scheduler.tenant("llm:a").is_some());
}

#[tokio::test]
async fn a_request_waits_for_a_busy_tenant_that_goes_idle() {
    let scheduler = scheduler(1000);
    let (evicted, evictor) = eviction_counter();
    scheduler.register_tenant("llm:a", TenantKind::Llm, 500, None, Some(evictor));
    let _lease = scheduler.acquire("llm:a", 500, None).await.unwrap();
    scheduler.begin_request("llm:a").unwrap();

    scheduler.register_tenant("llm:b", TenantKind::Llm, 600, None, None);
    // The request of the second tenant waits for the first one to be done.
    let waiter = {
        let scheduler = scheduler.clone();
        tokio::spawn(async move { scheduler.acquire("llm:b", 600, None).await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(evicted.load(Ordering::SeqCst), 0, "the busy tenant stays");

    // The request of the first tenant is over: its memory may be taken.
    scheduler.end_request("llm:a").unwrap();
    let lease = waiter
        .await
        .expect("the task did not panic")
        .expect("the request is answered");
    assert_eq!(evicted.load(Ordering::SeqCst), 1);
    assert_eq!(lease.bytes(), 600);
}

#[tokio::test]
async fn a_pinned_tenant_is_not_waited_for() {
    let scheduler = scheduler(1000);
    let (evicted, evictor) = eviction_counter();
    scheduler.register_tenant("llm:a", TenantKind::Llm, 500, None, Some(evictor));
    let _lease = scheduler.acquire("llm:a", 500, None).await.unwrap();
    scheduler.begin_request("llm:a").unwrap();
    scheduler.set_pinned("llm:a", true).unwrap();

    scheduler.register_tenant("llm:b", TenantKind::Llm, 600, None, None);
    // A pinned tenant never gives its memory back: the request fails at
    // once instead of waiting for a tenant that stays.
    let error = scheduler
        .acquire("llm:b", 600, Some(Duration::from_millis(300)))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ai_scheduler::SchedulerError::InsufficientMemory { .. }
    ));
    assert_eq!(evicted.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn expired_tenants_are_swept() {
    let scheduler = scheduler(1000);
    let (evicted, evictor) = eviction_counter();
    scheduler.register_tenant(
        "tts:a",
        TenantKind::Tts,
        100,
        Some(Duration::from_millis(10)),
        Some(evictor),
    );
    let _lease = scheduler.acquire("tts:a", 100, None).await.unwrap();
    scheduler.set_state("tts:a", TenantState::Running).unwrap();

    tokio::time::sleep(Duration::from_millis(30)).await;
    let swept = scheduler.sweep_expired().await.unwrap();
    assert_eq!(swept, vec!["tts:a".to_string()]);
    assert_eq!(evicted.load(Ordering::SeqCst), 1);
    assert!(scheduler.tenant("tts:a").is_none());
}

#[tokio::test]
async fn request_counters_are_tracked() {
    let scheduler = scheduler(1000);
    scheduler.register_tenant("tts:a", TenantKind::Tts, 100, None, None);
    let _lease = scheduler.acquire("tts:a", 100, None).await.unwrap();
    assert_eq!(scheduler.begin_request("tts:a").unwrap(), 1);
    assert_eq!(scheduler.begin_request("tts:a").unwrap(), 2);
    assert_eq!(scheduler.end_request("tts:a").unwrap(), 1);
    assert_eq!(scheduler.end_request("tts:a").unwrap(), 0);
    assert_eq!(scheduler.tenant("tts:a").unwrap().active_requests, 0);
}
