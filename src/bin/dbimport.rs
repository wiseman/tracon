use adsbx_json::v2::{
    AgedPosition, Aircraft, AltitudeOrGround, DatabaseFlags, Emergency, MessageType, NavMode,
    SilType,
};
use anyhow::Result;
use chrono::{TimeZone, Utc};
use dump::{db::adsbx::insert_aircraft, for_each_adsbx_json};
use tokio_postgres::NoTls;
use futures::stream::{FuturesUnordered, StreamExt};
use tokio_stream::wrappers::Iter;

#[derive(StructOpt, Debug)]
struct CliArgs {
    #[structopt(help = "Input files")]
    pub paths: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = CliArgs::from_args();

    // Connect to the PostgreSQL database
    let (client, connection) = tokio_postgres::connect(
        "host=localhost user=adsbx password=adsbx dbname=adsbx",
        NoTls,
    )
    .await?;

    // Spawn a task to manage the connection
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("Connection error: {}", e);
        }
    });

    for_each_adsbx_json(&args.paths, |adsbx_data| {
        let now = adsbx_data.now;
        let client = &client;

        let insert_futures: FuturesUnordered<_> = adsbx_data
            .aircraft
            .iter()
            .map(|ac| async move { insert_aircraft(client, &now, ac).await })
            .collect();

        let results = Iter::new(insert_futures.into_iter()).collect::<Vec<_>>().await;

        for result in results {
            if let Err(e) = result {
                eprintln!("Insert error: {}", e);
            }
        }

        None
    });

    Ok(())
}
