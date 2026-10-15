//! Tells the core when the network comes and goes (the NetworkMonitor
//! portal), so that push pauses while offline and reconnects at once when
//! the network is back.

use std::sync::Arc;

use ashpd::desktop::network_monitor::NetworkMonitor;
use cumulas_core::CoreService;
use futures::StreamExt;

pub async fn watch(service: Arc<CoreService>) {
    let follow = async {
        let monitor = NetworkMonitor::new().await?;
        let mut changes = monitor.receive_changed().await?;
        let mut last = None;
        loop {
            let available = monitor.is_available().await?;
            if last != Some(available) {
                tracing::debug!("network monitor: available {available}");
                service.network_changed(available);
                last = Some(available);
            }
            if changes.next().await.is_none() {
                return Ok::<_, ashpd::Error>(());
            }
        }
    };
    if let Err(e) = follow.await {
        // Without the portal, the core assumes it's online and finds out
        // the hard way, as before.
        tracing::info!("no network monitor: {e}");
    }
}
