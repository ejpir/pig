//! Bounded OS DNS service. Uses the device's resolver/VPN, not a hard-coded public DNS.
//! getaddrinfo cannot be cancelled, so never put it on a per-request Tokio blocking pool:
//! dropping that runtime would wait for DNS after the HTTP deadline/cancellation fired.
//! At most two process-lifetime lookup threads and two queued requests exist. Cancelled
//! queued work is skipped; a stuck OS resolver consumes capacity, never new threads.
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
    sync::{Arc, Mutex, OnceLock, mpsc},
    time::Duration,
};
use tokio::sync::oneshot;
type Lookup = Arc<dyn Fn(String) -> io::Result<Vec<SocketAddr>> + Send + Sync>;
type Query = (String, oneshot::Sender<io::Result<Vec<SocketAddr>>>);
struct Pool(mpsc::SyncSender<Query>);
impl Pool {
    fn new(lookup: Lookup) -> io::Result<Self> {
        let (tx, rx) = mpsc::sync_channel::<Query>(2);
        let rx = Arc::new(Mutex::new(rx));
        for _ in 0..2 {
            let rx = rx.clone();
            let lookup = lookup.clone();
            std::thread::Builder::new()
                .name("pi-os-dns".into())
                .spawn(move || {
                    loop {
                        let request = rx.lock().unwrap_or_else(|e| e.into_inner()).recv();
                        let Ok((name, reply)) = request else {
                            break;
                        };
                        if !reply.is_closed() {
                            let _ = reply.send(lookup(name));
                        }
                    }
                })?;
        }
        Ok(Self(tx))
    }
    fn request(&self, name: String) -> io::Result<oneshot::Receiver<io::Result<Vec<SocketAddr>>>> {
        let (tx, rx) = oneshot::channel();
        self.0
            .try_send((name, tx))
            .map_err(|_| io::Error::other("DNS capacity unavailable"))?;
        Ok(rx)
    }
}
pub(crate) struct OsResolver;
impl Resolve for OsResolver {
    fn resolve(&self, name: Name) -> Resolving {
        static POOL: OnceLock<io::Result<Pool>> = OnceLock::new();
        let pool = POOL.get_or_init(|| {
            Pool::new(Arc::new(|name| {
                (name.as_str(), 0)
                    .to_socket_addrs()
                    .map(|addresses| addresses.take(64).collect())
            }))
        });
        let request = pool
            .as_ref()
            .map_err(|_| io::Error::other("DNS unavailable"))
            .and_then(|pool| pool.request(name.as_str().to_owned()));
        Box::pin(async move {
            let response = request?;
            let result = tokio::time::timeout(Duration::from_secs(10), response)
                .await
                .map_err(|_| io::Error::other("DNS deadline"))?
                .map_err(|_| io::Error::other("DNS unavailable"))??;
            Ok(Box::new(result.into_iter()) as Addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    #[test]
    fn stalled_lookup_capacity_is_bounded_and_cancelled_queue_is_skipped() -> io::Result<()> {
        let active = Arc::new(AtomicUsize::new(0));
        let running = active.clone();
        let release = Arc::new(AtomicBool::new(false));
        let released = release.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let called = calls.clone();
        let pool = Pool::new(Arc::new(move |_| {
            called.fetch_add(1, Ordering::Relaxed);
            running.fetch_add(1, Ordering::Relaxed);
            while !released.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(1));
            }
            running.fetch_sub(1, Ordering::Relaxed);
            Ok(vec![])
        }))?;
        let first = pool.request("synthetic.invalid".into())?;
        let second = pool.request("synthetic.invalid".into())?;
        let until = std::time::Instant::now() + Duration::from_secs(2);
        while active.load(Ordering::Relaxed) != 2 {
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
        let queued1 = pool.request("synthetic.invalid".into())?;
        let queued2 = pool.request("synthetic.invalid".into())?;
        assert!(pool.request("synthetic.invalid".into()).is_err());
        drop((first, second, queued1, queued2)); // no join or wait for OS lookup
        release.store(true, Ordering::Relaxed);
        drop(pool);
        while active.load(Ordering::Relaxed) != 0 {
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        Ok(())
    }
}
