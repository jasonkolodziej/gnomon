use gnomon_api::pb::scheduler_client::SchedulerClient;
pub async fn connect() -> anyhow::Result<SchedulerClient<tonic::transport::Channel>> {
    let addr = std::env::var("GNOMON_ADDR").unwrap_or_else(|_| "http://127.0.0.1:7631".into());
    Ok(SchedulerClient::connect(addr).await?)
}
