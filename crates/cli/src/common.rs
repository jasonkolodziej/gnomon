use gnomon_api::pb::scheduler_client::SchedulerClient;
/// Connects to `GNOMON_ADDR` (a comma-separated list tries each controller
/// of a replicated group in turn); connect and each request are bounded by
/// `GNOMON_TIMEOUT_SECS` (default 10).
pub async fn connect() -> anyhow::Result<SchedulerClient<tonic::transport::Channel>> {
    let addr = std::env::var("GNOMON_ADDR").unwrap_or_else(|_| "http://127.0.0.1:7631".into());
    let secs = match std::env::var("GNOMON_TIMEOUT_SECS") {
        Ok(v) => v
            .parse::<f64>()
            .ok()
            .filter(|s| s.is_finite() && *s > 0.0)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "GNOMON_TIMEOUT_SECS must be a positive number of seconds, got '{v}'"
                )
            })?,
        Err(_) => 10.0,
    };
    let timeout = std::time::Duration::from_secs_f64(secs);
    let mut errors = Vec::new();
    for a in addr.split(',').map(str::trim).filter(|a| !a.is_empty()) {
        let endpoint = tonic::transport::Endpoint::from_shared(a.to_string())?
            .connect_timeout(timeout)
            .timeout(timeout);
        match endpoint.connect().await {
            Ok(channel) => return Ok(SchedulerClient::new(channel)),
            Err(e) => errors.push(format!("{a}: {e}")),
        }
    }
    anyhow::bail!("no controller reachable ({})", errors.join("; "))
}
